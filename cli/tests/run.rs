mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_edge");

static DIR_SEQ: AtomicU32 = AtomicU32::new(0);

fn scratch(name: &str) -> PathBuf {
    let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("edge-run-test-{}-{name}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_in(dir: &Path, args: &[&str], stdin: Option<&str>) -> (String, String, i32) {
    let mut cmd = Command::new(BIN);
    // Scratch-local module cache, so no case reads or writes the real one.
    cmd.current_dir(dir).args(args).env("XDG_CACHE_HOME", dir).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn().unwrap();
    if let Some(input) = stdin {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/* A manifest declaring official names the way `edge add` writes them. */
// 010100101010 THESE URLS STOP RESOLVING ONCE THE CDN DROPS THE STD, POINT THEM AT THE REGISTRY WHEN EDGE-PYTHON-STD PUBLISHES.
fn manifest(names: &[&str]) -> String {
    let imports: Vec<String> = names
        .iter()
        .map(|name| {
            let url = match *name {
                "json" | "re" | "math" | "struct" => format!("https://cdn.edgepython.com/std/{name}.wasm"),
                "test" => "https://cdn.edgepython.com/std/test.py".to_string(),
                "dom" => "https://cdn.edgepython.com/js/builtins/dom/entry.py".to_string(),
                _ => format!("https://cdn.edgepython.com/js/builtins/{name}/index.js"),
            };
            format!("\"{name}\": \"{url}\"")
        })
        .collect();
    format!("{{ \"imports\": {{ {} }} }}\n", imports.join(", "))
}

#[test]
fn runs_a_script_and_streams_stdout() {
    let dir = scratch("run");
    std::fs::write(dir.join("main.py"), "print(\"cli ok\")\n").unwrap();
    let (out, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "cli ok\n");
    assert_eq!(code, 0);
}

#[test]
fn resolves_relative_imports_from_the_script_dir() {
    let dir = scratch("imports");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/helper.py"), "def double(n):\n  return n * 2\n").unwrap();
    std::fs::write(dir.join("main.py"), "from .lib.helper import double\nprint(double(21))\n").unwrap();
    let (out, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn sleep_waits_on_the_wall_clock() {
    let dir = scratch("sleep");
    std::fs::write(dir.join("main.py"), "await sleep(0.3)\nprint(\"woke\")\n").unwrap();
    let started = std::time::Instant::now();
    let (out, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(started.elapsed().as_secs_f64() >= 0.3);
    assert_eq!(out, "woke\n");
    assert_eq!(code, 0);
}

#[test]
fn system_exit_code_propagates() {
    let dir = scratch("exit");
    std::fs::write(dir.join("main.py"), "raise SystemExit(7)\n").unwrap();
    let (_, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(code, 7);
}

#[test]
fn piped_stdin_is_the_script_when_no_path_is_given() {
    let dir = scratch("stdin");
    let (out, _, code) = run_in(&dir, &["run"], Some("print(1 + 1)\n"));
    assert_eq!(out, "2\n");
    assert_eq!(code, 0);
}

#[test]
fn events_file_feeds_receive() {
    let dir = scratch("events");
    std::fs::write(dir.join("main.py"), "msg = await receive()\nprint(\"got\", msg)\n").unwrap();
    std::fs::write(dir.join("events.txt"), "ping\n").unwrap();
    let (out, _, code) = run_in(&dir, &["run", "main.py", "--events", "events.txt"], None);
    assert_eq!(out, "got ping\n");
    assert_eq!(code, 0);
}

#[test]
fn snapshot_saves_and_restores_across_processes() {
    let dir = scratch("snapshot");
    std::fs::write(dir.join("main.py"), "print(\"phase1\")\nmsg = await receive()\nprint(\"resumed\", msg)\n").unwrap();
    let (out, err, code) = run_in(&dir, &["run", "main.py", "--save-state", "state.bin"], None);
    assert_eq!(out, "phase1\n");
    assert!(err.contains("state saved"), "stderr was: {err}");
    assert_eq!(code, 0);
    std::fs::write(dir.join("ev.txt"), "later\n").unwrap();
    let (out, _, code) = run_in(&dir, &["run", "--restore-state", "state.bin", "--events", "ev.txt"], None);
    assert_eq!(out, "resumed later\n");
    assert_eq!(code, 0);
}

#[test]
fn repl_keeps_state_between_lines() {
    let dir = scratch("repl");
    let (out, _, code) = run_in(&dir, &["repl"], Some("x = 40\nprint(x + 2)\n.exit\n"));
    assert!(out.contains("42"), "stdout was: {out}");
    assert_eq!(code, 0);
}

#[test]
fn test_runner_verdicts_come_from_system_exit() {
    let dir = scratch("testrun");
    // A local runner stands in for the published one, the verdict comes from the file either way.
    std::fs::write(dir.join("edge.json"), "{ \"imports\": { \"test\": \"./runner.py\" } }\n").unwrap();
    std::fs::write(dir.join("runner.py"), "_tests = []\n\ndef run():\n    raise SystemExit(0)\n").unwrap();
    std::fs::write(dir.join("green_test.py"), "raise SystemExit(0)\n").unwrap();
    let (out, _, code) = run_in(&dir, &["test"], None);
    assert!(out.contains("green_test.py"), "stdout was: {out}");
    assert_eq!(code, 0);
    std::fs::write(dir.join("red_test.py"), "raise SystemExit(1)\n").unwrap();
    let (_, _, code) = run_in(&dir, &["test"], None);
    assert_eq!(code, 1);
}

// A packed package imports its own files from inside itself, and its pin covers every one of them.
#[test]
fn a_packed_package_imports_from_inside_itself() {
    let lib = scratch("package");
    std::fs::create_dir_all(lib.join("src")).unwrap();
    std::fs::write(lib.join("edge.json"), "{ \"name\": \"greet\", \"version\": \"0.1.0\", \"imports\": { \"_words\": \"./src/words.py\" } }\n").unwrap();
    std::fs::write(lib.join("main.py"), "from .src.hello import hello\n").unwrap();
    std::fs::write(lib.join("src/hello.py"), "from _words import WORD\n\ndef hello(name):\n    return WORD + \" \" + name\n").unwrap();
    std::fs::write(lib.join("src/words.py"), "WORD = \"hello\"\n").unwrap();
    let (_, err, code) = run_in(&lib, &["build"], None);
    assert_eq!(code, 0, "build failed: {err}");

    let app = scratch("package-app");
    std::fs::copy(lib.join("app.edge"), app.join("greet.edge")).unwrap();
    let pin = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(&std::fs::read(app.join("greet.edge")).unwrap()));
    std::fs::write(app.join("edge.json"), format!("{{ \"imports\": {{ \"greet\": \"./greet.edge#sha256-{pin}\" }} }}\n")).unwrap();
    std::fs::write(app.join("main.py"), "from greet import hello\nprint(hello(\"edge\"))\n").unwrap();
    let (out, err, code) = run_in(&app, &["run", "main.py"], None);
    assert_eq!((out.as_str(), code), ("hello edge\n", 0), "stderr: {err}");
}

#[derive(serde::Deserialize)]
struct CorpusCase {
    src: String,
    #[serde(default)]
    output: Vec<String>,
    // An expected error substring, the case passes when the run fails carrying it.
    error: Option<String>,
}

/* The network fixture the corpus points at, canned http and the event streams of the shared corpus. */
fn spawn_fixture() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("tcp addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
    port
}

// Parts written one at a time, one splits a CRLF, as the browser suite's mock sends them.
const SSE_FIELDS: [&str; 6] = [
    "\u{FEFF}data: bom first\n\n",
    ": a comment\r",
    "\nevent: ping\r\ndata: skipped\r\n\r\n",
    "id: 7\rdata: first line\rdata: second line\r\r",
    "data:no space\nretry: 5000\n\n",
    "id\ndata: after empty id\n\n",
];

fn serve(mut stream: std::net::TcpStream) {
    use std::io::{BufRead, Read, Write};
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut last_event_id = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("last-event-id")
        {
            last_event_id = Some(value.trim().to_string());
        }
    }
    let http = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let events = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    // An event stream stays open until the client goes away.
    let mut hold = || while matches!(reader.read(&mut [0u8; 64]), Ok(n) if n > 0) {};
    match path.as_str() {
        "/text" => drop(stream.write_all(http("hello from mock").as_bytes())),
        "/json" => drop(stream.write_all(http("{\"ok\":true}").as_bytes())),
        "/sse" => {
            let _ = stream.write_all(events.as_bytes());
            for i in 1..=3 {
                let _ = stream.write_all(format!("id: {i}\ndata: event {i}\n\n").as_bytes());
            }
            hold();
        }
        "/sse-fields" => {
            let _ = stream.write_all(events.as_bytes());
            for part in SSE_FIELDS {
                let _ = stream.write_all(part.as_bytes());
                let _ = stream.flush();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            hold();
        }
        // The first connection ends after one event, the retry echoes the Last-Event-ID it carried.
        "/sse-reconnect" => match last_event_id {
            None => drop(stream.write_all(format!("{events}retry: 250\nid: 41\ndata: first\n\n").as_bytes())),
            Some(last) => {
                let _ = stream.write_all(format!("{events}data: last={last}\n\n").as_bytes());
                hold();
            }
        },
        _ => drop(stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")),
    }
}

// Runs every shared builtins corpus against the CLI, mirroring the JS host cases.
#[test]
#[ignore = "010100101010 THESE CORPORA TEST THE SYSTEM LIBRARIES, RESTORE ONCE EDGE-PYTHON-STD PUBLISHES THEM TO THE REGISTRY"]
fn builtin_corpora_mirror_the_web_api() {
    common::cdn_base().unwrap_or_else(|e| panic!("{e}"));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/cases/builtins");
    let port = spawn_fixture();
    let base = format!("http://127.0.0.1:{port}");
    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let cap = path.file_stem().unwrap().to_string_lossy().into_owned();
        let cases: Vec<CorpusCase> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for (i, case) in cases.iter().enumerate() {
            ran += 1;
            let scratch = scratch(&format!("{cap}-corpus"));
            let src = case.src.replace("{BASE}", &base);
            // The JS host harness prepends the same star import, bare names resolve to the module exports.
            std::fs::write(scratch.join("edge.json"), manifest(&[&cap])).unwrap();
            std::fs::write(scratch.join("main.py"), format!("from {cap} import *\n{src}\n")).unwrap();
            let (out, err, code) = run_in(&scratch, &["run", "main.py"], None);
            if let Some(want) = &case.error {
                if code == 0 || !err.contains(want) {
                    failures.push(format!("[{cap} #{i}] expected error {want:?}, got code {code} err {err:?}"));
                }
                continue;
            }
            let want = format!("{}\n", case.output.join("\n"));
            if code != 0 || out != want {
                failures.push(format!("[{cap} #{i}] {:?}\n  got  {:?} (code {code}, err {err})\n  want {:?}", case.src, out, want));
            }
        }
    }
    assert!(ran > 0, "no corpus cases ran, discovery is broken");
    assert!(failures.is_empty(), "{} corpus case(s) failed:\n{}", failures.len(), failures.join("\n"));
}

/* A cached url loads with no pin in the spec, then stays pinned to those first bytes. */
#[test]
fn a_cached_module_is_pinned_to_its_first_bytes() {
    let dir = scratch("pin");
    // Seed cache and pin the way a download would. The url never resolves, so nothing can refetch.
    let url = "https://cdn.test/helper.py";
    let key = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(url.as_bytes()));
    let blob = dir.join("edge").join("modules").join(format!("{key}.py"));
    let src = "def double(n):\n    return n * 2\n";
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::write(&blob, src).unwrap();
    let pin = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(src.as_bytes()));
    std::fs::write(blob.with_extension("py.lock"), &pin).unwrap();
    std::fs::write(dir.join("edge.json"), format!("{{ \"imports\": {{ \"helper\": \"{url}\" }} }}\n")).unwrap();
    std::fs::write(dir.join("main.py"), "from helper import double\nprint(double(21))\n").unwrap();

    let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "42\n", "stderr was: {err}");
    assert_eq!(code, 0, "stderr was: {err}");

    std::fs::write(&blob, "def double(n):\n    return 0\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("integrity drift"), "stderr was: {err}");
    assert_eq!(code, 1);
}

/* A #sha256- fragment on a manifest target verifies the fetched bytes on every run. */
#[test]
fn a_manifest_pin_verifies_the_target_bytes() {
    let dir = scratch("fragpin");
    let src = "def double(n):\n    return n * 2\n";
    std::fs::write(dir.join("helper.py"), src).unwrap();
    let pin = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(src.as_bytes()));
    std::fs::write(dir.join("edge.json"), format!("{{ \"imports\": {{ \"helper\": \"./helper.py#sha256-{pin}\" }} }}\n")).unwrap();
    std::fs::write(dir.join("main.py"), "from helper import double\nprint(double(21))\n").unwrap();

    let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "42\n", "stderr was: {err}");
    assert_eq!(code, 0, "stderr was: {err}");

    let bad = "0".repeat(64);
    std::fs::write(dir.join("edge.json"), format!("{{ \"imports\": {{ \"helper\": \"./helper.py#sha256-{bad}\" }} }}\n")).unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("integrity check failed"), "stderr was: {err}");
    assert_eq!(code, 1);
}

/* Dotted imports anchor at the nearest edge.json dir, not at the importing file. */
#[test]
fn dotted_imports_anchor_at_the_manifest_root() {
    let dir = scratch("rooted");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(dir.join("edge.json"), "{ \"imports\": {} }\n").unwrap();
    std::fs::write(dir.join("lib/util.py"), "def f():\n    return 'root-lib'\n").unwrap();
    std::fs::write(dir.join("web/main.py"), "from lib.util import f\nprint(f())\n").unwrap();

    let (out, err, code) = run_in(&dir, &["run", "web/main.py"], None);
    assert_eq!(out, "root-lib\n", "stderr was: {err}");
    assert_eq!(code, 0, "stderr was: {err}");
}

/* Quoted specs are not imports at all, they fail like any missing module. */
#[test]
fn quoted_imports_are_not_found() {
    let dir = scratch("quoted");
    std::fs::write(dir.join("helper.py"), "def double(n):\n    return n * 2\n").unwrap();
    std::fs::write(dir.join("main.py"), "from \"./helper.py\" import double\n").unwrap();

    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("module './helper.py' not found"), "stderr was: {err}");
    assert_eq!(code, 1);
}

