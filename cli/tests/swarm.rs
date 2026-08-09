use std::io::Write;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_edge");

// A case file, expect is the assertion, publish and listen drive the live-server cases.
#[derive(serde::Deserialize)]
struct Case {
    #[serde(default)]
    expect: Vec<String>,
    // Lines fed to the ingress after boot, only for cases that listen.
    #[serde(default)]
    publish: Vec<String>,
    // Substring the /status body must contain, only for cases with a control port.
    #[serde(default)]
    expect_status: Option<String>,
    // POSTs against the control port, each reply body must carry expect as a substring.
    #[serde(default)]
    post: Vec<Post>,
    #[serde(default)]
    runtime: Runtime,
}

// One POST to the control port, path and body go out, expect matches the reply.
#[derive(serde::Deserialize)]
struct Post {
    path: String,
    body: String,
    expect: String,
}

#[derive(serde::Deserialize, Default)]
struct Runtime {
    #[serde(default)]
    listen: Option<String>,
    #[serde(default)]
    control: Option<String>,
}

// Runs every cli/tests/swarm/*.yml case, asserting the swarm's stdout matches its expect block.
#[test]
fn swarm_cases_match_their_expected_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/swarm");
    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        ran += 1;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let case: Case = serde_yaml_ng::from_str(&text).unwrap();

        let (got, status, replies) = match &case.runtime.listen {
            Some(addr) => run_server(&path, addr, &case.publish, case.runtime.control.as_deref(), &case.post),
            None => (run_batch(&path), String::new(), Vec::new()),
        };
        // Order across groups is not fixed, compare as a sorted multiset of lines.
        let (mut got, mut expected) = (got, case.expect.clone());
        got.sort();
        expected.sort();
        if got != expected {
            failures.push(format!("[{name}] output mismatch\n  want {expected:?}\n  got  {got:?}"));
        }
        if let Some(want) = &case.expect_status
            && !status.contains(want.as_str()) {
            failures.push(format!("[{name}] status missing {want:?}, got {status:?}"));
        }
        for (i, (p, reply)) in case.post.iter().zip(&replies).enumerate() {
            if !reply.contains(p.expect.as_str()) {
                failures.push(format!("[{name}] post #{i} reply missing {:?}, got {reply:?}", p.expect));
            }
        }
    }
    assert!(ran > 0, "no swarm cases found");
    assert!(failures.is_empty(), "{} swarm case(s) failed:\n{}", failures.len(), failures.join("\n"));
}

// A batch swarm runs to completion, its stdout lines are the result.
fn run_batch(path: &std::path::Path) -> Vec<String> {
    let out = Command::new(BIN).args(["swarm", path.to_str().unwrap()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

// A server swarm stays alive, publish feeds its ingress, then output, /status and posts are read.
// The manifest is copied into a tempdir so its default wal lands there, never in the repo.
fn run_server(path: &std::path::Path, listen: &str, publish: &[String], control: Option<&str>, posts: &[Post]) -> (Vec<String>, String, Vec<String>) {
    let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
    let scratch = std::env::temp_dir().join(format!("edge-swarm-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let manifest = scratch.join("swarm.yml");
    std::fs::copy(path, &manifest).unwrap();

    let mut child = Command::new(BIN).args(["swarm", manifest.to_str().unwrap()]).stdout(Stdio::piped()).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(400));
    if let Ok(mut sock) = TcpStream::connect(addr) {
        for line in publish {
            let _ = writeln!(sock, "{line}");
        }
        let _ = sock.flush();
    }
    std::thread::sleep(Duration::from_millis(400));
    let control_addr = control.map(|c| c.strip_prefix("tcp://").unwrap_or(c));
    let status = control_addr.map(get_status).unwrap_or_default();
    let replies = match control_addr {
        Some(c) => posts.iter().map(|p| post_eval(c, &p.path, &p.body)).collect(),
        None => Vec::new(),
    };
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    (String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect(), status, replies)
}

// POSTs the body to a control path over a bare HTTP request, returning just the reply body.
fn post_eval(addr: &str, path: &str, body: &str) -> String {
    use std::io::Read;
    let Ok(mut sock) = TcpStream::connect(addr) else { return String::new() };
    let req = format!("POST {path} HTTP/1.0\r\nHost: {addr}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = sock.write_all(req.as_bytes());
    let mut resp = String::new();
    let _ = sock.read_to_string(&mut resp);
    resp.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default()
}

/* An untrusted client sends a whole project bundle to an eval group, which runs it in isolation. */
#[test]
fn eval_group_runs_a_bundled_project_over_the_wire() {
    use compiler::native::pack::{Bundle, Entry};
    let bundle = Bundle {
        entry: "main.py".to_string(),
        files: vec![
            Entry { path: "main.py".to_string(), bytes: b"import util\nprint(util.hi())\n".to_vec() },
            Entry { path: "util.py".to_string(), bytes: b"def hi():\n    return \"bundled and run\"\n".to_vec() },
            Entry { path: "packages.json".to_string(), bytes: b"{ \"imports\": { \"util\": \"./util.py\" } }\n".to_vec() },
        ],
    };
    let line = format!("runners EDGEPKG:{}", compiler::util::ws::base64_encode(&bundle.encode()));

    let scratch = std::env::temp_dir().join(format!("edge-swarm-bundle-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let manifest = scratch.join("swarm.yml");
    std::fs::write(&manifest, "runtime:\n  listen: tcp://127.0.0.1:7811\ngroups:\n  runners:\n    eval: true\n").unwrap();

    let mut child = Command::new(BIN).args(["swarm", manifest.to_str().unwrap()]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // Retry the connect until the ingress binds, the swarm boots slower under a loaded test run.
    let mut sock = None;
    for _ in 0..40 {
        if let Ok(s) = TcpStream::connect("127.0.0.1:7811") { sock = Some(s); break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut sock = sock.expect("ingress never came up");
    let _ = writeln!(sock, "{line}");
    let _ = sock.flush();
    std::thread::sleep(Duration::from_millis(800));
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    let err = String::from_utf8_lossy(&out.stderr);
    let got: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    assert_eq!(got, vec!["bundled and run"], "stdout was {got:?}, stderr {err:?}");
}

// Fetches /status over a bare HTTP GET, returning just the response body.
fn get_status(addr: &str) -> String {
    use std::io::Read;
    let Ok(mut sock) = TcpStream::connect(addr) else { return String::new() };
    let _ = write!(sock, "GET /status HTTP/1.0\r\nHost: {addr}\r\n\r\n");
    let mut resp = String::new();
    let _ = sock.read_to_string(&mut resp);
    resp.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default()
}
