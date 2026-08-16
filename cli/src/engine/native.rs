use anyhow::Result;
use compiler::native::{boot_vm, drive, drive_session, parse_source, restore_and_run, RunOpts, Step};
use compiler::parser::SSAChunk;
use compiler::vm::{Limits, VM};
use std::io::{IsTerminal, Read};
use std::path::Path;

use super::{Backend, Outcome};

/// The in-process engine behind the same seam as the browser session, no server, no CDP.
pub struct NativeSession {
    vm: Option<VM<'static>>,
    packages: Option<String>,
}

impl NativeSession {
    pub fn open(packages: Option<&Path>) -> Self {
        Self { vm: None, packages: packages.map(path_spec) }
    }
}

impl Backend for NativeSession {
    /// Prints stream straight to stdout through the VM hook, so `on_line` never fires here.
    fn eval(&mut self, src: &str, base: Option<&str>, _on_line: &mut dyn FnMut(&str)) -> Result<Outcome> {
        let chunk = match parse_source(src, base.unwrap_or(""), self.packages.as_deref()) {
            Ok(c) => c,
            Err(rendered) => return Ok(Outcome { err: Some(rendered), exit_code: None }),
        };
        let vm = match self.vm.as_mut() {
            // One interpreter across evals, each input adopts a fresh chunk so history never re-executes.
            Some(v) => {
                let chunk_static: &'static SSAChunk = Box::leak(Box::new(chunk));
                v.adopt_entry_chunk(chunk_static);
                v.reset_budget(Limits::sandbox().ops);
                v
            }
            None => self.vm.insert(boot_vm(chunk, Limits::sandbox(), 0)),
        };
        // Named native imports live only in the chunk extern table, mirror them for later evals.
        if let Err(e) = vm.bind_chunk_externs() {
            let msg = e.render();
            vm.clear_error_state();
            return Ok(Outcome { err: Some(msg), exit_code: None });
        }
        let step = drive_session(vm, src, None);
        vm.clear_error_state();
        Ok(match step {
            Step::Done => Outcome { err: None, exit_code: None },
            Step::Exit(code) => Outcome { err: None, exit_code: Some(code as i32) },
            Step::Error(t) | Step::Suspended(t) => Outcome { err: Some(t), exit_code: None },
        })
    }

    /// Next eval boots a fresh interpreter, dropping the VM is the whole reset.
    fn reset(&mut self) -> Result<()> {
        self.vm = None;
        Ok(())
    }
}

/// One-shot native run mirroring `engine::run`, returns the process exit code.
pub fn run(file: Option<&Path>, opts: &RunOpts) -> Result<i32> {
    if let Some(state) = &opts.restore_state {
        return Ok(restore_and_run(state, opts));
    }
    // A packed .edge or .package runs its unpacked entry, matching a direct `./app.edge`.
    if let Some(path) = file
        && let Some(payload) = crate::cmd::build::file_payload(path) {
        return run_bundle(&payload, opts);
    }
    let mut stdin = std::io::stdin();
    let (src, name) = match file {
        Some(p) => (std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("reading {}: {e}", p.display()))?, path_spec(p)),
        None => {
            // A bare `edge run` from a terminal would block on stdin forever, force a pipe or path.
            if stdin.is_terminal() {
                anyhow::bail!("no script given; pass a file path or pipe Python to stdin");
            }
            let mut s = String::new();
            stdin.read_to_string(&mut s).map_err(|e| anyhow::anyhow!("reading stdin: {e}"))?;
            (s, String::from("<stdin>"))
        }
    };
    let dir = compiler::packages::dir_of(&name).to_string();
    let chunk = match parse_source(&src, &dir, opts.packages.as_deref()) {
        Ok(c) => c,
        Err(rendered) => {
            crate::ui::traceback(&rendered);
            return Ok(1);
        }
    };
    let mut vm = boot_vm(chunk, Limits::sandbox(), opts.preempt);
    // With a file argument piped stdin feeds `input()`, one line per call with any CR dropped.
    if file.is_some() && !stdin.is_terminal() {
        let mut buf = String::new();
        if stdin.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
            vm.input_buffer = buf.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        }
    }
    Ok(drive(&mut vm, &src, Some(&name), opts))
}

/// Unpacks a bundle into a temp dir and runs its entry.
pub fn run_bundle(payload: &[u8], opts: &RunOpts) -> Result<i32> {
    let bundle = compiler::native::pack::Bundle::decode(payload).map_err(|e| anyhow::anyhow!("corrupt bundle: {e}"))?;
    let dir = tempfile::tempdir().map_err(|e| anyhow::anyhow!("creating a temp dir: {e}"))?;
    let entry = bundle.write_to(dir.path()).map_err(|e| anyhow::anyhow!("unpacking the bundle: {e}"))?;
    let src = std::fs::read_to_string(&entry).map_err(|e| anyhow::anyhow!("reading the entry: {e}"))?;
    let name = path_spec(&entry);
    let d = compiler::packages::dir_of(&name).to_string();
    let chunk = match parse_source(&src, &d, None) {
        Ok(c) => c,
        Err(rendered) => { crate::ui::traceback(&rendered); return Ok(1); }
    };
    let mut vm = boot_vm(chunk, Limits::sandbox(), opts.preempt);
    Ok(drive(&mut vm, &src, Some(&name), opts))
}

/// Forward-slash spec of a path, the shape the resolver walks.
fn path_spec(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
