use std::collections::HashMap;

use crate::parser::SSAChunk;
use crate::vm::Limits;

use super::config::{Group, Message, Out, SwarmConfig};
use super::node::{Node, Step};
use super::pool::Router;

// A parsed group ready to boot nodes from, chunk shared across every replica.
struct GroupState {
    source: String,
    // Fixed groups share one parsed chunk, eval groups compile each message so it is None.
    chunk: Option<&'static SSAChunk>,
    dir: String,
    retry: usize,
    // Node ceiling, nodes spawn lazily up to this instead of all at boot.
    max: usize,
    limits: Limits,
    preempt: usize,
    out: Out,
    nodes: Vec<Node>,
}

// The single-threaded cooperative loop, one instance owns every node in the swarm.
pub struct Scheduler {
    groups: Vec<GroupState>,
    by_name: HashMap<String, usize>,
    pending: Vec<Message>,
    // Tracebacks of nodes that raised uncaught and were retired.
    crashes: Vec<String>,
    // Set in sharded mode, routes sends whose group lives on another thread.
    router: Option<Router>,
    // Live counters published for the control endpoint, None when no control port is set.
    stats: Option<std::sync::Arc<super::server::Stats>>,
}

impl Scheduler {
    pub fn new(config: SwarmConfig) -> Result<Self, String> {
        let mut groups = Vec::new();
        let mut by_name = HashMap::new();
        let mut pending = Vec::new();
        for g in config.groups {
            by_name.insert(g.name.clone(), groups.len());
            pending.extend(g.inbox.iter().map(|m| Message { group: m.group.clone(), body: m.body.clone(), attempts: 0 }));
            groups.push(GroupState::boot(g, config.max_nodes)?);
        }
        Ok(Scheduler { groups, by_name, pending, crashes: Vec::new(), router: None, stats: None })
    }

    // Wires the shared counters the control endpoint reads, published each tick.
    pub fn set_stats(&mut self, stats: Option<std::sync::Arc<super::server::Stats>>) {
        self.stats = stats;
    }

    // Pumps messages and runs nodes until nothing is left to deliver or run.
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

    // Boots one node per group so producers run, the pool then grows lazily on demand.
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
            // No local work, block until another shard sends some or the whole swarm quiesces.
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
        let mut nodes = 0;
        let mut idle = 0;
        for g in &self.groups {
            for n in &g.nodes {
                nodes += 1;
                if n.idle {
                    idle += 1;
                }
            }
        }
        stats.set(nodes, nodes - idle, idle, self.pending.len(), self.crashes.len());
    }

    // Delivers each queued message to a node of its target group, spawning on demand.
    fn route_pending(&mut self) {
        let msgs = core::mem::take(&mut self.pending);
        for m in msgs {
            let Some(&gi) = self.by_name.get(&m.group) else { continue };
            if let Some(ni) = self.groups[gi].pick() {
                self.groups[gi].nodes[ni].deliver(m);
            }
        }
    }

    // Runs every runnable node once, collecting what they send, false when none progressed.
    fn tick(&mut self) -> bool {
        let mut progressed = false;
        for gi in 0..self.groups.len() {
            for ni in 0..self.groups[gi].nodes.len() {
                let node = &self.groups[gi].nodes[ni];
                // Only run a node with work, fresh boot or delivered mail.
                if node.done || (node.mailbox.is_empty() && node.ran) {
                    continue;
                }
                progressed = true;
                let src = self.groups[gi].source.clone();
                let step = self.groups[gi].nodes[ni].step(&src);
                self.collect_sends();
                if let Step::Failed(tb, msg) = step {
                    self.crashes.push(tb);
                    self.handle_crash(gi, msg);
                }
            }
            self.groups[gi].nodes.retain(|n| !n.done);
        }
        self.publish_stats();
        progressed
    }

    // Retries a crashed message on another node up to the group's retry count, else drops it dead.
    fn handle_crash(&mut self, gi: usize, msg: Option<Message>) {
        let Some(mut msg) = msg else { return };
        if msg.attempts < self.groups[gi].retry {
            msg.attempts += 1;
            self.pending.push(msg);
        } else if let Some(stats) = &self.stats {
            stats.add_dead();
        }
    }

    // Drains what the nodes just sent, routing cross-shard groups out and keeping local ones.
    fn collect_sends(&mut self) {
        for out in crate::native::builtins::swarm::drain_outbox() {
            let msg = Message { group: out.group, body: out.body, attempts: 0 };
            match &self.router {
                Some(r) if !self.by_name.contains_key(&msg.group) => r.route(msg),
                _ => self.pending.push(msg),
            }
        }
    }
}

impl GroupState {
    fn boot(g: Group, max_nodes: usize) -> Result<Self, String> {
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
            max: g.replicas.min(max_nodes),
            limits: g.limits,
            preempt: g.preempt,
            out: g.out,
            nodes: Vec::new(),
        })
    }

    // Boots a fresh node, fixed nodes share the group chunk, eval nodes start empty and untrusted.
    fn spawn(&mut self) -> usize {
        let node = match self.chunk {
            Some(chunk) => {
                let mut vm = crate::vm::VM::with_limits(chunk, self.limits);
                vm.strict_input = true;
                vm.set_time_hook(crate::native::now_ns);
                vm.set_preempt_interval(self.preempt);
                wire_output(&mut vm, &self.out);
                Node::fixed(vm)
            }
            None => Node::eval(self.dir.clone(), self.limits, self.preempt),
        };
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /* Picks a node for a message, an idle one first, else a fresh spawn under the ceiling,
       else the least-loaded live node when the group is saturated. */
    fn pick(&mut self) -> Option<usize> {
        if let Some(i) = (0..self.nodes.len()).find(|&i| self.nodes[i].idle && !self.nodes[i].done) {
            return Some(i);
        }
        if self.nodes.len() < self.max {
            return Some(self.spawn());
        }
        (0..self.nodes.len())
            .filter(|&i| !self.nodes[i].done)
            .min_by_key(|&i| self.nodes[i].mailbox.len())
    }
}

// Points a node's print output at stdout or nowhere.
fn wire_output(vm: &mut crate::vm::VM<'static>, out: &Out) {
    match out {
        // A per-file sink needs a closure hook, deferred until print_hook takes one.
        Out::Stdout | Out::File(_) => vm.print_hook = Some(print_stdout),
        Out::Null => vm.print_hook = None,
    }
}

pub(super) fn print_stdout(s: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}
