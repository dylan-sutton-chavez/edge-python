use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compiler::modules::dir_of;
use compiler::vm::Limits;

use crate::host::{driver, now_ns, Host, Project, Sink, Status, Vm};
use crate::pack::{base64_decode, Bundle, BUNDLE_TAG};

use super::config::{Message, Out};

// Epoch ticks an untrusted run may consume, the ticker advances one every 100 ms.
const EVAL_DEADLINE_TICKS: u64 = 100;
const EVAL_TICK_NS: u64 = 100_000_000;
// The reply when host-side waits would outlast the deadline, worded like the epoch trap.
const EVAL_TIME_LIMIT: &str = "error: RuntimeError: run exceeded its time limit";
// Linear memory an untrusted run may grow to.
const EVAL_MEMORY: usize = 256 << 20;

// How an actor runs, a fixed program looping over receive(), or an untrusted per-message evaluator.
enum Mode {
    // A persistent interpreter driven by push_event into receive().
    Fixed { vm: Box<Vm>, started: bool },
    // Boots a fresh isolated interpreter per message, no state or send between snippets.
    Eval { limits: Limits, preempt: usize },
}

/* What a group hands an actor for a step, the pieces a fresh eval interpreter needs. */
#[derive(Clone)]
pub struct Context {
    pub host: Rc<Host>,
    pub source: Rc<String>,
    pub out: Out,
    // The group's edge.json, what an eval snippet without its own resolves through.
    pub manifest: String,
}

// One live actor plus the mailbox its work drains from.
pub struct Actor {
    mode: Mode,
    // A deque so draining the oldest message is O(1) instead of shifting the whole buffer.
    pub mailbox: VecDeque<Message>,
    pub done: bool,
    // False until the first step, a fresh fixed actor runs once to reach its first receive().
    pub ran: bool,
    // True when parked in receive() with an empty mailbox, the load balancer's free signal.
    pub idle: bool,
    // Set when a wait ended, the actor runs again even with an empty mailbox.
    pub runnable: bool,
    // Wall-clock deadline of a sleep, the scheduler wakes the actor past it.
    pub wake_at: Option<u64>,
    // The message fed into the interpreter this step, so the scheduler can retry it on a crash.
    in_flight: Option<Message>,
    // Set once the program took a message from its mailbox.
    consumed: bool,
}

// What a run step left the actor waiting on.
pub enum Step {
    // Ran to completion, the actor can be retired.
    Done,
    // Parked in receive(), feed it a message to wake it.
    Waiting,
    // Parked on a timer until the wall-clock deadline.
    Sleeping(u64),
    // Parked on a host call a worker thread is answering.
    Blocked,
    // Raised, carries the traceback and the message that was being processed.
    Failed(String, Option<Message>),
}

impl Actor {
    // A fixed actor wraps a booted interpreter, the program starts on its first step.
    pub fn fixed(vm: Vm) -> Self {
        Self::new(Mode::Fixed { vm: Box::new(vm), started: false })
    }

    // An eval actor holds only the settings to boot a fresh interpreter per snippet.
    pub fn eval(limits: Limits, preempt: usize) -> Self {
        Self::new(Mode::Eval { limits, preempt })
    }

    fn new(mode: Mode) -> Self {
        Actor { mode, mailbox: VecDeque::new(), done: false, ran: false, idle: false, runnable: false, wake_at: None, in_flight: None, consumed: false }
    }

    // Delivers a message to the mailbox, waking the actor from its idle wait.
    pub fn deliver(&mut self, msg: Message) {
        self.mailbox.push_back(msg);
        self.idle = false;
    }

    /* Injects the completions a blocked actor waited on, true when it can run again. */
    pub fn poll(&mut self) -> bool {
        match &mut self.mode {
            Mode::Fixed { vm, .. } => vm.poll().unwrap_or(0) > 0,
            Mode::Eval { .. } => false,
        }
    }

    /* Streams the actor's interpreter still has open. */
    pub fn streams(&self) -> usize {
        match &self.mode {
            Mode::Fixed { vm, .. } => vm.streams(),
            Mode::Eval { .. } => 0,
        }
    }

