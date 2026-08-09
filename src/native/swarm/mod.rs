mod config;
mod node;
mod pool;
mod scheduler;
mod server;

pub use config::{Group, Message, Out, SwarmConfig};
pub use server::Stats;

use std::path::Path;
use std::sync::{Arc, Mutex};

// Runs a swarm to quiescence across `threads` schedulers, one per core when auto-sized.
pub fn run(config: SwarmConfig, threads: usize) -> i32 {
    pool::run(config, threads)
}

/* Runs the swarm as a live server, an ingress on `addr` feeds it and the wal at `wal_path`
   survives restarts by replaying unprocessed messages. `stats` publishes live counts when set.
   `on_ingress` receives the queue sender so the caller can feed messages of its own, the cli
   uses it for the control endpoint that answers eval runs. */
pub fn serve(config: SwarmConfig, addr: &str, wal_path: &Path, stats: Option<Arc<Stats>>, on_ingress: impl FnOnce(std::sync::mpsc::Sender<Message>)) -> i32 {
    let (wal, recovered) = match server::Wal::open(wal_path) {
        Ok(pair) => pair,
        Err(e) => { eprintln!("error: cannot open wal '{}': {e}", wal_path.display()); return 1; }
    };
    let mut scheduler = match Scheduler::new(config) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    scheduler.set_stats(stats);
    let (tx, rx) = std::sync::mpsc::channel();
    // Recovered messages re-enter the queue before the ingress opens.
    for m in recovered {
        let _ = tx.send(m);
    }
    on_ingress(tx.clone());
    let wal = Arc::new(Mutex::new(wal));
    if let Err(e) = server::spawn_ingress(addr, tx, wal.clone()) {
        eprintln!("error: cannot bind ingress '{addr}': {e}");
        return 1;
    }
    scheduler.run_serving(rx, wal)
}

use scheduler::Scheduler;
