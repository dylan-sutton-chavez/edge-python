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
    let mut imports = Vec::new();
    let mut system = Vec::new();
    for name in names {
        match *name {
            "json" | "re" | "math" | "struct" => imports.push(format!("\"{name}\": \"https://cdn.edgepython.com/std/{name}.wasm\"")),
            "test" => imports.push("\"test\": \"https://cdn.edgepython.com/std/test.py\"".to_string()),
            "dom" => imports.push("\"dom\": \"https://cdn.edgepython.com/js/builtins/dom/entry.py\"".to_string()),
            _ => system.push(format!("\"{name}\": \"https://cdn.edgepython.com/js/builtins/{name}/index.js\"")),
        }
    }
    format!("{{ \"imports\": {{ {} }}, \"system\": {{ {} }} }}\n", imports.join(", "), system.join(", "))
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
    std::fs::write(dir.join("packages.json"), manifest(&["test"])).unwrap();
    std::fs::write(dir.join("green_test.py"), "raise SystemExit(0)\n").unwrap();
    let (out, _, code) = run_in(&dir, &["test"], None);
    assert!(out.contains("green_test.py"), "stdout was: {out}");
    assert_eq!(code, 0);
    std::fs::write(dir.join("red_test.py"), "raise SystemExit(1)\n").unwrap();
    let (_, _, code) = run_in(&dir, &["test"], None);
    assert_eq!(code, 1);
}

/* The traceback names the entry script, the frame the compiler renders as `<input>`. */
#[test]
fn tracebacks_name_the_script() {
    let dir = scratch("traceback");
    std::fs::write(dir.join("broken.py"), "before = 1\nx = 1 / 0\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "broken.py"], None);
    assert!(err.contains("ZeroDivisionError"), "stderr was: {err}");
    assert!(err.contains("--> broken.py:2:1"), "stderr was: {err}");
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

/* The http fixture the network corpus points at, serves `/text` and `/json` on a free port. */
fn spawn_fixture() -> u16 {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind loopback");
    let port = server.server_addr().to_ip().expect("tcp addr").port();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let path = req.url().split('?').next().unwrap_or("").to_string();
            let (body, mime, status) = match path.as_str() {
                "/text" => ("hello from mock", "text/plain", 200),
                "/json" => ("{\"ok\":true}", "application/json", 200),
                _ => ("", "text/plain", 404),
            };
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], mime.as_bytes()).expect("static header");
            let _ = req.respond(tiny_http::Response::from_string(body).with_header(header).with_status_code(status));
        }
    });
    port
}

// Runs every shared builtins corpus against the CLI, mirroring the JS host cases.
#[test]
fn builtin_corpora_mirror_the_web_api() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/cases/builtins");
    let base = format!("http://127.0.0.1:{}", spawn_fixture());
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
            std::fs::write(scratch.join("packages.json"), manifest(&[&cap])).unwrap();
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

/* A declared name the CLI cannot serve names the host it needs, facade or system module. */
#[test]
fn browser_only_imports_say_so() {
    let dir = scratch("browserhint");
    std::fs::write(dir.join("packages.json"), manifest(&["dom", "storage"])).unwrap();
    for name in ["dom", "storage"] {
        std::fs::write(dir.join("main.py"), format!("import {name}\n")).unwrap();
        let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
        assert!(err.contains(&format!("module '{name}' requires a browser")), "stderr was: {err}");
        assert_eq!(code, 1);
    }
}

/* No name resolves on its own, an undeclared official package fails at compile time. */
#[test]
fn undeclared_names_fail_at_compile_time() {
    let dir = scratch("undeclared");
    std::fs::write(dir.join("main.py"), "import json\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("module 'json' is not provided by this host and no packages.json declares it"), "stderr was: {err}");
    assert_eq!(code, 1);
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
    std::fs::write(dir.join("packages.json"), format!("{{ \"imports\": {{ \"helper\": \"{url}\" }} }}\n")).unwrap();
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
    std::fs::write(dir.join("packages.json"), format!("{{ \"imports\": {{ \"helper\": \"./helper.py#sha256-{pin}\" }} }}\n")).unwrap();
    std::fs::write(dir.join("main.py"), "from helper import double\nprint(double(21))\n").unwrap();

    let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "42\n", "stderr was: {err}");
    assert_eq!(code, 0, "stderr was: {err}");

    let bad = "0".repeat(64);
    std::fs::write(dir.join("packages.json"), format!("{{ \"imports\": {{ \"helper\": \"./helper.py#sha256-{bad}\" }} }}\n")).unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("integrity check failed"), "stderr was: {err}");
    assert_eq!(code, 1);
}

