use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_edge");

static DIR_SEQ: AtomicU32 = AtomicU32::new(0);

fn scratch(name: &str) -> PathBuf {
    let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("edge-native-test-{}-{name}-{seq}", std::process::id()));
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

#[test]
fn runs_a_script_and_streams_stdout() {
    let dir = scratch("run");
    std::fs::write(dir.join("main.py"), "print(\"native ok\")\n").unwrap();
    let (out, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "native ok\n");
    assert_eq!(code, 0);
}

#[test]
fn resolves_quoted_imports_from_the_script_dir() {
    let dir = scratch("imports");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/helper.py"), "def double(n):\n  return n * 2\n").unwrap();
    std::fs::write(dir.join("main.py"), "from \"./lib/helper.py\" import double\nprint(double(21))\n").unwrap();
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
    std::fs::write(dir.join("green_test.py"), "raise SystemExit(0)\n").unwrap();
    let (out, _, code) = run_in(&dir, &["test"], None);
    assert!(out.contains("green_test.py"), "stdout was: {out}");
    assert_eq!(code, 0);
    std::fs::write(dir.join("red_test.py"), "raise SystemExit(1)\n").unwrap();
    let (_, _, code) = run_in(&dir, &["test"], None);
    assert_eq!(code, 1);
}

#[test]
fn builtin_time_module_mirrors_the_web_api() {
    let dir = scratch("host");
    std::fs::write(dir.join("main.py"), concat!(
        "import time\n",
        "t = time.gmtime(0)\n",
        "print(t)\n",
        "print(time.strftime(\"%Y-%m-%d\", t), time.tzname())\n",
        "print(time.ctime(86400))\n",
    )).unwrap();
    let (out, _, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "[1970,1,1,0,0,0,3,1,-1]\n1970-01-01 UTC\nFri Jan  2 00:00:00 1970\n");
    assert_eq!(code, 0);
}

#[test]
fn web_only_imports_point_at_the_web_flag() {
    let dir = scratch("webhint");
    std::fs::write(dir.join("main.py"), "import dom\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("requires the web runtime"), "stderr was: {err}");
    assert_eq!(code, 1);
}

#[test]
fn native_only_flags_reject_web() {
    let dir = scratch("guard");
    std::fs::write(dir.join("e.txt"), "x\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "--web", "--events", "e.txt"], None);
    assert!(err.contains("native-only"), "stderr was: {err}");
    assert_eq!(code, 1);
}

/* A cached url loads with no pin in the spec, then stays pinned to those first bytes. */
#[test]
fn a_cached_module_is_pinned_to_its_first_bytes() {
    let dir = scratch("pin");
    // Seed cache and pin the way a download would. The url never resolves, so nothing can refetch.
    let url = "https://cdn.test/helper.py";
    let key = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(url.as_bytes()));
    let blob = dir.join("edge-native").join(format!("{key}.py"));
    let src = "def double(n):\n    return n * 2\n";
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::write(&blob, src).unwrap();
    let pin = compiler::util::sha256::hex_encode(&compiler::util::sha256::sha256(src.as_bytes()));
    std::fs::write(blob.with_extension("py.lock"), &pin).unwrap();
    std::fs::write(dir.join("packages.json"), format!("{{ \"imports\": {{ \"helper\": \"{url}\" }} }}\n")).unwrap();
    std::fs::write(dir.join("main.py"), "from helper import double\nprint(double(21))\n").unwrap();

    // The gate this replaced refused unpinned bytes outright, breaking every default std import.
    let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert_eq!(out, "42\n", "stderr was: {err}");
    assert_eq!(code, 0, "stderr was: {err}");

    std::fs::write(&blob, "def double(n):\n    return 0\n").unwrap();
    let (_, err, code) = run_in(&dir, &["run", "main.py"], None);
    assert!(err.contains("integrity drift"), "stderr was: {err}");
    assert_eq!(code, 1);
}

// Locates a staged or locally built plugin, a staged one missing is a wiring bug.
fn std_plugin(pkg: &str) -> Option<PathBuf> {
    match std::env::var("EDGE_STD_DIR") {
        // CI stages this run's artifacts here, a missing plugin is a wiring bug, not a skip.
        Ok(d) => {
            let staged = Path::new(&d).join("native").join(format!("{pkg}-{}.{}", std::env::consts::ARCH, std::env::consts::DLL_EXTENSION));
            assert!(staged.exists(), "EDGE_STD_DIR is set but {} is missing", staged.display());
            Some(staged)
        }
        Err(_) => {
            // `struct` is a Rust keyword, so its crate and its local artifact carry the edge prefix.
            let crate_name = if pkg == "struct" { "edge_struct" } else { pkg };
            let lib = format!("{}{crate_name}.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX.trim_start_matches('.'));
            let local = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../std/{pkg}/target/release")).join(lib);
            local.exists().then_some(local)
        }
    }
}

// Exercises dlopen plus the exported edge_* bridge end to end for every native std package.
#[test]
fn loads_std_plugins_from_disk() {
    // A package whose exports are gated out of the native build would resolve to zero bindings.
    let cases = [
        ("json", "import json\nprint(json.dumps({\"a\": [1, True, None]}))\n", "{\"a\":[1,true,null]}\n"),
        ("re", "import re\nprint(re.search(r'\\d+', 'abc123def'))\n", "123\n"),
        ("math", "import math\nprint(math.floor(2.7))\n", "2\n"),
        ("struct", "import struct\nprint(struct.calcsize('i'))\n", "4\n"),
    ];
    let mut ran = 0;
    for (pkg, src, want) in cases {
        let Some(so) = std_plugin(pkg) else { continue };
        let dir = scratch(pkg);
        std::fs::write(
            dir.join("packages.json"),
            format!("{{ \"imports\": {{ \"{pkg}\": \"{}\" }} }}\n", so.display()),
        ).unwrap();
        std::fs::write(dir.join("main.py"), src).unwrap();
        let (out, err, code) = run_in(&dir, &["run", "main.py"], None);
        assert_eq!(out, want, "{pkg} stdout, stderr was: {err}");
        assert_eq!(code, 0, "{pkg} exit code, stderr was: {err}");
        ran += 1;
    }
    if ran == 0 {
        eprintln!("skipping, build the std packages with cargo build --release first");
    }
}
