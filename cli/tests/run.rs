mod common;
// The CLI's RFC 6455 codec, the websocket echo fixture frames with it.
#[path = "../src/ws.rs"]
mod ws;

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
    std::fs::write(dir.join("edge.json"), manifest(&["test"])).unwrap();
    std::fs::write(dir.join("green_test.py"), "raise SystemExit(0)\n").unwrap();
    let (out, _, code) = run_in(&dir, &["test"], None);
    assert!(out.contains("green_test.py"), "stdout was: {out}");
    assert_eq!(code, 0);
    std::fs::write(dir.join("red_test.py"), "raise SystemExit(1)\n").unwrap();
    let (_, _, code) = run_in(&dir, &["test"], None);
    assert_eq!(code, 1);
}

#[derive(serde::Deserialize)]
struct CorpusCase {
    src: String,
    #[serde(default)]
    output: Vec<String>,
    // An expected error substring, the case passes when the run fails carrying it.
    error: Option<String>,
}

/* The network fixture the corpus points at, canned http, three sse events and a ws echo. */
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

fn serve(mut stream: std::net::TcpStream) {
    use ws::{accept_key, encode_frame, parse_frame};
    use std::io::{BufRead, Read, Write};
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut key = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("sec-websocket-key")
        {
            key = Some(value.trim().to_string());
        }
    }
    let http = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    match (path.as_str(), key) {
        ("/text", _) => drop(stream.write_all(http("hello from mock").as_bytes())),
        ("/json", _) => drop(stream.write_all(http("{\"ok\":true}").as_bytes())),
        ("/sse", _) => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n");
            for i in 1..=3 {
                let _ = stream.write_all(format!("id: {i}\ndata: event {i}\n\n").as_bytes());
            }
            // The stream stays open until the client goes away.
            while matches!(reader.read(&mut [0u8; 64]), Ok(n) if n > 0) {}
        }
        ("/ws", Some(key)) => {
            let _ = write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key));
            let (mut buf, mut chunk) = (Vec::new(), [0u8; 4096]);
            while let Ok(n @ 1..) = reader.read(&mut chunk) {
                buf.extend_from_slice(&chunk[..n]);
                while let Some((opcode, payload, used)) = parse_frame(&buf) {
                    buf.drain(..used);
                    let reply = match opcode {
                        0x8 => return drop(stream.write_all(&encode_frame(0x8, &payload, None))),
                        0x9 => encode_frame(0xA, &payload, None),
                        _ => encode_frame(opcode, &payload, None),
                    };
                    let _ = stream.write_all(&reply);
                }
            }
        }
        _ => drop(stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")),
    }
}

// Runs every shared builtins corpus against the CLI, mirroring the JS host cases.
#[test]
fn builtin_corpora_mirror_the_web_api() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/cases/builtins");
    let port = spawn_fixture();
    let (base, ws_base) = (format!("http://127.0.0.1:{port}"), format!("ws://127.0.0.1:{port}"));
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
            let src = case.src.replace("{BASE}", &base).replace("{WS_BASE}", &ws_base);
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

/* edge build packs a project into a standalone .edge that runs on its own, imports and all. */
#[test]
fn standalone_edge_runs_the_packed_project() {
    let dir = scratch("standalone");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/util.py"), "def greet():\n    return \"packed\"\n").unwrap();
    std::fs::write(dir.join("edge.json"), "{ \"imports\": { \"util\": \"./lib/util.py\" } }\n").unwrap();
    std::fs::write(dir.join("main.py"), "import util\nprint(util.greet())\n").unwrap();

    let (_, err, code) = run_in(&dir, &["build", "--out", "app.edge"], None);
    assert_eq!(code, 0, "build stderr was: {err}");

    let app = dir.join("app.edge");
    let out = Command::new(&app).current_dir(&dir).stdin(Stdio::null()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "packed\n");
    assert_eq!(out.status.code().unwrap_or(-1), 0);
}

/* edge run accepts a packed .edge, matching a direct ./app.edge invocation. */
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

/* edge build --bundle writes a lightweight .package carrying the project tree. */
#[test]
fn bundle_writes_a_package_file() {
    let dir = scratch("bundle");
    std::fs::write(dir.join("main.py"), "print(\"hi\")\n").unwrap();
    let (_, err, code) = run_in(&dir, &["build", "--bundle", "--out", "app.package"], None);
    assert_eq!(code, 0, "build stderr was: {err}");
    let bytes = std::fs::read(dir.join("app.package")).unwrap();
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
        let cases: Vec<Case> = serde_json::from_str(include_str!("engine.json")).expect("engine.json parse");
        let (base, log) = fixture();
        let mut failures = Vec::new();
        for case in &cases {
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
