use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub ok: bool,
}

fn spawn(mut cmd: Command, stdin_data: &str, timeout: Duration) -> Result<Outcome, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_data.as_bytes());
    }
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    Ok(Outcome { stdout, stderr, ok: status.success() })
}

/* The manifest every cell runs under, the official names as `edge add` writes them. */
pub const MANIFEST: &str = r#"{
  "imports": {
    "json": "https://cdn.edgepython.com/std/json.wasm",
    "re": "https://cdn.edgepython.com/std/re.wasm",
    "math": "https://cdn.edgepython.com/std/math.wasm",
    "struct": "https://cdn.edgepython.com/std/struct.wasm",
    "test": "https://cdn.edgepython.com/std/test.py",
    "dom": "https://cdn.edgepython.com/js/builtins/dom/entry.py"
  },
  "system": {
    "storage": "https://cdn.edgepython.com/js/builtins/storage/index.js",
    "network": "https://cdn.edgepython.com/js/builtins/network/index.js",
    "time": "https://cdn.edgepython.com/js/builtins/time/index.js",
    "actor": "https://cdn.edgepython.com/js/builtins/actor/index.js"
  }
}
"#;

// A fresh scratch dir holding the manifest, one per cell so runs never share files.
fn scratch() -> Result<std::path::PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("skill-cell-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("tempdir failed: {e}"))?;
    std::fs::write(dir.join("packages.json"), MANIFEST).map_err(|e| format!("write packages.json failed: {e}"))?;
    Ok(dir)
}

pub fn run_script(edge: &str, src: &str, timeout: Duration) -> Result<Outcome, String> {
    let dir = scratch()?;
    let mut cmd = Command::new(edge);
    cmd.arg("run").arg("--packages").arg(dir.join("packages.json"));
    let outcome = spawn(cmd, src, timeout);
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}

pub fn run_actor(edge: &str, yml: &str, timeout: Duration) -> Result<Outcome, String> {
    let dir = scratch()?;
    let path = dir.join("actor.yml");
    std::fs::write(&path, yml).map_err(|e| format!("write actor.yml failed: {e}"))?;
    let mut cmd = Command::new(edge);
    cmd.arg("actor").arg(&path);
    let outcome = spawn(cmd, "", timeout);
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}

pub fn check(expect: &str, outcome: &Outcome, verdict_is_error: bool) -> Result<(), String> {
    if verdict_is_error {
        if outcome.ok {
            return Err("expected a failing run, exit code was 0".to_string());
        }
        if !outcome.stderr.contains(expect.trim()) {
            return Err(format!(
                "stderr mismatch\n  expected substring: {}\n  got: {}",
                expect.trim(),
                outcome.stderr.trim()
            ));
        }
        return Ok(());
    }
    if !outcome.ok {
        return Err(format!(
            "run failed\n  stderr: {}",
            outcome.stderr.trim()
        ));
    }
    if expect.trim_end() != outcome.stdout.trim_end() {
        return Err(format!(
            "stdout mismatch\n  expected: {}\n  got: {}",
            expect.trim_end(),
            outcome.stdout.trim_end()
        ));
    }
    Ok(())
}