    /* True once the instance hosting this actor trapped. */
    pub fn trapped(&self) -> bool {
        match &self.mode {
            Mode::Fixed { vm, .. } => vm.instance().borrow().poisoned(),
            Mode::Eval { .. } => false,
        }
    }

    /* The message it was processing and the ones still queued, what a retired actor hands back. */
    pub fn into_messages(mut self) -> (Option<Message>, Vec<Message>) {
        (self.in_flight.take(), std::mem::take(&mut self.mailbox).into_iter().collect())
    }

    /* Queued messages go back out only when the program reads its mailbox, else another run would loop. */
    pub fn leftover(self) -> Vec<Message> {
        if self.consumed { self.mailbox.into_iter().collect() } else { Vec::new() }
    }

    /* Everything the last step sent, an eval actor never sends. */
    pub fn take_sends(&mut self) -> Vec<(String, String)> {
        match &mut self.mode {
            Mode::Fixed { vm, .. } => vm.take_sends(),
            Mode::Eval { .. } => Vec::new(),
        }
    }

    pub fn step(&mut self, ctx: &Context) -> Step {
        self.ran = true;
        self.runnable = false;
        match &mut self.mode {
            Mode::Fixed { .. } => self.step_fixed(ctx),
            Mode::Eval { .. } => self.step_eval(ctx),
        }
    }

    // Drives the persistent interpreter, feeding one mailbox message per receive().
    fn step_fixed(&mut self, ctx: &Context) -> Step {
        let Mode::Fixed { vm, started } = &mut self.mode else { unreachable!() };
        loop {
            let result = if *started {
                vm.resume()
            } else {
                *started = true;
                vm.start(&ctx.source, None)
            };
            let status = match result {
                Ok(status) => status,
                Err(e) => {
                    self.done = true;
                    return Step::Failed(format!("error: {e}"), self.in_flight.take());
                }
            };
            match status {
                Status::Done | Status::Exit(_) => {
                    self.done = true;
                    return Step::Done;
                }
                Status::Preempted => continue,
                Status::PendingEvent => {
                    self.in_flight = None;
                    // A stream event that arrived mid-step goes before the mailbox.
                    if vm.drain_buffered() > 0 {
                        continue;
                    }
                    let Some(msg) = self.mailbox.pop_front() else {
                        self.idle = true;
                        return Step::Waiting;
                    };
                    if vm.push_event(&msg.body) {
                        self.consumed = true;
                        self.in_flight = Some(msg);
                        continue;
                    }
                    self.mailbox.push_front(msg);
                    self.idle = true;
                    return Step::Waiting;
                }
                Status::PendingTimer(deadline) => return Step::Sleeping(deadline),
                Status::PendingHostCall => {
                    vm.dispatch();
                    if vm.inflight() > 0 {
                        return Step::Blocked;
                    }
                    self.done = true;
                    return Step::Failed(format!("error: {}", driver::suspend_message("a host call")), self.in_flight.take());
                }
                Status::PendingFrame => {
                    self.done = true;
                    return Step::Failed(format!("error: {}", driver::suspend_message("a render frame")), self.in_flight.take());
                }
                Status::Error(tb) => {
                    self.done = true;
                    return Step::Failed(tb, self.in_flight.take());
                }
            }
        }
    }

    /* Runs each message in a fresh capped interpreter, an eval actor keeps no state and never sends. */
    fn step_eval(&mut self, ctx: &Context) -> Step {
        let Mode::Eval { limits, preempt } = &self.mode else { unreachable!() };
        let (limits, preempt) = (*limits, *preempt);
        while let Some(msg) = self.mailbox.pop_front() {
            let outcome = run_eval(ctx, &msg.body, limits, preempt, msg.reply.is_some());
            if let Some(reply) = msg.reply {
                let _ = reply.send(outcome);
            }
        }
        self.idle = true;
        Step::Waiting
    }
}

