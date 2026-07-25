/*
`edge test` discover *_test.py files and drive each through one engine session. Verdicts come only from SystemExit codes, never from parsed output.
*/

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::engine::{self, Session};
use crate::pkg::Manifest;
use crate::ui;

// Runs registered tests; exit 3 flags an empty file.
const DRIVER: &str = "import test\nif not test._tests:\n    raise SystemExit(3)\ntest.run()";

pub fn run(manifest_path: &Path, path: Option<&Path>) -> Result<()> {
    let target = path.unwrap_or(Path::new("."));
    let files = if target.is_file() {
        vec![target.to_path_buf()]
    } else {
        let mut found = Vec::new();
        discover(target, &mut found);
        found.sort();
        found
    };
    if files.is_empty() {
        bail!("no *_test.py files found under {}", target.display());
    }

    let manifest = Manifest::load(manifest_path)?;
    let mut session = open_or_die(&manifest);

    let started = std::time::Instant::now();
    let mut failed = 0usize;
    for (i, file) in files.iter().enumerate() {
        if i > 0 && session.reset().is_err() {
            drop(session);
            session = open_or_die(&manifest);
        }
        let result = std::fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))
            .and_then(|src| run_file(&mut session, &src, file));
        let (ok, reason) = match result {
            Ok(v) => v,
            // A wedged session poisons later files; reopen.
            Err(e) => {
                ui::error(&e);
                drop(session);
                session = open_or_die(&manifest);
                (false, Some("error"))
            }
        };
        let name = file.strip_prefix(".").unwrap_or(file);
        ui::test_verdict(ok, &name.display().to_string(), reason);
        if !ok { failed += 1; }
    }

    ui::test_summary(files.len() - failed, files.len(), started.elapsed().as_secs_f64());
    drop(session);
    if failed > 0 { std::process::exit(1); }
    Ok(())
}

/// Exit 2 keeps infra failures distinct from red tests.
fn open_or_die(manifest: &Manifest) -> Session {
    match Session::open(manifest) {
        Ok(s) => s,
        Err(e) => {
            ui::error(&e);
            std::process::exit(2);
        }
    }
}

/// Eval the file, then the driver when it didn't exit itself.
fn run_file(session: &mut Session, src: &str, file: &Path) -> Result<(bool, Option<&'static str>)> {
    let base = engine::base_dir(file);
    let outcome = session.eval(src, base.as_deref(), engine::emit_chunk)?;
    let outcome = match (outcome.err, outcome.exit_code) {
        (Some(err), _) => {
            ui::traceback(&err);
            return Ok((false, None));
        }
        // The file drove run() itself.
        (None, Some(code)) => return Ok((code == 0, None)),
        (None, None) => session.eval(DRIVER, None, engine::emit_chunk)?,
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

/// Collect *_test.py under `dir`, skipping dist and hidden entries.
fn discover(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name == "dist" { continue; }
        let path = entry.path();
        if path.is_dir() {
            discover(&path, out);
        } else if name.ends_with("_test.py") {
            out.push(path);
        }
    }
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
        let mut found = Vec::new();
        discover(dir.path(), &mut found);
        found.sort();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(dir.path()).unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["a_test.py", "sub/b_test.py"]);
    }

    #[test]
    fn base_dir_maps_nested_files_only() {
        assert_eq!(engine::base_dir(Path::new("tests/a_test.py")), Some("tests/".into()));
        assert_eq!(engine::base_dir(Path::new("./sub/a_test.py")), Some("sub/".into()));
        assert_eq!(engine::base_dir(Path::new("a_test.py")), None);
        assert_eq!(engine::base_dir(Path::new("../x_test.py")), None);
    }
}
