use std::collections::{HashMap, VecDeque};

use slab::Slab;

use crate::parser::SSAChunk;
use crate::vm::Limits;

use super::config::{Group, Message, Out, ActorConfig};
use super::actor::{Actor, Step};
use super::pool::Router;

// A parsed group ready to boot actors from, chunk shared across every replica.
struct GroupState {
    source: String,
    // Fixed groups share one parsed chunk, eval groups compile each message so it is None.
    chunk: Option<&'static SSAChunk>,
    dir: String,
    retry: usize,
    // Actor ceiling, actors spawn lazily up to this instead of all at boot.
    max: usize,
    limits: Limits,
    preempt: usize,
    out: Out,
    // Stable keys survive removal, so the work queues below never dangle.
    actors: Slab<Actor>,
    // Keys with work to run, drained each tick instead of scanning every actor.
    ready: VecDeque<usize>,
    // Keys parked in receive(), popped first when a message needs a actor.
    idle_free: Vec<usize>,
}

// The single-threaded cooperative loop, one instance owns every actor in the actor.
pub struct Scheduler {
    groups: Vec<GroupState>,
    by_name: HashMap<String, usize>,
    pending: Vec<Message>,
    // Tracebacks of actors that raised uncaught and were retired.
    crashes: Vec<String>,
    // Set in sharded mode, routes sends whose group lives on another thread.
    router: Option<Router>,
    // Live counters published for the control endpoint, None when no control port is set.
    stats: Option<std::sync::Arc<super::server::Stats>>,
}

impl Scheduler {
    pub fn new(config: ActorConfig) -> Result<Self, String> {
        let mut groups = Vec::new();
        let mut by_name = HashMap::new();
        let mut pending = Vec::new();
        for g in config.groups {
            by_name.insert(g.name.clone(), groups.len());
            pending.extend(g.inbox.iter().map(|m| Message { group: m.group.clone(), body: m.body.clone(), attempts: 0, reply: None }));
            groups.push(GroupState::boot(g, config.max_actors)?);
        }
        Ok(Scheduler { groups, by_name, pending, crashes: Vec::new(), router: None, stats: None })
    }

    // Wires the shared counters the control endpoint reads, published each tick.
    pub fn set_stats(&mut self, stats: Option<std::sync::Arc<super::server::Stats>>) {
        self.stats = stats;
    }

    // Pumps messages and runs actors until nothing is left to deliver or run.
    pub fn run(&mut self) -> i32 {
        self.spawn_seed();
        loop {
            self.route_pending();
            if !self.tick() && self.pending.is_empty() {
                break;
            }
        }
        self.report()
    }

    // A live server, runs local work then blocks on the ingress instead of ending.
    pub fn run_serving(&mut self, rx: std::sync::mpsc::Receiver<Message>, wal: std::sync::Arc<std::sync::Mutex<super::server::Wal>>) -> i32 {
        self.spawn_seed();
        loop {
            while let Ok(m) = rx.try_recv() {
                self.pending.push(m);
            }
            self.route_pending();
            if self.tick() || !self.pending.is_empty() {
                continue;
            }
            // Fully drained, publish the idle state and compact the log before parking.
            self.publish_stats();
            wal.lock().unwrap().compact(&self.pending);
            // Idle, wait for the ingress to deliver more, ending only when it closes.
            match rx.recv() {
                Ok(m) => self.pending.push(m),
                Err(_) => break,
            }
        }
        self.report()
    }

    // Boots one actor per group so producers run, the pool then grows lazily on demand.
    fn spawn_seed(&mut self) {
        for g in &mut self.groups {
            if g.max > 0 {
                g.spawn();
            }
        }
    }

    // Same loop across shards, cross-group sends leave by the router, others arrive by rx.
    pub fn run_sharded(&mut self, router: Router, rx: std::sync::mpsc::Receiver<Message>) -> i32 {
        let barrier = router.barrier();
        self.router = Some(router);
        self.spawn_seed();
        loop {
            // Drain what arrived from other shards before running.
            while let Ok(m) = rx.try_recv() {
                barrier.consume();
                self.pending.push(m);
            }
            self.route_pending();
            if self.tick() || !self.pending.is_empty() {
                continue;
            }
            // No local work, block until another shard sends some or the whole actor quiesces.
            if barrier.park_until_work_or_done() {
                break;
            }
        }
        self.report()
    }

    fn report(&self) -> i32 {
        for f in &self.crashes {
            eprintln!("{f}");
        }
        i32::from(!self.crashes.is_empty())
    }

    // Writes the live counts to the shared stats so the control endpoint can read them.
    fn publish_stats(&self) {
        let Some(stats) = &self.stats else { return };
        let mut actors = 0;
        let mut idle = 0;
        for g in &self.groups {
            for (_, n) in &g.actors {
                actors += 1;
                if n.idle {
                    idle += 1;
                }
            }
        }
        stats.set(actors, actors - idle, idle, self.pending.len(), self.crashes.len());
    }

    // Delivers each queued message to a actor of its target group, spawning on demand.
    fn route_pending(&mut self) {
        let msgs = core::mem::take(&mut self.pending);
        for m in msgs {
            let Some(&gi) = self.by_name.get(&m.group) else { continue };
            let g = &mut self.groups[gi];
            if let Some(key) = g.pick() {
                g.actors[key].deliver(m);
                g.ready.push_back(key);
            }
        }
    }

