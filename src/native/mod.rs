use crate::lexer::lex;
use crate::parser::{Diagnostic, Parser, SSAChunk};
use crate::vm::{Limits, VM};
use std::io::Write;

mod driver;
mod builtins;
mod io;
mod loader;
pub mod pack;
mod resolver;
pub mod swarm;

pub use driver::{drive, drive_session, restore_and_run, Step};

use resolver::FileResolver;

/* Run flags, every path is host-side, none reach the sandboxed script. */
#[derive(Default)]
pub struct RunOpts {
    pub packages: Option<String>,
    pub preempt: usize,
    pub events: Option<String>,
    pub save_state: Option<String>,
    pub restore_state: Option<String>,
}

// Streams one print payload to stdout, flushed so it interleaves with stderr.
fn stream_stdout(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

// Wall-clock ns, the same base PendingTimer deadlines are minted against.
pub(crate) fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/* Lex and parse with the disk resolver, Err is rendered diagnostics. */
pub fn parse_source(src: &str, dir: &str, packages: Option<&str>) -> Result<SSAChunk, String> {
    parse(src, dir, packages, false)
}

/* Parses untrusted eval code, the resolver withholds the swarm module from it. */
pub fn parse_eval(src: &str, dir: &str, packages: Option<&str>) -> Result<SSAChunk, String> {
    parse(src, dir, packages, true)
}

fn parse(src: &str, dir: &str, packages: Option<&str>, untrusted: bool) -> Result<SSAChunk, String> {
    let (tokens, lex_errs) = lex(src);
    let mut r = FileResolver::new(dir, packages);
    if untrusted {
        r = r.untrusted();
    }
    let mut p = Parser::with_resolver(src, tokens.into_iter(), Box::new(r));
    for e in lex_errs {
        p.errors.push(Diagnostic { start: e.start, end: e.end, msg: e.msg.into() });
    }
    let (mut chunk, errs) = p.parse();
    if !errs.is_empty() {
        let mut buf = String::new();
        for (i, e) in errs.iter().enumerate() {
            if i > 0 { buf.push('\n'); }
            buf.push_str(&e.render(src, None));
        }
        return Err(buf);
    }
    crate::vm::optimizer::constant_fold(&mut chunk);
    Ok(chunk)
}

/* Leak the chunk so plugin callbacks and snapshots outlive this frame, then boot with host hooks. */
pub fn boot_vm(chunk: SSAChunk, limits: Limits, preempt: usize) -> VM<'static> {
    let chunk_static: &'static SSAChunk = Box::leak(Box::new(chunk));
    let mut vm = VM::with_limits(chunk_static, limits);
    vm.strict_input = true;
    vm.print_hook = Some(stream_stdout);
    vm.set_time_hook(now_ns);
    vm.set_preempt_interval(preempt);
    vm
}
