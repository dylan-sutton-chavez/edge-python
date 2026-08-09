use crate::bridge::VmGuard;
use crate::vm::types::{SchedulerStatus, VmErr};
use crate::vm::{Limits, VM};

use super::config::Message;

// How a node runs, a fixed program looping over receive(), or an untrusted per-message evaluator.
enum Mode {
    // A persistent VM sharing the group chunk, driven by push_event into receive().
    Fixed(Box<VM<'static>>),
    // Recompiles each message as a fresh isolated program, no state or send between snippets.
    Eval { dir: String, limits: Limits, preempt: usize },
}

// One live node plus the mailbox its work drains from.
pub struct Node {
    mode: Mode,
    // A deque so draining the oldest message is O(1) instead of shifting the whole buffer.
    pub mailbox: std::collections::VecDeque<Message>,
    pub done: bool,
    // False until the first step, a fresh fixed node runs once to reach its first receive().
    pub ran: bool,
    // True when parked in receive() with an empty mailbox, the load balancer's free signal.
    pub idle: bool,
    // The message fed into the VM this step, so the scheduler can retry it on a crash.
    in_flight: Option<Message>,
}

// What a run step left the node waiting on.
pub enum Step {
    // Ran to completion, the node can be retired.
    Done,
    // Parked in receive(), feed it a message to wake it.
    Waiting,
    // Raised, carries the traceback and the message that was being processed.
    Failed(String, Option<Message>),
}

impl Node {
    // A fixed node wraps a booted VM sharing the group chunk.
    pub fn fixed(vm: VM<'static>) -> Self {
        Node { mode: Mode::Fixed(Box::new(vm)), mailbox: std::collections::VecDeque::new(), done: false, ran: false, idle: false, in_flight: None }
    }

    // An eval node holds only the settings to boot a fresh VM per snippet.
    pub fn eval(dir: String, limits: Limits, preempt: usize) -> Self {
        Node { mode: Mode::Eval { dir, limits, preempt }, mailbox: std::collections::VecDeque::new(), done: false, ran: false, idle: false, in_flight: None }
    }

    // Delivers a message to the mailbox, waking the node from its idle wait.
    pub fn deliver(&mut self, msg: Message) {
        self.mailbox.push_back(msg);
        self.idle = false;
    }

    pub fn step(&mut self, src: &str) -> Step {
        self.ran = true;
        match &mut self.mode {
            Mode::Fixed(_) => self.step_fixed(src),
            Mode::Eval { .. } => self.step_eval(),
        }
    }

    // Drives the persistent fixed VM, feeding one mailbox message per receive().
    fn step_fixed(&mut self, src: &str) -> Step {
        let Mode::Fixed(vm) = &mut self.mode else { unreachable!() };
        loop {
            if let Some(msg) = self.mailbox.front()
                && vm.push_event(&msg.body).is_ok() {
                self.in_flight = self.mailbox.pop_front();
            }
            let result = { let _guard = VmGuard::new(vm); vm.run() };
            match result {
                Ok(_) | Err(VmErr::HostYield(SchedulerStatus::Done)) => {
                    self.done = true;
                    return Step::Done;
                }
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => continue,
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    self.in_flight = None;
                    if self.mailbox.is_empty() {
                        self.idle = true;
                        return Step::Waiting;
                    }
                }
                Err(VmErr::HostYield(_)) => return Step::Waiting,
                Err(e) => {
                    if vm.system_exit_code().is_some() {
                        self.done = true;
                        return Step::Done;
                    }
                    let tb = e.render_traceback(src, vm.error_pos(), None, vm.call_stack_frames(), vm.function_names_ref());
                    self.done = true;
                    return Step::Failed(tb, self.in_flight.take());
                }
            }
        }
    }

    /* Runs each queued message as its own fresh program, the chunk drops with the VM so nothing
       leaks and no state crosses between snippets. An eval node never sends, so it cannot orchestrate. */
    fn step_eval(&mut self) -> Step {
        let Mode::Eval { dir, limits, preempt } = &self.mode else { unreachable!() };
        let (dir, limits, preempt) = (dir.clone(), *limits, *preempt);
        while let Some(msg) = self.mailbox.pop_front() {
            if msg.reply.is_some() {
                super::scheduler::capture_begin();
            }
            let mut failed = None;
            // A bundled project rides base64 behind a marker, unpacked to an isolated temp dir.
            let (source, base, _hold) = match unbundle(&msg.body) {
                // The resolver walks up from `{base}packages.json`, so the temp dir base needs a trailing slash.
                Some((src, tmp)) => { let p = format!("{}/", tmp.path().to_string_lossy()); (src, p, Some(tmp)) }
                None => (msg.body.clone(), dir.clone(), None),
            };
            match crate::native::parse_eval(&source, &base, None) {
                Err(e) => failed = Some(e),
                Ok(chunk) => {
                    let chunk: Box<crate::parser::SSAChunk> = Box::new(chunk);
                    // SAFETY the chunk outlives the vm here, both are dropped at the end of this block.
                    let chunk_ref: &'static crate::parser::SSAChunk = unsafe { &*(chunk.as_ref() as *const _) };
                    let mut vm = VM::with_limits(chunk_ref, limits);
                    vm.strict_input = true;
                    vm.set_time_hook(crate::native::now_ns);
                    vm.set_preempt_interval(preempt);
                    vm.print_hook = Some(super::scheduler::print_stdout);
                    loop {
                        let result = { let _guard = VmGuard::new(&mut vm); vm.run() };
                        match result {
                            Err(VmErr::HostYield(SchedulerStatus::Preempted)) => continue,
                            Err(VmErr::HostYield(_)) | Ok(_) => break,
                            Err(e) => {
                                failed = Some(e.render_traceback(&source, vm.error_pos(), None, vm.call_stack_frames(), vm.function_names_ref()));
                                break;
                            }
                        }
                    }
                    // vm dropped before chunk, then chunk dropped, no leak.
                    drop(vm);
                }
            }
            if let Some(reply) = msg.reply {
                let out = super::scheduler::capture_take();
                let _ = reply.send(match failed {
                    Some(e) => Err(e),
                    None => Ok(out),
                });
            }
        }
        self.idle = true;
        Step::Waiting
    }
}

// Marks an eval body that carries a base64 project bundle rather than a raw snippet.
const BUNDLE_TAG: &str = "EDGEPKG:";

/* Decodes and materializes a bundled project into a temp dir, returning its entry source and the dir.
   The dir drops with the caller so the untrusted tree never outlives the run. */
fn unbundle(body: &str) -> Option<(String, tempfile::TempDir)> {
    let b64 = body.strip_prefix(BUNDLE_TAG)?;
    let bytes = crate::util::ws::base64_decode(b64.trim())?;
    let bundle = crate::native::pack::Bundle::decode(&bytes).ok()?;
    let tmp = tempfile::tempdir().ok()?;
    let entry = bundle.write_to(tmp.path()).ok()?;
    let source = std::fs::read_to_string(&entry).ok()?;
    Some((source, tmp))
}
