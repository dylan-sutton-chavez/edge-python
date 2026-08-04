/*
JSON-driven CLI suite. Each case in `cli.json` is one tempdir + one spawn of the `edge` binary.
Every case runs unconditionally so a real bug can't hide behind a tag.
*/

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    #[serde(default)] given: BTreeMap<String, String>,
    #[serde(default)] env: BTreeMap<String, String>,
    // "both" runs the case through native and web, "web" prepends --web, absent runs as written.
    #[serde(default)] engine: Option<String>,
    run: Vec<String>,
    #[serde(default)] stdin: String,
    #[serde(default)] stdout: Vec<String>,
    #[serde(default)] stderr: Vec<String>,
    #[serde(default)] fails: Option<Vec<String>>,
    #[serde(default)] creates: Vec<String>,
    #[serde(default)] contains: BTreeMap<String, String>,
}

#[test]
fn cli_suite() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("cli.json")).expect("cli.json parse");
    let bin = env!("CARGO_BIN_EXE_edge");
    let mut failed = vec![];
    for c in &cases {
        // Shared-semantics cases run through both engines, a divergence is a bug in one of them.
        let passes: &[bool] = match c.engine.as_deref() {
            None => &[false],
            Some("web") => &[true],
            Some("both") => &[false, true],
            Some(other) => panic!("cli.json: unknown engine {other:?}"),
        };
        for &web in passes {
            if let Err(e) = check(bin, c, web) {
                failed.push(format!("[edge {}{}] {e}", if web { "--web " } else { "" }, c.run.join(" ")));
            }
        }
    }
    assert!(failed.is_empty(), "\n{}", failed.join("\n"));
}

fn check(bin: &str, c: &Case, web: bool) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (p, v) in &c.given {
        let path = dir.path().join(p);
        if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
        std::fs::write(path, v).map_err(|e| e.to_string())?;
    }
    let mut cmd = Command::new(bin);
    if web { cmd.arg("--web"); }
    let mut child = cmd.args(&c.run).current_dir(&dir).envs(&c.env)
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
