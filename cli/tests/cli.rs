mod common;

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    #[serde(default)] given: BTreeMap<String, String>,
    // Binary fixtures, each destination copied from a path relative to the repository root.
    #[serde(default)] copy: BTreeMap<String, String>,
    #[serde(default)] env: BTreeMap<String, String>,
    run: Vec<String>,
    #[serde(default)] stdin: String,
    #[serde(default)] stdout: Vec<String>,
    #[serde(default)] stderr: Vec<String>,
    #[serde(default)] fails: Option<Vec<String>>,
    #[serde(default)] creates: Vec<String>,
    #[serde(default)] contains: BTreeMap<String, String>,
}

/// JSON-driven CLI suite, each case in `cli.json` is one tempdir plus one spawn of the `edge` binary.
#[test]
fn cli_suite() {
    suite(include_str!("cli.json"));
}

/// Third party wasm plugins built from `pdk/example`, run the same way a project declares them.
#[test]
fn plugin_suite() {
    suite(include_str!("plugins.json"));
}

fn suite(json: &str) {
    let cases: Vec<Case> = serde_json::from_str(json).expect("case file parse");
    let bin = env!("CARGO_BIN_EXE_edge");
    let mut failed = vec![];
    for c in &cases {
        if let Err(e) = check(bin, c) {
            failed.push(format!("[edge {}] {e}", c.run.join(" ")));
        }
    }
    assert!(failed.is_empty(), "\n{}", failed.join("\n"));
}

fn check(bin: &str, c: &Case) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (p, v) in &c.given {
        let path = dir.path().join(p);
        if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
        std::fs::write(path, v).map_err(|e| e.to_string())?;
    }
    for (dest, src) in &c.copy {
        let from = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(src);
        let path = dir.path().join(dest);
        if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
        std::fs::copy(&from, path).map_err(|e| format!("fixture {}: {e}, build it first", from.display()))?;
    }
    let mut child = Command::new(bin).args(&c.run).current_dir(&dir).envs(common::local_tree().iter().cloned()).envs(&c.env)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| e.to_string())?;
    if !c.stdin.is_empty() {
        child.stdin.as_mut().unwrap().write_all(c.stdin.as_bytes()).map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take()); // close stdin so the process sees EOF
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let so = String::from_utf8_lossy(&out.stdout);
    let se = String::from_utf8_lossy(&out.stderr);
    let exit = out.status.code().unwrap_or(-1);
    let want_fail = c.fails.is_some();
    if (exit != 0) != want_fail {
        return Err(format!("exit {exit}; want {}; stderr: {se}", if want_fail { "non-zero" } else { "0" }));
    }
    for n in c.stderr.iter().chain(c.fails.iter().flatten()) {
        if !se.contains(n.as_str()) { return Err(format!("stderr missing {n:?}; got: {se}")); }
    }
    for n in &c.stdout { if !so.contains(n) { return Err(format!("stdout missing {n:?}; got: {so}")); } }
    for f in &c.creates { if !dir.path().join(f).exists() { return Err(format!("file missing: {f}")); } }
    for (f, n) in &c.contains {
        let t = std::fs::read_to_string(dir.path().join(f)).map_err(|e| e.to_string())?;
        if !t.contains(n) { return Err(format!("{f} missing {n:?}; got: {t}")); }
    }
    Ok(())
}