/* One untrusted program to its end, Err is the traceback, Ok the print a waiting caller gets. */
fn run_eval(ctx: &Context, body: &str, limits: Limits, preempt: usize, capture: bool) -> Result<String, String> {
    let (source, files, entry_dir) = match unbundle(body) {
        Some((source, files, entry_dir)) => (source, files, entry_dir),
        None => (body.to_string(), HashMap::new(), String::new()),
    };
    let buffer = Arc::new(Mutex::new(String::new()));
    let sink: Sink = if capture {
        let buffer = buffer.clone();
        Box::new(move |s: &str| {
            if let Ok(mut b) = buffer.lock() {
                b.push_str(s);
            }
        })
    } else {
        sink_for(&ctx.out)
    };
    let own_manifest = files.contains_key(&format!("{entry_dir}edge.json"));
    let mut project = Project::bundle(files, &entry_dir, true);
    if !own_manifest {
        project.manifest = Some(ctx.manifest.clone());
    }
    let mut vm = ctx.host.vm(sink, project, Some(EVAL_DEADLINE_TICKS), Some(EVAL_MEMORY)).map_err(|e| format!("error: {e}"))?;
    vm.set_preempt_interval(preempt).map_err(|e| format!("error: {e}"))?;
    vm.set_limits(&limits).map_err(|e| format!("error: {e}"))?;
    // Host-side waits count toward the same budget the epoch ticker enforces inside the sandbox.
    let deadline = now_ns() + EVAL_DEADLINE_TICKS * EVAL_TICK_NS;
    let mut status = vm.start(&source, None).map_err(|e| format!("error: {e}"))?;
    loop {
        status = match status {
            Status::Done | Status::Exit(_) => break,
            Status::Error(tb) => return Err(tb),
            Status::Preempted => vm.resume().map_err(|e| format!("error: {e}"))?,
            Status::PendingTimer(wake) => {
                if wake > deadline {
                    return Err(EVAL_TIME_LIMIT.to_string());
                }
                std::thread::sleep(Duration::from_nanos(wake.saturating_sub(now_ns())));
                vm.resume().map_err(|e| format!("error: {e}"))?
            }
            Status::PendingHostCall => {
                vm.dispatch();
                if vm.inflight() == 0 {
                    return Err(format!("error: {}", driver::suspend_message("a host call")));
                }
                let left = Duration::from_nanos(deadline.saturating_sub(now_ns()));
                if vm.wait(Some(left)).map_err(|e| format!("error: {e}"))? == 0 {
                    return Err(EVAL_TIME_LIMIT.to_string());
                }
                vm.resume().map_err(|e| format!("error: {e}"))?
            }
            Status::PendingEvent => return Err(format!("error: {}", driver::suspend_message("receive()"))),
            Status::PendingFrame => return Err(format!("error: {}", driver::suspend_message("a render frame"))),
        };
    }
    drop(vm);
    Ok(buffer.lock().map(|b| b.clone()).unwrap_or_default())
}

/* Decodes a bundled project into an in-memory tree, its entry source and entry dir. */
fn unbundle(body: &str) -> Option<(String, HashMap<String, Vec<u8>>, String)> {
    let b64 = body.strip_prefix(BUNDLE_TAG)?;
    let bytes = base64_decode(b64.trim())?;
    let bundle = Bundle::decode(&bytes).ok()?;
    let entry = bundle.entry.clone();
    let files = bundle.into_files();
    let source = String::from_utf8_lossy(files.get(&entry)?).into_owned();
    Some((source, files, dir_of(&entry).to_string()))
}

/* The print sink a group's setting names, a file opens in append mode. */
pub fn sink_for(out: &Out) -> Sink {
    match out {
        Out::Stdout => driver::stdout_sink(),
        Out::Null => Box::new(|_: &str| {}),
        Out::File(path) => match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(file) => {
                let file = Mutex::new(file);
                Box::new(move |s: &str| {
                    use std::io::Write;
                    if let Ok(mut f) = file.lock() {
                        let _ = f.write_all(s.as_bytes());
                    }
                })
            }
            Err(_) => driver::stdout_sink(),
        },
    }
}
