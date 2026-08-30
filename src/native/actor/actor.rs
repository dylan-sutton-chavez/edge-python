use std::sync::OnceLock;
use std::sync::mpsc::{channel, Sender};

use crate::bridge::VmGuard;
use crate::vm::types::{SchedulerStatus, VmErr};
use crate::vm::{Limits, VM};

use super::config::Message;

// How a actor runs, a fixed program looping over receive(), or an untrusted per-message evaluator.
enum Mode {
    // A persistent VM sharing the group chunk, driven by push_event into receive().
    Fixed(Box<VM<'static>>),
    // Recompiles each message as a fresh isolated program, no state or send between snippets.
    Eval { dir: String, limits: Limits, preempt: usize },
}

// One live actor plus the mailbox its work drains from.
pub struct Actor {
    mode: Mode,
    // A deque so draining the oldest message is O(1) instead of shifting the whole buffer.
    pub mailbox: std::collections::VecDeque<Message>,
    pub done: bool,
    // False until the first step, a fresh fixed actor runs once to reach its first receive().
    pub ran: bool,
    // True when parked in receive() with an empty mailbox, the load balancer's free signal.
    pub idle: bool,
    // The message fed into the VM this step, so the scheduler can retry it on a crash.
    in_flight: Option<Message>,
}

// What a run step left the actor waiting on.
pub enum Step {
    // Ran to completion, the actor can be retired.
    Done,
    // Parked in receive(), feed it a message to wake it.
    Waiting,
    // Raised, carries the traceback and the message that was being processed.
    Failed(String, Option<Message>),
}

impl Actor {
    // A fixed actor wraps a booted VM sharing the group chunk.
    pub fn fixed(vm: VM<'static>) -> Self {
        Actor { mode: Mode::Fixed(Box::new(vm)), mailbox: std::collections::VecDeque::new(), done: false, ran: false, idle: false, in_flight: None }
    }

    // An eval actor holds only the settings to boot a fresh VM per snippet.
    pub fn eval(dir: String, limits: Limits, preempt: usize) -> Self {
        Actor { mode: Mode::Eval { dir, limits, preempt }, mailbox: std::collections::VecDeque::new(), done: false, ran: false, idle: false, in_flight: None }
    }

    // Delivers a message to the mailbox, waking the actor from its idle wait.
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

    /* Runs each queued message through the persistent locked executor, the seccomp allowlist confines every syscall it or a plugin it loads attempts, an eval actor keeps no state and never sends so it cannot orchestrate. */
    fn step_eval(&mut self) -> Step {
        let Mode::Eval { dir, limits, preempt } = &self.mode else { unreachable!() };
        let (dir, limits, preempt) = (dir.clone(), *limits, *preempt);
        while let Some(msg) = self.mailbox.pop_front() {
            // A bundled project rides base64 behind a marker, unpacked on this trusted thread so the locked run only reads it.
            let (source, base, _hold) = match unbundle(&msg.body) {
                // The resolver walks up from `{base}packages.json`, so the temp dir base needs a trailing slash.
                Some((src, tmp)) => { let p = format!("{}/", tmp.path().to_string_lossy()); (src, p, Some(tmp)) }
                None => (msg.body.clone(), dir.clone(), None),
            };
            let (failed, out) = run_sandboxed(source, base, limits, preempt, msg.reply.is_some());
            if let Some(reply) = msg.reply {
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

/* Hands one untrusted program to the persistent locked executor and waits for its result, the traceback or, when a caller waits, its captured print output. */
fn run_sandboxed(source: String, base: String, limits: Limits, preempt: usize, capture: bool) -> (Option<String>, String) {
    let (tx, rx) = channel();
    let job = Job { source, base, limits, preempt, capture, reply: tx };
    if executor().send(job).is_err() {
        return (Some("sandbox executor is down".to_string()), String::new());
    }
    rx.recv().unwrap_or_else(|_| (Some("sandbox executor stopped".to_string()), String::new()))
}

// One untrusted run submitted to the executor, its reply carries the traceback or captured output.
struct Job {
    source: String,
    base: String,
    limits: Limits,
    preempt: usize,
    capture: bool,
    reply: Sender<(Option<String>, String)>,
}

/* The one thread every untrusted run executes on, locked to the seccomp allowlist once then reused, so eval actors multiplex over a single confined thread with no per-run spawn. */
fn executor() -> &'static Sender<Job> {
    static EXEC: OnceLock<Sender<Job>> = OnceLock::new();
    EXEC.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            #[cfg(target_os = "linux")]
            let lock = proxy::lock_thread();
            for job in rx {
                #[cfg(target_os = "linux")]
                if let Err(e) = &lock {
                    let _ = job.reply.send((Some(format!("sandbox error: {e}")), String::new()));
                    continue;
                }
                let _ = job.reply.send(execute(&job));
            }
        });
        tx
    })
}

// Compiles and runs one untrusted program on the calling thread, which the executor keeps locked.
fn execute(job: &Job) -> (Option<String>, String) {
    if job.capture {
        super::scheduler::capture_begin();
    }
    let mut failed = None;
    match crate::native::parse_eval(&job.source, &job.base, None) {
        Err(e) => failed = Some(e),
        Ok(chunk) => {
            let chunk: Box<crate::parser::SSAChunk> = Box::new(chunk);
            // SAFETY the chunk outlives the vm here, both drop at the end of this block.
            let chunk_ref: &'static crate::parser::SSAChunk = unsafe { &*(chunk.as_ref() as *const _) };
            let mut vm = VM::with_limits(chunk_ref, job.limits);
            vm.set_time_hook(crate::native::now_ns);
            vm.set_preempt_interval(job.preempt);
            vm.print_hook = Some(super::scheduler::print_stdout);
            loop {
                let result = { let _guard = VmGuard::new(&mut vm); vm.run() };
                match result {
                    Err(VmErr::HostYield(SchedulerStatus::Preempted)) => continue,
                    Err(VmErr::HostYield(_)) | Ok(_) => break,
                    Err(e) => {
                        failed = Some(e.render_traceback(&job.source, vm.error_pos(), None, vm.call_stack_frames(), vm.function_names_ref()));
                        break;
                    }
                }
            }
            drop(vm);
        }
    }
    let out = if job.capture { super::scheduler::capture_take() } else { String::new() };
    (failed, out)
}

// Marks an eval body that carries a base64 project bundle rather than a raw snippet.
const BUNDLE_TAG: &str = "EDGEPKG:";

/* Decodes and materializes a bundled project into a temp dir, returning its entry source and the dir, the dir drops with the caller so the untrusted tree never outlives the run. */
fn unbundle(body: &str) -> Option<(String, tempfile::TempDir)> {
    let b64 = body.strip_prefix(BUNDLE_TAG)?;
    let bytes = crate::util::ws::base64_decode(b64.trim())?;
    let bundle = crate::native::pack::Bundle::decode(&bytes).ok()?;
    let tmp = tempfile::tempdir().ok()?;
    let entry = bundle.write_to(tmp.path()).ok()?;
    let source = std::fs::read_to_string(&entry).ok()?;
    Some((source, tmp))
}
