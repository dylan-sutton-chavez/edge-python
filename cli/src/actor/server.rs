use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvError, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::config::Message;

// Bytes one message may carry, an ingress line or a control body alike.
pub const MAX_MESSAGE: usize = 16 << 20;
// Messages sent and not yet taken by the scheduler, past it the inbox refuses more.
const INBOX: usize = 1024;
// Bytes a log may reach before done records get rewritten away.
const COMPACT_AT: u64 = 64 << 20;

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

/* The messages clients sent, each marked done once an actor finishes it, so a restart replays only the rest. */
pub struct Wal {
    path: PathBuf,
    file: File,
    next: u64,
    // The record size of each message not yet done, what a compaction keeps.
    open: HashMap<u64, u64>,
    live: u64,
    size: u64,
    // A message logged since the last fsync.
    dirty: bool,
}

impl Wal {
    /* Opens the log, returning it plus the messages a previous run left unfinished, rewritten as the new log. */
    pub fn open(path: &Path) -> std::io::Result<(Self, Vec<Message>)> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let (mut left, mut next) = (BTreeMap::new(), 0);
        if let Ok(file) = File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                match line.split('\t').collect::<Vec<_>>()[..] {
                    [id] if let Ok(id) = id.parse::<u64>() => {
                        left.remove(&id);
                    }
                    [id, group, body] if let Ok(id) = id.parse::<u64>() => {
                        next = next.max(id + 1);
                        left.insert(id, Message { id: Some(id), ..Message::new(unesc(group), unesc(body)) });
                    }
                    _ => {}
                }
            }
        }
        let recovered: Vec<Message> = left.into_values().collect();
        let tmp = path.with_extension("wal.tmp");
        let mut out = BufWriter::new(File::create(&tmp)?);
        let mut open = HashMap::new();
        for m in &recovered {
            let id = m.id.unwrap_or_default();
            let record = record(id, m);
            out.write_all(record.as_bytes())?;
            open.insert(id, record.len() as u64);
        }
        let file = install(out, &tmp, path)?;
        let size = open.values().sum();
        Ok((Wal { path: path.to_path_buf(), file, next, open, live: size, size, dirty: false }, recovered))
    }

    /* Logs a message under the next id, written at once so a process crash keeps it. */
    pub fn append(&mut self, msg: &mut Message) {
        let id = self.next;
        self.next += 1;
        msg.id = Some(id);
        let record = record(id, msg);
        self.write(record.as_bytes());
        self.open.insert(id, record.len() as u64);
        self.live += record.len() as u64;
        self.dirty = true;
    }

    /* Marks a message done, rewriting the log once done records make up most of it. */
    pub fn ack(&mut self, id: u64) {
        let Some(len) = self.open.remove(&id) else { return };
        self.live -= len;
        self.write(format!("{id}\n").as_bytes());
        if self.size >= COMPACT_AT.max(2 * self.live) {
            self.compact();
        }
    }

    /* Forces what was written onto the disk, so a power cut keeps what a client heard was taken. */
    pub fn sync(&mut self) {
        if std::mem::take(&mut self.dirty) {
            let _ = self.file.sync_data();
        }
    }

    // A done record needs no fsync, losing one only replays a message twice.
    fn write(&mut self, bytes: &[u8]) {
        if self.file.write_all(bytes).is_ok() {
            self.size += bytes.len() as u64;
        }
    }

    /* Rewrites the log with only the records of messages not yet done. */
    fn compact(&mut self) {
        let tmp = self.path.with_extension("wal.tmp");
        let kept = (|| {
            let mut out = BufWriter::new(File::create(&tmp)?);
            for line in BufReader::new(File::open(&self.path)?).lines() {
                let line = line?;
                if let Some((id, _)) = line.split_once('\t')
                    && id.parse().is_ok_and(|id| self.open.contains_key(&id))
                {
                    out.write_all(line.as_bytes())?;
                    out.write_all(b"\n")?;
                }
            }
            install(out, &tmp, &self.path)
        })();
        if let Ok(file) = kept {
            self.file = file;
            self.size = self.live;
        }
    }
}

// One message record, id, group and body tab-separated with control chars escaped.
fn record(id: u64, m: &Message) -> String {
    format!("{id}\t{}\t{}\n", esc(&m.group), esc(&m.body))
}

/* Syncs a rewritten log and swaps it in by rename, the directory synced so the swap survives a crash. */
fn install(out: BufWriter<File>, tmp: &Path, path: &Path) -> std::io::Result<File> {
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    std::fs::rename(tmp, path)?;
    #[cfg(unix)]
    {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let _ = File::open(dir).and_then(|d| d.sync_all());
    }
    OpenOptions::new().append(true).open(path)
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
    pub fn admit(&self, mut msg: Message) -> Result<(), Refused> {
        if self.transit.fetch_add(1, Ordering::Relaxed) >= INBOX {
            self.transit.fetch_sub(1, Ordering::Relaxed);
            return Err(Refused::Full(msg));
        }
        // A caller waiting on its reply sends again when it fails, so only fire and forget messages are logged.
        if msg.reply.is_none() {
            self.wal.lock().unwrap().append(&mut msg);
        }
        self.tx.send(msg).map_err(|_| Refused::Down)
    }

    pub fn sync(&self) {
        self.wal.lock().unwrap().sync();
    }
}

impl Intake {
    pub fn try_recv(&self) -> Result<Message, TryRecvError> {
        self.rx.try_recv().inspect(|_| self.took())
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
