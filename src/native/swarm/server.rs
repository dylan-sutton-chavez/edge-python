use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use super::config::Message;

// Live swarm counters, the scheduler writes them and the control endpoint reads them.
#[derive(Default)]
pub struct Stats {
    nodes: AtomicUsize,
    active: AtomicUsize,
    idle: AtomicUsize,
    pending: AtomicUsize,
    crashes: AtomicUsize,
    dead: AtomicUsize,
}

impl Stats {
    pub fn set(&self, nodes: usize, active: usize, idle: usize, pending: usize, crashes: usize) {
        self.nodes.store(nodes, Ordering::Relaxed);
        self.active.store(active, Ordering::Relaxed);
        self.idle.store(idle, Ordering::Relaxed);
        self.pending.store(pending, Ordering::Relaxed);
        self.crashes.store(crashes, Ordering::Relaxed);
    }

    // A message that exhausted its retries and was dropped.
    pub fn add_dead(&self) {
        self.dead.fetch_add(1, Ordering::Relaxed);
    }

    // Renders the counters as a flat JSON object for the /status route.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"nodes\":{},\"active\":{},\"idle\":{},\"pending\":{},\"crashes\":{},\"dead\":{}}}",
            self.nodes.load(Ordering::Relaxed),
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
    Some(Message { group: unesc(g), body: unesc(b), attempts: 0 })
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

// Listens on addr, each `<group> <body>` line becomes a message persisted then queued.
pub fn spawn_ingress(addr: &str, tx: Sender<Message>, wal: Arc<Mutex<Wal>>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                let Some((group, body)) = line.split_once(' ') else { continue };
                let msg = Message { group: group.to_string(), body: body.to_string(), attempts: 0 };
                wal.lock().unwrap().append(&msg);
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    });
    Ok(())
}