/* edge build --app packs a project into a binary that runs on its own, imports and all. */
#[test]
fn standalone_binary_runs_the_packed_project() {
    let dir = scratch("standalone");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/util.py"), "def greet():\n    return \"packed\"\n").unwrap();
    std::fs::write(dir.join("edge.json"), "{ \"imports\": { \"util\": \"./lib/util.py\" } }\n").unwrap();
    std::fs::write(dir.join("main.py"), "import util\nprint(util.greet())\n").unwrap();

    let (_, err, code) = run_in(&dir, &["build", "--app", "--out", "app"], None);
    assert_eq!(code, 0, "build stderr was: {err}");

    let app = dir.join("app");
    let out = Command::new(&app).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "packed\n");
    assert_eq!(out.status.code().unwrap_or(-1), 0);
}

/* edge run accepts a packed .edge, the artifact edge build writes by default. */
#[test]
fn run_accepts_a_packed_edge() {
    let dir = scratch("run-edge");
    std::fs::write(dir.join("main.py"), "print(\"ran the bundle\")\n").unwrap();
    let (_, err, code) = run_in(&dir, &["build", "--out", "app.edge"], None);
    assert_eq!(code, 0, "build stderr was: {err}");
    let (out, err, code) = run_in(&dir, &["run", "app.edge"], None);
    assert_eq!(out, "ran the bundle\n", "stderr was: {err}");
    assert_eq!(code, 0);
}

