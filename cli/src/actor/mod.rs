#[allow(clippy::module_inception)]
mod actor;
mod config;
mod http;
mod net;
mod pool;
mod scheduler;
mod server;

pub use config::{ActorConfig, Group, Message, Out};

use std::path::Path;
use std::sync::{Arc, Mutex};

// Runs a pool to quiescence across `threads` schedulers, one per core when auto-sized.
pub fn run(config: ActorConfig, threads: usize) -> i32 {
    pool::run(config, threads)
}

// Runs the pool as a live server, its ingress on `listen` and its HTTP control port on `control`.
pub fn serve(config: ActorConfig, listen: &str, control: Option<&str>, wal_path: &Path) -> i32 {
    let (wal, recovered) = match server::Wal::open(wal_path) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: cannot open wal '{}': {e}", wal_path.display());
            return 1;
        }
    };
    let control = control.map(|addr| (addr, net::Routes::of(&config)));
    let runtime = match pool::runtime(&config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let mut scheduler = match Scheduler::new(config, runtime) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    scheduler.set_stats(control.as_ref().map(|(_, routes)| routes.stats.clone()));
    let wal = Arc::new(Mutex::new(wal));
    let (inbox, intake) = server::inbox(wal.clone());
    if let Err(e) = net::spawn(listen, control, inbox) {
        eprintln!("error: cannot bind ingress '{listen}': {e}");
        return 1;
    }
    // Recovered messages are in the log already, so they go straight to the scheduler.
    scheduler.run_serving(recovered, intake, wal)
}

use scheduler::Scheduler;