/* Dotted imports anchor at the nearest packages.json dir, not at the importing file. */
#[test]
fn dotted_imports_anchor_at_the_manifest_root() {
    let dir = scratch("rooted");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(dir.join("packages.json"), "{ \"imports\": {} }\n").unwrap();
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

/* A third party .wasm needs the JS host, a native library has no host at all. */
#[test]
fn binary_module_specs_say_where_they_run() {
    let dir = scratch("binary");
    let cases = [
        ("https://example.com/x.wasm", "module 'https://example.com/x.wasm' requires the JS host"),
        ("./vendor/x.so", "module './vendor/x.so' is not supported, ship a .wasm"),
        ("./vendor/x.dylib", "module './vendor/x.dylib' is not supported, ship a .wasm"),
    ];
    for (spec, want) in cases {
        std::fs::write(dir.join("packages.json"), format!("{{ \"imports\": {{ \"x\": \"{spec}\" }} }}\n")).unwrap();
        std::fs::write(dir.join("main.py"), "import x\n").unwrap();
        let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
        assert!(err.contains(want), "spec {spec}, stderr was: {err}");
        assert_eq!(code, 1);
    }
}

/* edge build packs a project into a standalone .edge that runs on its own, imports and all. */
#[test]
fn standalone_edge_runs_the_packed_project() {
    let dir = scratch("standalone");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/util.py"), "def greet():\n    return \"packed\"\n").unwrap();
    std::fs::write(dir.join("packages.json"), "{ \"imports\": { \"util\": \"./lib/util.py\" } }\n").unwrap();
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

// Website builds need the local JS host hooks, without them the fetch would hit the CDN.
fn web_build(dir: &Path) -> Option<(String, String, i32)> {
    if std::env::var_os("EDGE_JS_DIR").is_none() || std::env::var_os("EDGE_COMPILER_WASM").is_none() {
        eprintln!("skipping, set EDGE_JS_DIR and EDGE_COMPILER_WASM to cover website builds");
        return None;
    }
    Some(run_in(dir, &["build", "--web"], None))
}

/* A second web build must not collect the previous dist/ into dist/dist/. */
#[test]
fn rebuilding_web_does_not_nest_dist() {
    let dir = scratch("rebuild");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("packages.json"), "{}\n").unwrap();
    let Some((_, err, code)) = web_build(&dir) else { return };
    assert_eq!(code, 0, "first build stderr was: {err}");
    let (_, err, code) = web_build(&dir).unwrap();
    assert_eq!(code, 0, "second build stderr was: {err}");
    assert!(!dir.join("dist/dist").exists(), "dist was re-ingested on rebuild");
    assert!(dir.join("dist/main.py").exists());
}

/* A stale dist/ present before the first build is excluded from collection. */
#[test]
fn a_stale_dist_is_not_collected() {
    let dir = scratch("stale");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("packages.json"), "{}\n").unwrap();
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("dist/stale.py"), "print(\"stale\")\n").unwrap();
    let Some((_, err, code)) = web_build(&dir) else { return };
    assert_eq!(code, 0, "build stderr was: {err}");
    assert!(!dir.join("dist/dist").exists(), "stale dist was re-ingested");
}

/* Only the output dir itself is skipped, a deeper dir named dist is still packed. */
#[test]
fn a_deeper_dir_named_dist_is_packed() {
    let dir = scratch("deepdist");
    std::fs::write(dir.join("main.py"), "print(\"ok\")\n").unwrap();
    std::fs::write(dir.join("packages.json"), "{}\n").unwrap();
    std::fs::create_dir_all(dir.join("sub/dist")).unwrap();
    std::fs::write(dir.join("sub/dist/keep.py"), "print(\"keep\")\n").unwrap();
    let Some((_, err, code)) = web_build(&dir) else { return };
    assert_eq!(code, 0, "build stderr was: {err}");
    assert!(dir.join("dist/sub/dist/keep.py").exists());
}

/* The std packages ship inside the binary, their CDN specs resolve with no network. */
#[test]
fn std_packages_are_built_in() {
    let cases = [
        ("json", "import json\nprint(json.dumps({\"a\": [1, True, None]}))\n", "{\"a\":[1,true,null]}\n"),
        ("re", "import re\nprint(re.search(r'\\d+', 'abc123def'))\n", "123\n"),
        ("math", "import math\nprint(math.floor(2.7))\n", "2\n"),
        ("struct", "import struct\nprint(struct.calcsize('i'))\n", "4\n"),
        ("test", "from test import test\nprint(callable(test))\n", "True\n"),
    ];
    for (pkg, src, want) in cases {
        let dir = scratch(pkg);
        std::fs::write(dir.join("packages.json"), manifest(&[pkg])).unwrap();
        std::fs::write(dir.join("main.py"), src).unwrap();
        let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
        assert_eq!(out, want, "{pkg} stdout, stderr was: {err}");
        assert_eq!(code, 0, "{pkg} exit code, stderr was: {err}");
    }
}

/* The std spec `edge add` writes resolves to the built-in package, one manifest serves both hosts. */
#[test]
fn cdn_std_specs_map_to_the_built_in_package() {
    let dir = scratch("cdnstd");
    std::fs::write(dir.join("packages.json"), "{ \"imports\": { \"json\": \"https://cdn.edgepython.com/std/json.wasm\" } }\n").unwrap();
    std::fs::write(dir.join("main.py"), "import json\nprint(json.dumps([1]))\n").unwrap();
    let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "[1]\n", "stderr was: {err}");
    assert_eq!(code, 0);
}
