use compiler::lexer::lex;
use compiler::parser::{Diagnostic, Parser};
use compiler::vm::types::{SchedulerStatus, VmErr};
use compiler::vm::{Limits, VM};
use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

// Streams one print payload to stdout, flushed so it interleaves with stderr.
fn stream_stdout(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

fn pending_name(status: &SchedulerStatus) -> &'static str {
    match status {
        SchedulerStatus::PendingTimer(_) => "a timer",
        SchedulerStatus::PendingFrame => "a render frame",
        SchedulerStatus::PendingEvent => "an event",
        SchedulerStatus::PendingHostCall => "a host call",
        SchedulerStatus::Preempted => "a preempt resume",
        SchedulerStatus::Done => "nothing",
    }
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: edge-native <file.py>");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{path}': {e}");
            return ExitCode::from(2);
        }
    };
    let (tokens, lex_errs) = lex(&src);
    let mut parser = Parser::new(&src, tokens.into_iter());
    for e in lex_errs {
        parser.errors.push(Diagnostic { start: e.start, end: e.end, msg: e.msg.into() });
    }
    let (mut chunk, errs) = parser.parse();
    if !errs.is_empty() {
        for e in &errs {
            eprintln!("{}", e.render(&src, Some(&path)));
        }
        return ExitCode::FAILURE;
    }
    compiler::vm::optimizer::constant_fold(&mut chunk);
    let mut vm = VM::with_limits(&chunk, Limits::sandbox());
    vm.strict_input = true;
    vm.print_hook = Some(stream_stdout);
    let mut stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let mut buf = String::new();
        // Piped stdin mirrors the wasm host `set_input` line split.
        if stdin.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
            vm.input_buffer = buf.split('\n').map(String::from).collect();
        }
    }
    match vm.run() {
        Ok(_) | Err(VmErr::HostYield(SchedulerStatus::Done)) => ExitCode::SUCCESS,
        Err(VmErr::HostYield(status)) => {
            eprintln!("error: script suspended awaiting {}, no host wired in this binary", pending_name(&status));
            ExitCode::FAILURE
        }
        Err(e) => {
            if let Some(code) = vm.system_exit_code() {
                return ExitCode::from((code & 0xFF) as u8);
            }
            let traceback = e.render_traceback(&src, vm.error_pos(), Some(&path), vm.call_stack_frames(), vm.function_names_ref());
            eprintln!("{traceback}");
            ExitCode::FAILURE
        }
    }
}