/* edge build writes a portable .edge carrying the project tree. */
#[test]
fn build_writes_an_edge_bundle() {
    let dir = scratch("bundle");
    std::fs::write(dir.join("main.py"), "print(\"hi\")\n").unwrap();
    let (_, err, code) = run_in(&dir, &["build", "--out", "app.edge"], None);
    assert_eq!(code, 0, "build stderr was: {err}");
    let bytes = std::fs::read(dir.join("app.edge")).unwrap();
    assert!(bytes.starts_with(b"EDGEPKG\x01"), "missing bundle magic");
}

fn web_build(dir: &Path) -> (String, String, i32) {
    common::cdn_base().unwrap_or_else(|e| panic!("{e}"));
    run_in(dir, &["build", "--web"], None)
}

/* A second web build must not collect the previous dist/ into dist/dist/. */
#[test]
fn rebuilding_web_does_not_nest_dist() {
    let dir = scratch("rebuild");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("edge.json"), "{}\n").unwrap();
    let (_, err, code) = web_build(&dir);
    assert_eq!(code, 0, "first build stderr was: {err}");
    let (_, err, code) = web_build(&dir);
    assert_eq!(code, 0, "second build stderr was: {err}");
    assert!(!dir.join("dist/dist").exists(), "dist was re-ingested on rebuild");
    assert!(dir.join("dist/main.py").exists());
}

