use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvError, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::config::Message;

// Bytes one message may carry, an ingress line or a control body alike.
pub const MAX_MESSAGE: usize = 16 << 20;
// Messages sent and not yet taken by the scheduler, past it the inbox refuses more.
const INBOX: usize = 1024;

// Live actor counters, the scheduler writes them and the control endpoint reads them.
#[derive(Default)]
pub struct Stats {
    actors: AtomicUsize,
    active: AtomicUsize,
    idle: AtomicUsize,
    pending: AtomicUsize,
    crashes: AtomicUsize,
    dead: AtomicUsize,
}

impl Stats {
    pub fn set(&self, actors: usize, active: usize, idle: usize, pending: usize, crashes: usize) {
        self.actors.store(actors, Ordering::Relaxed);
        self.active.store(active, Ordering::Relaxed);
        self.idle.store(idle, Ordering::Relaxed);
        self.pending.store(pending, Ordering::Relaxed);
        self.crashes.store(crashes, Ordering::Relaxed);
    }

    // A message that exhausted its retries and was dropped.
    pub fn add_dead(&self) {
        self.dead.fetch_add(1, Ordering::Relaxed);
    }

    // Renders the counters as a flat JSON object for the /stats route.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"actors\":{},\"active\":{},\"idle\":{},\"pending\":{},\"crashes\":{},\"dead\":{}}}",
            self.actors.load(Ordering::Relaxed),
            self.active.load(Ordering::Relaxed),
            self.idle.load(Ordering::Relaxed),
            self.pending.load(Ordering::Relaxed),
            self.crashes.load(Ordering::Relaxed),
            self.dead.load(Ordering::Relaxed),
        )
    }
}

// A durable append-only log of pending messages, replayed on restart so nothing is lost.
pub struct Wal {
    path: PathBuf,
    file: std::fs::File,
}

impl Wal {
    // Opens the log, returning it plus any messages a previous run left unprocessed.
    pub fn open(path: &Path) -> std::io::Result<(Self, Vec<Message>)> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut recovered = Vec::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some(m) = decode(line) {
                    recovered.push(m);
                }
            }
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        Ok((Wal { path: path.to_path_buf(), file }, recovered))
    }

    // Appends one message and flushes so a clean process restart keeps it.
    pub fn append(&mut self, msg: &Message) {
        let _ = writeln!(self.file, "{}", encode(msg));
        let _ = self.file.flush();
    }

    // Rewrites the log with only what is still pending, an atomic rename swaps it in.
    pub fn compact(&mut self, pending: &[Message]) {
        let tmp = self.path.with_extension("wal.tmp");
        if let Ok(mut w) = std::fs::File::create(&tmp) {
            for m in pending {
                let _ = writeln!(w, "{}", encode(m));
            }
            let _ = w.flush();
            if std::fs::rename(&tmp, &self.path).is_ok()
                && let Ok(f) = std::fs::OpenOptions::new().append(true).open(&self.path) {
                self.file = f;
            }
        }
    }
}

// One record, group and body tab-separated with control chars escaped.
fn encode(m: &Message) -> String {
    format!("{}\t{}", esc(&m.group), esc(&m.body))
}

fn decode(line: &str) -> Option<Message> {
    let (g, b) = line.split_once('\t')?;
    Some(Message { group: unesc(g), body: unesc(b), attempts: 0, reply: None })
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/* The way into a live pool, each message logged once the pool has room for it. */
#[derive(Clone)]
pub struct Inbox {
    tx: Sender<Message>,
    wal: Arc<Mutex<Wal>>,
    // Sent and not yet taken, the count that holds a flood back.
    transit: Arc<AtomicUsize>,
}

/* Why the inbox turned a message away, a full one hands it back. */
pub enum Refused {
    Full(Message),
    Down,
}

/* What the scheduler drains the inbox through, each take making room for one more. */
pub struct Intake {
    rx: Receiver<Message>,
    transit: Arc<AtomicUsize>,
}

/* An inbox and its intake, over the log that records what comes in. */
pub fn inbox(wal: Arc<Mutex<Wal>>) -> (Inbox, Intake) {
    let (tx, rx) = channel();
    let transit = Arc::new(AtomicUsize::new(0));
    (Inbox { tx, wal, transit: transit.clone() }, Intake { rx, transit })
}

impl Inbox {
    /* Logs and queues a message, or hands it back while INBOX are already in transit. */
    pub fn admit(&self, msg: Message) -> Result<(), Refused> {
        if self.transit.fetch_add(1, Ordering::Relaxed) >= INBOX {
            self.transit.fetch_sub(1, Ordering::Relaxed);
            return Err(Refused::Full(msg));
        }
        self.wal.lock().unwrap().append(&msg);
        self.tx.send(msg).map_err(|_| Refused::Down)
    }
}

impl Intake {
    pub fn try_recv(&self) -> Option<Message> {
        self.rx.try_recv().ok().inspect(|_| self.took())
    }

    pub fn recv_timeout(&self, wait: Duration) -> Result<Message, RecvTimeoutError> {
        self.rx.recv_timeout(wait).inspect(|_| self.took())
    }

    pub fn recv(&self) -> Result<Message, RecvError> {
        self.rx.recv().inspect(|_| self.took())
    }

    fn took(&self) {
        self.transit.fetch_sub(1, Ordering::Relaxed);
    }
}