    // Runs each queued actor once, collecting what they send, false when none progressed.
    fn tick(&mut self) -> bool {
        let mut progressed = false;
        for gi in 0..self.groups.len() {
            for _ in 0..self.groups[gi].ready.len() {
                let Some(key) = self.groups[gi].ready.pop_front() else { break };
                let Some(actor) = self.groups[gi].actors.get(key) else { continue };
                if actor.done || (actor.mailbox.is_empty() && actor.ran) {
                    continue;
                }
                progressed = true;
                let src = self.groups[gi].source.clone();
                let step = self.groups[gi].actors[key].step(&src);
                self.collect_sends();
                self.settle(gi, key, step);
            }
        }
        self.publish_stats();
        progressed
    }

    // Re-queues a actor by its step outcome, retiring it from the slab on a crash.
    fn settle(&mut self, gi: usize, key: usize, step: Step) {
        let g = &mut self.groups[gi];
        match step {
            Step::Failed(tb, msg) => {
                g.actors.remove(key);
                self.crashes.push(tb);
                self.handle_crash(gi, msg);
            }
            _ if g.actors[key].done => { g.actors.remove(key); }
            _ if g.actors[key].idle => g.idle_free.push(key),
            _ => g.ready.push_back(key),
        }
    }

    // Retries a crashed message on another actor up to the group's retry count, else drops it dead.
    fn handle_crash(&mut self, gi: usize, msg: Option<Message>) {
        let Some(mut msg) = msg else { return };
        if msg.attempts < self.groups[gi].retry {
            msg.attempts += 1;
            self.pending.push(msg);
        } else if let Some(stats) = &self.stats {
            stats.add_dead();
        }
    }

    // Drains what the actors just sent, routing cross-shard groups out and keeping local ones.
    fn collect_sends(&mut self) {
        for out in crate::native::builtins::actor::drain_outbox() {
            let msg = Message { group: out.group, body: out.body, attempts: 0, reply: None };
            match &self.router {
                Some(r) if !self.by_name.contains_key(&msg.group) => r.route(msg),
                _ => self.pending.push(msg),
            }
        }
    }
}

impl GroupState {
    fn boot(g: Group, max_actors: usize) -> Result<Self, String> {
        // A fixed group parses its program once and shares it, an eval group compiles per message.
        let chunk = if g.eval {
            None
        } else {
            Some(&*Box::leak(Box::new(crate::native::parse_source(&g.source, &g.dir, None)?)))
        };
        Ok(GroupState {
            source: g.source,
            chunk,
            dir: g.dir,
            retry: g.retry,
            max: g.replicas.min(max_actors),
            limits: g.limits,
            preempt: g.preempt,
            out: g.out,
            actors: Slab::new(),
            ready: VecDeque::new(),
            idle_free: Vec::new(),
        })
    }

    // Boots a fresh actor, fixed actors share the group chunk, eval actors start empty and untrusted.
    fn spawn(&mut self) -> usize {
        let actor = match self.chunk {
            Some(chunk) => {
                let mut vm = crate::vm::VM::with_limits(chunk, self.limits);
                vm.set_time_hook(crate::native::now_ns);
                vm.set_preempt_interval(self.preempt);
                wire_output(&mut vm, &self.out);
                Actor::fixed(vm)
            }
            None => Actor::eval(self.dir.clone(), self.limits, self.preempt),
        };
        let key = self.actors.insert(actor);
        self.ready.push_back(key);
        key
    }

    /* Picks a actor for a message, an idle one first, else a fresh spawn under the ceiling, else the least-loaded live actor when the group is saturated. */
    fn pick(&mut self) -> Option<usize> {
        while let Some(key) = self.idle_free.pop() {
            if self.actors.get(key).is_some_and(|n| n.idle && !n.done) {
                return Some(key);
            }
        }
        if self.actors.len() < self.max {
            return Some(self.spawn());
        }
        self.actors.iter().filter(|(_, n)| !n.done).min_by_key(|(_, n)| n.mailbox.len()).map(|(k, _)| k)
    }
}

// Points a actor's print output at stdout or nowhere.
fn wire_output(vm: &mut crate::vm::VM<'static>, out: &Out) {
    match out {
        // A per-file sink needs a closure hook, deferred until print_hook takes one.
        Out::Stdout | Out::File(_) => vm.print_hook = Some(print_stdout),
        Out::Null => vm.print_hook = None,
    }
}

thread_local! {
    // Set while an eval run replies to a caller, its print accrues here instead of stdout.
    static CAPTURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

// Starts accruing print output for the eval run about to execute on this thread.
pub(super) fn capture_begin() {
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
}

// Drains the accrued output, restoring plain stdout printing.
pub(super) fn capture_take() -> String {
    CAPTURE.with(|c| c.borrow_mut().take().unwrap_or_default())
}

pub(super) fn print_stdout(s: &str) {
    use std::io::Write;
    if CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        match c.as_mut() {
            Some(buf) => { buf.push_str(s); true }
            None => false,
        }
    }) {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}
