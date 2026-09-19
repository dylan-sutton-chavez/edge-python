use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::host::driver::{base_dir, Session};
use crate::manifest::Manifest;
use crate::ui;

// Runs registered tests, exit 3 flags an empty file.
const TEST_DRIVER: &str = "import test\nif not test._tests:\n    raise SystemExit(3)\ntest.run()";

/// Discovers *_test.py files and drives each through one session, verdicts come only from SystemExit codes.
pub fn run(manifest_path: &Path, manifest: Option<&Path>, path: Option<&Path>) -> Result<()> {
    let target = path.unwrap_or(Path::new("."));
    let files = if target.is_file() { vec![target.to_path_buf()] } else { discover_tests(target) };
    if files.is_empty() {
        bail!("no *_test.py files found under {}", target.display());
    }
    // The driver imports test, so the manifest must declare it like any other name.
    if !Manifest::load(manifest_path)?.imports.contains_key("test") {
        bail!("declare test in edge.json (edge add test)");
    }

    let open = || Session::open(manifest);
    let mut session = open_or_die(&open);

    let started = std::time::Instant::now();
    let mut failed = 0usize;
    for (i, file) in files.iter().enumerate() {
        if i > 0 && session.reset().is_err() {
            drop(session);
            session = open_or_die(&open);
        }
        let result = std::fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))
            .and_then(|src| run_file(&mut session, &src, file));
        let (ok, reason) = match result {
            Ok(v) => v,
            // A wedged session poisons later files, reopen.
            Err(e) => {
                ui::error(&e);
                drop(session);
                session = open_or_die(&open);
                (false, Some("error"))
            }
        };
        let name = file.strip_prefix(".").unwrap_or(file);
        ui::test_verdict(ok, &name.display().to_string(), reason);
        if !ok {
            failed += 1;
        }
    }

    ui::test_summary(files.len() - failed, files.len(), started.elapsed().as_secs_f64());
    drop(session);
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/* Collects *_test.py under `dir` recursively, skipping dist and hidden entries, sorted. */
fn discover_tests(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name == "dist" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if name.ends_with("_test.py") {
            out.push(path);
        }
    }
}

/// Exit 2 keeps infra failures distinct from red tests.
fn open_or_die(open: &dyn Fn() -> Result<Session>) -> Session {
    match open() {
        Ok(s) => s,
        Err(e) => {
            ui::error(&e);
            std::process::exit(2);
        }
    }
}

/// Eval the file, then the driver when it didn't exit itself.
fn run_file(session: &mut Session, src: &str, file: &Path) -> Result<(bool, Option<&'static str>)> {
    let base = base_dir(file);
    let outcome = session.eval(src, base.as_deref(), None)?;
    let outcome = match (outcome.err, outcome.exit_code) {
        (Some(err), _) => {
            ui::traceback(&err);
            return Ok((false, None));
        }
        // The file drove run() itself.
        (None, Some(code)) => return Ok((code == 0, None)),
        (None, None) => session.eval(TEST_DRIVER, None, None)?,
    };
    if let Some(err) = outcome.err {
        ui::traceback(&err);
        return Ok((false, None));
    }
    Ok(match outcome.exit_code {
        Some(0) => (true, None),
        Some(3) => (false, Some("no tests registered")),
        _ => (false, None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_skips_dist_and_hidden() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["a_test.py", "sub/b_test.py", "dist/c_test.py", ".git/d_test.py", "util.py"] {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        let names: Vec<_> = discover_tests(dir.path()).iter().map(|p| p.strip_prefix(dir.path()).unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(names, ["a_test.py", "sub/b_test.py"]);
    }
}
