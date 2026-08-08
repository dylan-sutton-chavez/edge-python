use crate::bridge::VmGuard;
use crate::vm::snapshot;
use crate::vm::types::{SchedulerStatus, VmErr};
use crate::vm::VM;
use std::io::BufRead;


use super::RunOpts;

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

/* Serves one run to completion, `src` and `name` feed tracebacks, `opts` wires events and snapshots. */
pub fn drive(vm: &mut VM<'static>, src: &str, name: Option<&str>, opts: &RunOpts) -> i32 {
    let mut events: Option<std::io::BufReader<std::fs::File>> = None;
    let _ = super::io::install();
    loop {
        // The guard publishes the VM so `.so` plugin callbacks can re-enter through `edge_op`.
        let result = { let _guard = VmGuard::new(vm); vm.run() };
        match result {
            Ok(_) | Err(VmErr::HostYield(SchedulerStatus::Done)) => return 0,
            Err(VmErr::HostYield(SchedulerStatus::PendingTimer(deadline))) => {
                let now = super::now_ns();
                if deadline > now { std::thread::sleep(std::time::Duration::from_nanos(deadline - now)); }
            }
            Err(VmErr::HostYield(SchedulerStatus::Preempted)) => {}
            // Streamed events from the reactor (sse and ws) feed receive() first.
            Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) if super::io::has_pending() => {
                if !drive_events(vm) {
                    return park(vm, src, &SchedulerStatus::PendingEvent, opts);
                }
            }
            Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) if opts.events.is_some() => {
                match next_event(&mut events, opts.events.as_deref().unwrap_or_default()) {
                    Some(line) => {
                        if let Err(e) = vm.push_event(&line) {
                            eprintln!("error: cannot inject event: {}", e.render());
                            return 1;
                        }
                    }
                    // A drained events file can never serve the wait, park terminally.
                    None => return park(vm, src, &SchedulerStatus::PendingEvent, opts),
                }
            }
            // Native async host calls, drive the reactor then inject any finished results.
            Err(VmErr::HostYield(SchedulerStatus::PendingHostCall)) if super::io::has_pending() => {
                if !drive_host_calls(vm) {
                    return park(vm, src, &SchedulerStatus::PendingHostCall, opts);
                }
            }
            Err(VmErr::HostYield(status)) => return park(vm, src, &status, opts),
            Err(e) => {
                if let Some(code) = vm.system_exit_code() {
                    return (code & 0xFF) as i32;
                }
                let traceback = e.render_traceback(src, vm.error_pos(), name, vm.call_stack_frames(), vm.function_names_ref());
                eprintln!("{traceback}");
                return 1;
            }
        }
    }
}

// Advances the async runtime one step and injects finished host calls, false when nothing services it.
fn drive_host_calls(vm: &mut VM<'static>) -> bool {
    let drained = super::io::with(|net| {
        // One reactor tick advances the fetch tasks, completions land in the shared buffer.
        net.executor.poll();
        core::mem::take(&mut net.completions.borrow_mut().done)
    });
    let Some(done) = drained else { return false };
    if done.is_empty() {
        // Nothing finished yet, block the thread on the reactor until an event wakes a task.
        super::io::with(|net| net.reactor.tick(None));
        return true;
    }
    for (id, result) in done {
        match result {
            Ok(json) => { let _ = vm.push_host_result_by_id(id, &json); }
            Err(msg) => { vm.inject_host_error_by_id(id, VmErr::Raised(format!("RuntimeError: {msg}"))); }
        }
    }
    true
}

// Advances the runtime and pushes any streamed events into the VM queue, false when unserviced.
fn drive_events(vm: &mut VM<'static>) -> bool {
    let drained = super::io::with(|net| {
        net.executor.poll();
        core::mem::take(&mut net.completions.borrow_mut().events)
    });
    let Some(events) = drained else { return false };
    if events.is_empty() {
        // No event yet, block on the reactor until a stream task wakes.
        super::io::with(|net| net.reactor.tick(None));
        return true;
    }
    for json in events {
        if vm.push_event(&json).is_err() {
            return false;
        }
    }
    true
}

