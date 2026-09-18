use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slab::Slab;

use compiler::vm::Limits;

use crate::host::{now_ns, Host, Project, Runtime};

use super::actor::{sink_for, Actor, Context, Step};
use super::config::{ActorConfig, Group, Message};
use super::pool::Router;

// How long an idle scheduler naps before re-checking sleeping and blocked actors.
const NAP: Duration = Duration::from_millis(2);

// A group ready to boot actors from, every replica compiles the shared source itself.
struct GroupState {
    ctx: Context,
    dir: String,
    packages: Option<String>,
    retry: usize,
    // Actor ceiling, actors spawn lazily up to this instead of all at boot.
    max: usize,
    limits: Limits,
    preempt: usize,
    eval: bool,
    // Stable keys survive removal, so the work queues below never dangle.
    actors: Slab<Actor>,
    // Keys with work to run, drained each tick instead of scanning every actor.
    ready: VecDeque<usize>,
    // Keys parked in receive(), popped first when a message needs an actor.
    idle_free: Vec<usize>,
    // Keys parked on a timer or a host call, re-checked each tick.
    waiting: Vec<usize>,
}

// The single-threaded cooperative loop, one instance owns every actor in the shard.
pub struct Scheduler {
    groups: Vec<GroupState>,
    by_name: HashMap<String, usize>,
    pending: Vec<Message>,
    // Tracebacks of actors that raised uncaught and were retired.
    crashes: Vec<String>,
    // Set in sharded mode, routes sends whose group lives on another thread.
    router: Option<Router>,
    // Live counters published for the control endpoint, None when no control port is set.
    stats: Option<Arc<super::server::Stats>>,
}

impl Scheduler {
    pub fn new(config: ActorConfig, runtime: Arc<Runtime>) -> Result<Self, String> {
        let host = Host::new(runtime).map_err(|e| e.to_string())?;
        let mut groups = Vec::new();
        let mut by_name = HashMap::new();
        let mut pending = Vec::new();
        for g in config.groups {
            by_name.insert(g.name.clone(), groups.len());
            pending.extend(g.inbox.iter().map(|m| Message { group: m.group.clone(), body: m.body.clone(), attempts: 0, reply: None }));
            groups.push(GroupState::boot(g, config.max_actors, host.clone()));
        }
        Ok(Scheduler { groups, by_name, pending, crashes: Vec::new(), router: None, stats: None })
    }

    // Wires the shared counters the control endpoint reads, published each tick.
    pub fn set_stats(&mut self, stats: Option<Arc<super::server::Stats>>) {
        self.stats = stats;
    }

    // Pumps messages and runs actors until nothing is left to deliver or run.
    pub fn run(&mut self) -> i32 {
        self.spawn_seed();
        loop {
            self.route_pending();
            if self.tick() || !self.pending.is_empty() {
                continue;
            }
            if self.has_waiters() {
                std::thread::sleep(NAP);
                continue;
            }
            break;
        }
        self.report()
    }

