mod config;
mod node;
mod pool;
mod scheduler;
mod server;

pub use config::{Group, Message, Out, SwarmConfig};
pub use server::{Stats, Wal};

use std::path::Path;
use std::sync::{Arc, Mutex};

// Runs a swarm to quiescence across `threads` schedulers, one per core when auto-sized.
pub fn run(config: SwarmConfig, threads: usize) -> i32 {
    pool::run(config, threads)
}

// Runs the swarm as a live server, `on_ingress` receives the queue sender and wal for the caller.
pub fn serve(config: SwarmConfig, addr: &str, wal_path: &Path, stats: Option<Arc<Stats>>, on_ingress: impl FnOnce(std::sync::mpsc::Sender<Message>, Arc<Mutex<Wal>>)) -> i32 {
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
    let wal = Arc::new(Mutex::new(wal));
    on_ingress(tx.clone(), wal.clone());
    if let Err(e) = server::spawn_ingress(addr, tx, wal.clone()) {
        eprintln!("error: cannot bind ingress '{addr}': {e}");
        return 1;
    }
    scheduler.run_serving(rx, wal)
}

use scheduler::Scheduler;
