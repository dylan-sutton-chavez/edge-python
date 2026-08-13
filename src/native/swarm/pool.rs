use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};

use super::config::{Message, SwarmConfig};
use super::scheduler::Scheduler;

// Shared termination state, the swarm ends when every shard is idle with nothing in flight.
pub struct Barrier {
    // Messages routed to a shard but not yet consumed.
    inflight: AtomicUsize,
    total: usize,
    idle: Mutex<Quiescence>,
    changed: Condvar,
}

struct Quiescence {
    idle: usize,
    shutdown: bool,
}

// Routes a cross-thread message to the shard owning its group.
#[derive(Clone)]
pub struct Router {
    // One sender per shard, indexed the same as the scheduler threads.
    shards: Vec<Sender<Message>>,
    // Which shard owns each group, so a send reaches the thread running it.
    group_shard: HashMap<String, usize>,
    barrier: Arc<Barrier>,
}

impl Router {
    // Delivers a message to the shard that owns its target group, counting it in flight.
    pub fn route(&self, msg: Message) {
        if let Some(&shard) = self.group_shard.get(&msg.group) {
            self.barrier.inflight.fetch_add(1, Ordering::Release);
            if self.shards[shard].send(msg).is_err() {
                self.barrier.inflight.fetch_sub(1, Ordering::Release);
            }
            // Wake an idle shard so it re-checks its channel.
            self.barrier.changed.notify_all();
        }
    }

    pub fn barrier(&self) -> Arc<Barrier> {
        self.barrier.clone()
    }
}

impl Barrier {
    fn new(total: usize) -> Self {
        Barrier {
            inflight: AtomicUsize::new(0),
            total,
            idle: Mutex::new(Quiescence { idle: 0, shutdown: false }),
            changed: Condvar::new(),
        }
    }

    // A message was pulled off a channel, it no longer counts as in flight.
    pub fn consume(&self) {
        self.inflight.fetch_sub(1, Ordering::Release);
    }

    /* Blocks an idle shard until work arrives or the swarm quiesces, true means shut down, the last shard to idle with an empty in-flight count flips shutdown and wakes everyone. */
    pub fn park_until_work_or_done(&self) -> bool {
        let mut q = self.idle.lock().unwrap();
        q.idle += 1;
        if q.idle == self.total && self.inflight.load(Ordering::Acquire) == 0 {
            q.shutdown = true;
            self.changed.notify_all();
        }
        while !q.shutdown && self.inflight.load(Ordering::Acquire) == 0 {
            q = self.changed.wait(q).unwrap();
        }
        if q.shutdown {
            return true;
        }
        q.idle -= 1;
        false
    }
}

// Runs the swarm across `threads` schedulers, groups sharded round robin over threads.
pub fn run(config: SwarmConfig, threads: usize) -> i32 {
    let threads = threads.max(1);
    // One thread keeps the simple in-process path, no channels or router needed.
    if threads == 1 {
        return match Scheduler::new(config) {
            Ok(mut s) => s.run(),
            Err(e) => { eprintln!("error: {e}"); 1 }
        };
    }

    // Assign each group to a shard, grouping the config accordingly.
    let mut group_shard = HashMap::new();
    let mut shard_groups: Vec<Vec<_>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, g) in config.groups.into_iter().enumerate() {
        let shard = i % threads;
        group_shard.insert(g.name.clone(), shard);
        shard_groups[shard].push(g);
    }

    let mut senders = Vec::with_capacity(threads);
    let mut receivers = Vec::with_capacity(threads);
    for _ in 0..threads {
        let (tx, rx) = channel::<Message>();
        senders.push(tx);
        receivers.push(rx);
    }
    let barrier = Arc::new(Barrier::new(threads));
    let router = Router { shards: senders, group_shard, barrier };

    // Each shard runs its own scheduler on its own thread, cross-group sends cross by channel.
    let mut handles = Vec::new();
    for (groups, rx) in shard_groups.into_iter().zip(receivers) {
        let router = router.clone();
        let shard_config = SwarmConfig { groups, max_nodes: config.max_nodes };
        handles.push(std::thread::spawn(move || match Scheduler::new(shard_config) {
            Ok(mut s) => s.run_sharded(router, rx),
            Err(e) => { eprintln!("error: {e}"); 1 }
        }));
    }

    handles.into_iter().map(|h| h.join().unwrap_or(1)).max().unwrap_or(0)
}