    // A live server, runs local work then blocks on the ingress instead of ending.
    pub fn run_serving(&mut self, rx: std::sync::mpsc::Receiver<Message>, wal: Arc<std::sync::Mutex<super::server::Wal>>) -> i32 {
        self.spawn_seed();
        loop {
            while let Ok(m) = rx.try_recv() {
                self.pending.push(m);
            }
            self.route_pending();
            if self.tick() || !self.pending.is_empty() {
                continue;
            }
            // Actors parked on timers or host calls keep the loop polling.
            if self.has_waiters() {
                match rx.recv_timeout(NAP) {
                    Ok(m) => self.pending.push(m),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
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
            if self.has_waiters() {
                std::thread::sleep(NAP);
                continue;
            }
            // No local work, block until another shard sends some or the whole pool quiesces.
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

    fn has_waiters(&self) -> bool {
        self.groups.iter().any(|g| !g.waiting.is_empty())
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

    // Delivers each queued message to an actor of its target group, spawning on demand.
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
        let now = now_ns();
        for gi in 0..self.groups.len() {
            self.groups[gi].wake(now);
            for _ in 0..self.groups[gi].ready.len() {
                let Some(key) = self.groups[gi].ready.pop_front() else { break };
                let Some(actor) = self.groups[gi].actors.get(key) else { continue };
                if actor.done || (actor.mailbox.is_empty() && actor.ran && !actor.runnable) {
                    continue;
                }
                progressed = true;
                let ctx = self.groups[gi].ctx.clone();
                let step = self.groups[gi].actors[key].step(&ctx);
                self.collect_sends(gi, key);
                self.settle(gi, key, step);
            }
        }
        self.publish_stats();
        progressed
    }

    // Re-queues an actor by its step outcome, retiring it from the slab on a crash.
    fn settle(&mut self, gi: usize, key: usize, step: Step) {
        let g = &mut self.groups[gi];
        match step {
            Step::Failed(tb, msg) => {
                g.actors.remove(key);
                self.crashes.push(tb);
                self.handle_crash(gi, msg);
            }
            Step::Sleeping(deadline) => {
                g.actors[key].wake_at = Some(deadline);
                g.waiting.push(key);
            }
            Step::Blocked => g.waiting.push(key),
            _ if g.actors[key].done => {
                g.actors.remove(key);
            }
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

    // Drains what the actor just sent, routing cross-shard groups out and keeping local ones.
    fn collect_sends(&mut self, gi: usize, key: usize) {
        let sent = match self.groups[gi].actors.get_mut(key) {
            Some(actor) => actor.take_sends(),
            None => return,
        };
        for (group, body) in sent {
            let msg = Message { group, body, attempts: 0, reply: None };
            match &self.router {
                Some(r) if !self.by_name.contains_key(&msg.group) => r.route(msg),
                _ => self.pending.push(msg),
            }
        }
    }
}

impl GroupState {
    fn boot(g: Group, max_actors: usize, host: Rc<Host>) -> Self {
        GroupState {
            ctx: Context { host, source: Rc::new(g.source), out: g.out },
            dir: g.dir,
            packages: g.packages,
            retry: g.retry,
            max: g.replicas.min(max_actors),
            limits: g.limits,
            preempt: g.preempt,
            eval: g.eval,
            actors: Slab::new(),
            ready: VecDeque::new(),
            idle_free: Vec::new(),
            waiting: Vec::new(),
        }
    }

    /* Boots a fresh actor with its own interpreter unless it evals, None when no slot is left. */
    fn spawn(&mut self) -> Option<usize> {
        let actor = if self.eval {
            Actor::eval(self.limits, self.preempt)
        } else {
            let project = Project::disk(&self.dir, self.packages.as_deref());
            let mut vm = self.ctx.host.vm(sink_for(&self.ctx.out), project, None, None).ok()?;
            vm.set_preempt_interval(self.preempt).ok()?;
            vm.set_limits(&self.limits).ok()?;
            Actor::fixed(vm)
        };
        let key = self.actors.insert(actor);
        self.ready.push_back(key);
        Some(key)
    }

    /* Picks an idle actor, else a fresh spawn under the ceiling, else the least-loaded live one. */
    fn pick(&mut self) -> Option<usize> {
        while let Some(key) = self.idle_free.pop() {
            if self.actors.get(key).is_some_and(|n| n.idle && !n.done) {
                return Some(key);
            }
        }
        if self.actors.len() < self.max
            && let Some(key) = self.spawn()
        {
            return Some(key);
        }
        self.actors.iter().filter(|(_, n)| !n.done).min_by_key(|(_, n)| n.mailbox.len()).map(|(k, _)| k)
    }

    // Moves sleepers past their deadline and actors whose host calls answered back to ready.
    fn wake(&mut self, now: u64) {
        let waiting = core::mem::take(&mut self.waiting);
        for key in waiting {
            let Some(actor) = self.actors.get_mut(key) else { continue };
            let awake = match actor.wake_at {
                Some(deadline) => deadline <= now,
                None => actor.poll(),
            };
            if awake {
                actor.wake_at = None;
                actor.runnable = true;
                self.ready.push_back(key);
            } else {
                self.waiting.push(key);
            }
        }
    }
}
