use std::io::Write;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_edge");

/* The edge binary with a private cache, JavaScript modules resolve through the local CDN. */
fn edge() -> Command {
    let base = std::env::var("EDGE_CDN_BASE").unwrap_or_else(|_| panic!("set EDGE_CDN_BASE (npm run cdn:local in infra)"));
    let cache = std::env::temp_dir().join(format!("edge-actor-cache-{}", std::process::id()));
    let mut cmd = Command::new(BIN);
    cmd.env("EDGE_CDN_BASE", base).env("XDG_CACHE_HOME", cache);
    cmd
}

// A case file, expect is the assertion, publish and listen drive the live-server cases.
#[derive(serde::Deserialize)]
struct Case {
    #[serde(default)]
    expect: Vec<String>,
    // Lines fed to the ingress after boot, only for cases that listen.
    #[serde(default)]
    publish: Vec<String>,
    // Substring the /stats body must contain, only for cases with a control port.
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

// Runs every cli/tests/actor/*.yml case, asserting the pool's stdout matches its expect block.
#[test]
fn actor_cases_match_their_expected_output() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/actor");
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
            Some(addr) => run_server(&path, addr, &case),
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
            && !status.contains(want.as_str())
        {
            failures.push(format!("[{name}] status missing {want:?}, got {status:?}"));
        }
        for (i, (p, reply)) in case.post.iter().zip(&replies).enumerate() {
            if !reply.contains(p.expect.as_str()) {
                failures.push(format!("[{name}] post #{i} reply missing {:?}, got {reply:?}", p.expect));
            }
        }
    }
    assert!(ran > 0, "no actor cases found");
    assert!(failures.is_empty(), "{} actor case(s) failed:\n{}", failures.len(), failures.join("\n"));
}

// A batch pool runs to completion, its stdout lines are the result.
fn run_batch(path: &std::path::Path) -> Vec<String> {
    let out = edge().args(["actor", path.to_str().unwrap()]).stdin(Stdio::null()).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

// A server pool stays alive, publish feeds the ingress, then stdout, /stats and posts are read.
fn run_server(path: &std::path::Path, listen: &str, case: &Case) -> (Vec<String>, String, Vec<String>) {
    let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
    let scratch = std::env::temp_dir().join(format!("edge-actor-{}-{}", std::process::id(), path.file_stem().unwrap().to_string_lossy()));
    let _ = std::fs::create_dir_all(&scratch);
    let manifest = scratch.join("actor.yml");
    std::fs::copy(path, &manifest).unwrap();
    // The groups resolve actor through the manifest beside the yml, so it travels along.
    std::fs::copy(path.with_file_name("edge.json"), scratch.join("edge.json")).unwrap();

    let mut child = edge().args(["actor", manifest.to_str().unwrap()]).stdin(Stdio::null()).stdout(Stdio::piped()).spawn().unwrap();
    if let Some(mut sock) = connect(addr) {
        for line in &case.publish {
            let _ = writeln!(sock, "{line}");
        }
        let _ = sock.flush();
    }
    std::thread::sleep(Duration::from_millis(400));
    let control_addr = case.runtime.control.as_deref().map(|c| c.strip_prefix("tcp://").unwrap_or(c));
    let status = match (control_addr, &case.expect_status) {
        (Some(c), Some(want)) => status_until(c, want),
        (Some(c), None) => get_status(c),
        (None, _) => String::new(),
    };
    let replies = match control_addr {
        Some(c) => case.post.iter().map(|p| post_eval(c, &p.path, &p.body)).collect(),
        None => Vec::new(),
    };
    // A /send is accepted once queued, give the actors a beat to print before the kill.
    std::thread::sleep(Duration::from_millis(400));
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    (String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect(), status, replies)
}

// Connects once the port binds, a debug pool can take seconds to boot.
fn connect(addr: &str) -> Option<TcpStream> {
    for _ in 0..200 {
        if let Ok(sock) = TcpStream::connect(addr) {
            return Some(sock);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

// POSTs the body to a control path over a bare HTTP request, returning just the reply body.
fn post_eval(addr: &str, path: &str, body: &str) -> String {
    use std::io::Read;
    let Some(mut sock) = connect(addr) else { return String::new() };
    let req = format!("POST {path} HTTP/1.0\r\nHost: {addr}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = sock.write_all(req.as_bytes());
    let mut resp = String::new();
    let _ = sock.read_to_string(&mut resp);
    resp.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default()
}

/* The bundle wire format `edge build --bundle` writes, magic, entry, then length-prefixed files. */
fn bundle(entry: &str, files: &[(&str, &str)]) -> Vec<u8> {
    fn put(b: &mut Vec<u8>, bytes: &[u8]) {
        b.extend_from_slice(bytes.len().to_string().as_bytes());
        b.push(b'\n');
        b.extend_from_slice(bytes);
    }
    let mut b = b"EDGEPKG\x01".to_vec();
    put(&mut b, entry.as_bytes());
    b.extend_from_slice(files.len().to_string().as_bytes());
    b.push(b'\n');
    for (path, content) in files {
        put(&mut b, path.as_bytes());
        put(&mut b, content.as_bytes());
    }
    b
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/* An untrusted client sends a whole project bundle to an eval group, which runs it in isolation. */
#[test]
fn eval_group_runs_a_bundled_project_over_the_wire() {
    let payload = bundle("main.py", &[
        ("main.py", "import util\nprint(util.hi())\n"),
        ("util.py", "def hi():\n    return \"bundled and run\"\n"),
        ("edge.json", "{ \"imports\": { \"util\": \"./util.py\" } }\n"),
    ]);
    let line = format!("runners EDGEPKG:{}", base64_encode(&payload));

    let scratch = std::env::temp_dir().join(format!("edge-actor-bundle-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let manifest = scratch.join("actor.yml");
    std::fs::write(&manifest, "runtime:\n  listen: tcp://127.0.0.1:7811\ngroups:\n  runners:\n    eval: true\n").unwrap();

    let mut child = edge().args(["actor", manifest.to_str().unwrap()]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut sock = connect("127.0.0.1:7811").expect("ingress never came up");
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

// Polls /stats until it carries `want`, messages settle after the publish returns.
fn status_until(addr: &str, want: &str) -> String {
    let mut body = String::new();
    for _ in 0..50 {
        body = get_status(addr);
        if body.contains(want) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    body
}

// Fetches /stats over a bare HTTP GET, returning just the response body.
fn get_status(addr: &str) -> String {
    use std::io::Read;
    let Some(mut sock) = connect(addr) else { return String::new() };
    let _ = write!(sock, "GET /stats HTTP/1.0\r\nHost: {addr}\r\n\r\n");
    let mut resp = String::new();
    let _ = sock.read_to_string(&mut resp);
    resp.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default()
}