/* A stale dist/ present before the first build is excluded from collection. */
#[test]
fn a_stale_dist_is_not_collected() {
    let dir = scratch("stale");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("edge.json"), "{}\n").unwrap();
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("dist/stale.py"), "print(\"stale\")\n").unwrap();
    let (_, err, code) = web_build(&dir);
    assert_eq!(code, 0, "build stderr was: {err}");
    assert!(!dir.join("dist/dist").exists(), "stale dist was re-ingested");
}

/* Only the output dir itself is skipped, a deeper dir named dist is still packed. */
#[test]
fn a_deeper_dir_named_dist_is_packed() {
    let dir = scratch("deepdist");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("edge.json"), "{}\n").unwrap();
    std::fs::create_dir_all(dir.join("sub/dist")).unwrap();
    std::fs::write(dir.join("sub/dist/keep.py"), "print(\"keep\")\n").unwrap();
    let (_, err, code) = web_build(&dir);
    assert_eq!(code, 0, "build stderr was: {err}");
    assert!(dir.join("dist/sub/dist/keep.py").exists());
}

/* The cases of cli/tests/engine.json, each one tempdir whose steps run in order. */
mod engine_corpus {
    use super::{run_in, scratch};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    // A text piece, or a text repeated so large inputs stay out of the JSON.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Part {
        Text(String),
        Repeat(String, usize),
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        name: String,
        // 010100101010 A CASE THAT WAITS ON EDGE-PYTHON-STD PUBLISHING ITS PACKAGES, SKIPPED UNTIL THEN.
        #[serde(default)]
        pending: Option<String>,
        #[serde(default)]
        given: BTreeMap<String, String>,
        #[serde(default)]
        generate: BTreeMap<String, Vec<Part>>,
        steps: Vec<Step>,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Step {
        run: Vec<String>,
        #[serde(default)]
        stdin: Option<Vec<Part>>,
        #[serde(default)]
        stdout: Vec<String>,
        #[serde(default)]
        stderr: Vec<String>,
        #[serde(default)]
        exit: i32,
        // A lower bound on stderr's size, the whole text must survive.
        #[serde(default)]
        stderr_bytes: usize,
        // Stdout lines starting with each prefix, counted exactly.
        #[serde(default)]
        lines: BTreeMap<String, usize>,
        // Requests the fixture saw for each path since the case began.
        #[serde(default)]
        requests: BTreeMap<String, usize>,
        // A lower bound on the size of each file the step leaves behind.
        #[serde(default)]
        file_bytes: BTreeMap<String, u64>,
    }