/* The park report, web-only waits point at --web, host-servable ones at their flag. */
fn suspend_message(status: &SchedulerStatus) -> String {
    match status {
        SchedulerStatus::PendingFrame => "script suspended awaiting a render frame, requires the web runtime (run with --web)".to_string(),
        SchedulerStatus::PendingEvent => "script suspended awaiting an event (wire --events <file>)".to_string(),
        s => format!("script suspended awaiting {}, nothing can resume it here", pending_name(s)),
    }
}

/* Unservable park, snapshot when asked, otherwise report the missing wait and fail. */
fn park(vm: &VM<'static>, src: &str, status: &SchedulerStatus, opts: &RunOpts) -> i32 {
    if let Some(file) = &opts.save_state {
        let blob = snapshot::save(vm, src);
        if let Err(e) = std::fs::write(file, blob) {
            eprintln!("error: cannot write state to '{file}': {e}");
            return 1;
        }
        eprintln!("suspended awaiting {}, state saved to '{file}'", pending_name(status));
        return 0;
    }
    eprintln!("error: {}", suspend_message(status));
    1
}

/* Lazy line reader over `--events`, a FIFO blocks until a writer shows up, a file replays. */
fn next_event(reader: &mut Option<std::io::BufReader<std::fs::File>>, path: &str) -> Option<String> {
    if reader.is_none() {
        match std::fs::File::open(path) {
            Ok(f) => *reader = Some(std::io::BufReader::new(f)),
            Err(e) => { eprintln!("error: cannot open events '{path}': {e}"); return None; }
        }
    }
    let mut line = String::new();
    match reader.as_mut()?.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches('\n').to_string()),
    }
}

pub enum Step {
    Done,
    Exit(u8),
    Error(String),
    Suspended(String),
}

/* Session driver for repl and test, serves timers and preempt, hands everything else back as text. */
pub fn drive_session(vm: &mut VM<'static>, src: &str, name: Option<&str>) -> Step {
    loop {
        let result = { let _guard = VmGuard::new(vm); vm.run() };
        match result {
            Ok(_) | Err(VmErr::HostYield(SchedulerStatus::Done)) => return Step::Done,
            Err(VmErr::HostYield(SchedulerStatus::PendingTimer(deadline))) => {
                let now = super::now_ns();
                if deadline > now { std::thread::sleep(std::time::Duration::from_nanos(deadline - now)); }
            }
            Err(VmErr::HostYield(SchedulerStatus::Preempted)) => {}
            Err(VmErr::HostYield(status)) => return Step::Suspended(suspend_message(&status)),
            Err(e) => {
                if let Some(code) = vm.system_exit_code() {
                    return Step::Exit((code & 0xFF) as u8);
                }
                let traceback = e.render_traceback(src, vm.error_pos(), name, vm.call_stack_frames(), vm.function_names_ref());
                return Step::Error(traceback);
            }
        }
    }
}

/* Boot from the blob's embedded source, overlay its saved state, keep driving. */
pub fn restore_and_run(file: &str, opts: &RunOpts) -> i32 {
    let blob = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => { eprintln!("error: cannot read state '{file}': {e}"); return 2; }
    };
    let source = match snapshot::source_of(&blob) {
        Ok(s) => s.to_string(),
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let limits = match snapshot::limits_of(&blob) {
        Ok(l) => l,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let chunk = match super::parse_source(&source, "", opts.packages.as_deref()) {
        Ok(c) => c,
        Err(_) => { eprintln!("error: snapshot source no longer parses; was it saved by another compiler version?"); return 1; }
    };
    let mut vm = super::boot_vm(chunk, limits, opts.preempt);
    if let Err(e) = snapshot::restore(&mut vm, &blob) {
        eprintln!("error: {e}");
        return 1;
    }
    drive(&mut vm, &source, None, opts)
}