    fn join(parts: &[Part]) -> String {
        parts.iter().map(|p| match p {
            Part::Text(t) => t.clone(),
            Part::Repeat(t, n) => t.repeat(*n),
        }).collect()
    }

    /* Serves `/lib/mod.py`, answers 404 to the rest and logs every path it was asked for. */
    fn fixture() -> (String, Arc<Mutex<Vec<String>>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind loopback");
        let port = server.server_addr().to_ip().expect("tcp addr").port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let path = req.url().to_string();
                seen.lock().unwrap().push(path.clone());
                let resp = match path.as_str() {
                    "/lib/mod.py" => tiny_http::Response::from_string("def f():\n    return 7\n"),
                    _ => tiny_http::Response::from_string("").with_status_code(404),
                };
                let _ = req.respond(resp);
            }
        });
        (format!("http://127.0.0.1:{port}"), log)
    }

    #[test]
    fn engine_corpus() {
        crate::common::cdn_base().unwrap_or_else(|e| panic!("{e}"));
        let cases: Vec<Case> = serde_json::from_str(include_str!("engine.json")).expect("engine.json parse");
        let (base, log) = fixture();
        let mut failures = Vec::new();
        for case in cases.iter().filter(|c| c.pending.is_none()) {
            log.lock().unwrap().clear();
            let dir = scratch("engine");
            for (path, text) in &case.given {
                let file = dir.join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, text.replace("{BASE}", &base)).unwrap();
            }
            for (path, parts) in &case.generate {
                std::fs::write(dir.join(path), join(parts)).unwrap();
            }
            for (i, step) in case.steps.iter().enumerate() {
                let args: Vec<&str> = step.run.iter().map(String::as_str).collect();
                let stdin = step.stdin.as_deref().map(join);
                let (out, err, code) = run_in(&dir, &args, stdin.as_deref());
                let mut fail = |why: String| failures.push(format!("[{} #{i}] {why}", case.name));
                if code != step.exit {
                    fail(format!("exit {code}, want {}; stderr {:?}", step.exit, &err[..err.len().min(2000)]));
                }
                for want in &step.stdout {
                    if !out.contains(want.as_str()) {
                        fail(format!("stdout missing {want:?}, got {:?}", &out[..out.len().min(2000)]));
                    }
                }
                for want in &step.stderr {
                    if !err.contains(want.as_str()) {
                        fail(format!("stderr missing {want:?}, got {:?}", &err[..err.len().min(2000)]));
                    }
                }
                if err.len() < step.stderr_bytes {
                    fail(format!("stderr is {} bytes, want at least {}", err.len(), step.stderr_bytes));
                }
                for (prefix, want) in &step.lines {
                    let got = out.lines().filter(|l| l.starts_with(prefix.as_str())).count();
                    if got != *want {
                        fail(format!("{got} stdout lines start with {prefix:?}, want {want}; stderr {:?}", &err[..err.len().min(2000)]));
                    }
                }
                for (path, want) in &step.file_bytes {
                    let got = std::fs::metadata(dir.join(path)).map(|m| m.len()).unwrap_or(0);
                    if got < *want {
                        fail(format!("{path} is {got} bytes, want at least {want}"));
                    }
                }
                for (path, want) in &step.requests {
                    let got = log.lock().unwrap().iter().filter(|p| *p == path).count();
                    if got != *want {
                        fail(format!("{path} was requested {got} times, want {want}"));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} engine case(s) failed:\n{}", failures.len(), failures.join("\n"));
    }
}
