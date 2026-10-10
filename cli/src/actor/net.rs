use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token, Waker};
use slab::Slab;

use super::config::{ActorConfig, Message, Reply};
use super::http::{self, Parse};
use super::server::{Inbox, Refused, Stats, MAX_MESSAGE};

// Connections open across the ingress and control, one past it is closed on accept.
const MAX_CONNECTIONS: usize = 10_000;
// How soon a connection held back by a full inbox tries again.
const RETRY: Duration = Duration::from_millis(2);
// How often control connections are checked against their deadline.
const SWEEP: Duration = Duration::from_secs(1);
// Time a control client has to send its request, and later to take its reply.
const DEADLINE: Duration = Duration::from_secs(30);
// Seconds an eval caller waits past the group timeout, room for the runs queued ahead.
const EVAL_QUEUE_WAIT: u64 = 20;
// The listeners and the waker hold the top tokens, connections take their slab keys.
const INGRESS: Token = Token(usize::MAX);
const CONTROL: Token = Token(usize::MAX - 1);
const WAKE: Token = Token(usize::MAX - 2);

/* What the control port answers, the live counters, its groups, and the eval ones with their timeout. */
pub struct Routes {
    pub stats: Arc<Stats>,
    groups: Vec<String>,
    eval: Vec<(String, u64)>,
}

impl Routes {
    pub fn of(config: &ActorConfig) -> Self {
        let groups = config.groups.iter().map(|g| g.name.clone()).collect();
        let eval = config.groups.iter().filter(|g| g.eval).map(|g| (g.name.clone(), g.timeout)).collect();
        Routes { stats: Arc::new(Stats::default()), groups, eval }
    }
}

/* Binds the ingress and the control port and serves both from one thread. */
pub fn spawn(listen: &str, control: Option<(&str, Routes)>, inbox: Inbox) -> std::io::Result<()> {
    let poll = Poll::new()?;
    let ingress = bind(poll.registry(), listen, INGRESS)?;
    // A control port that cannot bind leaves the ingress serving.
    let control = control.and_then(|(addr, routes)| match bind(poll.registry(), addr, CONTROL) {
        Ok(listener) => Some((listener, routes)),
        Err(_) => {
            eprintln!("warning: cannot bind control endpoint '{addr}'");
            None
        }
    });
    let waker = Arc::new(Waker::new(poll.registry(), WAKE)?);
    let (to, answers) = channel();
    let mut net = Net { poll, ingress, control, conns: Slab::new(), stalled: Vec::new(), inbox, to, waker, answers, waiting: HashMap::new(), next: 0, swept: Instant::now() };
    std::thread::spawn(move || net.run());
    Ok(())
}

fn bind(registry: &Registry, addr: &str, token: Token) -> std::io::Result<TcpListener> {
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let mut listener = TcpListener::from_std(listener);
    registry.register(&mut listener, token, Interest::READABLE)?;
    Ok(listener)
}

/* Every connection of the server on one thread, the OS waking it only for sockets with work. */
struct Net {
    poll: Poll,
    ingress: TcpListener,
    control: Option<(TcpListener, Routes)>,
    conns: Slab<Conn>,
    // Ingress connections holding a message the inbox had no room for.
    stalled: Vec<usize>,
    inbox: Inbox,
    // Where eval runs answer and what wakes the loop for it.
    to: Sender<(u64, Result<String, String>)>,
    waker: Arc<Waker>,
    answers: Receiver<(u64, Result<String, String>)>,
    // The connection each eval caller waits on, by reply id.
    waiting: HashMap<u64, usize>,
    next: u64,
    swept: Instant,
}

enum Conn {
    Line(Line),
    Http(Http),
}

/* An ingress client, each line a message, holding the one the inbox handed back. */
struct Line {
    stream: TcpStream,
    buf: Vec<u8>,
    // Where the next line starts, and how far past it no newline turned up.
    head: usize,
    seen: usize,
    held: Option<Message>,
    eof: bool,
    // Set while it waits in the stalled list, so it never sits there twice.
    stalled: bool,
}

/* A control client, its request read whole, then its reply written before the close. */
struct Http {
    stream: TcpStream,
    buf: Vec<u8>,
    state: State,
    // When the connection is cut, or its eval caller hears the wait ran out.
    until: Instant,
}

enum State {
    // The request still arriving, true once a 100 Continue went out.
    Reading(bool),
    // Parked under a reply id until its eval run answers.
    Waiting(u64),
    // The reply and how much of it the socket took.
    Writing(Vec<u8>, usize),
}

/* What a pass over a connection left it as. */
enum Flow {
    Open,
    Stalled,
    Closed,
    Down,
}

impl Net {
    fn run(&mut self) {
        let mut events = Events::with_capacity(1024);
        loop {
            let wait = if self.stalled.is_empty() { self.control.as_ref().map(|_| SWEEP) } else { Some(RETRY) };
            if let Err(e) = self.poll.poll(&mut events, wait) {
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            for event in &events {
                let alive = match event.token() {
                    INGRESS | CONTROL => {
                        self.accept(event.token());
                        true
                    }
                    WAKE => {
                        self.answer();
                        true
                    }
                    Token(key) => self.pump(key),
                };
                if !alive {
                    return;
                }
            }
            for key in std::mem::take(&mut self.stalled) {
                if let Some(Conn::Line(line)) = self.conns.get_mut(key)
                    && line.stalled
                {
                    line.stalled = false;
                    if !self.pump(key) {
                        return;
                    }
                }
            }
            if self.swept.elapsed() >= SWEEP {
                self.sweep();
            }
        }
    }

    /* Takes every waiting connection on a listener, closing those past the cap. */
    fn accept(&mut self, token: Token) {
        loop {
            let listener = match (token, &self.control) {
                (INGRESS, _) => &self.ingress,
                (_, Some((listener, _))) => listener,
                (_, None) => return,
            };
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                // None left, or no descriptor free until a connection closes.
                Err(_) => return,
            };
            if self.conns.len() >= MAX_CONNECTIONS {
                continue;
            }
            let entry = self.conns.vacant_entry();
            let interest = if token == INGRESS { Interest::READABLE } else { Interest::READABLE | Interest::WRITABLE };
            if self.poll.registry().register(&mut stream, Token(entry.key()), interest).is_ok() {
                entry.insert(match token {
                    INGRESS => Conn::Line(Line { stream, buf: Vec::new(), head: 0, seen: 0, held: None, eof: false, stalled: false }),
                    _ => Conn::Http(Http { stream, buf: Vec::new(), state: State::Reading(false), until: Instant::now() + DEADLINE }),
                });
            }
        }
    }

    /* Moves one connection on, false once the pool is gone. */
    fn pump(&mut self, key: usize) -> bool {
        let flow = match self.conns.get_mut(key) {
            None => return true,
            Some(Conn::Line(line)) => {
                let flow = line.pump(&self.inbox);
                if matches!(flow, Flow::Stalled) && !line.stalled {
                    line.stalled = true;
                    self.stalled.push(key);
                }
                flow
            }
            Some(Conn::Http(_)) => self.pump_http(key),
        };
        match flow {
            Flow::Down => return false,
            Flow::Closed => self.close(key),
            Flow::Open | Flow::Stalled => {}
        }
        true
    }

    /* Reads a control request, routes it once whole, and writes its reply. */
    fn pump_http(&mut self, key: usize) -> Flow {
        let Some(Conn::Http(conn)) = self.conns.get_mut(key) else { return Flow::Open };
        let State::Reading(continued) = conn.state else { return conn.flush() };
        let closed = conn.fill();
        let reply = match http::parse(&conn.buf) {
            Parse::Partial(_) if closed => return Flow::Closed,
            Parse::Partial(expects) => {
                if expects && !continued {
                    let _ = conn.stream.write(http::CONTINUE);
                    conn.state = State::Reading(true);
                }
                return Flow::Open;
            }
            Parse::Bad(status, why) => http::text(status, why),
            Parse::Done(request) => match self.route(key, request) {
                Some(reply) => reply,
                None => return Flow::Open,
            },
        };
        self.reply(key, reply);
        Flow::Open
    }

    /* Answers a control request at once, or parks it until its eval run ends. */
    fn route(&mut self, key: usize, request: http::Request) -> Option<Vec<u8>> {
        let routes = &self.control.as_ref()?.1;
        if request.path == "/stats" {
            return Some(http::json(200, &routes.stats.to_json()));
        }
        if request.post
            && let Some(group) = request.path.strip_prefix("/pub/")
            && routes.groups.iter().any(|g| g == group)
        {
            let msg = Message { group: group.to_string(), body: request.body, attempts: 0, reply: None };
            return Some(match self.inbox.admit(msg) {
                Ok(()) => http::json(202, "{\"ok\":true}"),
                Err(refused) => refusal(&refused),
            });
        }
        if request.post
            && let Some(group) = request.path.strip_prefix("/eval/")
            && let Some(&(_, timeout)) = routes.eval.iter().find(|(g, _)| g == group)
        {
            let id = self.next;
            self.next += 1;
            let reply = Reply { id, to: self.to.clone(), wake: self.waker.clone() };
            let msg = Message { group: group.to_string(), body: request.body, attempts: 0, reply: Some(reply) };
            if let Err(refused) = self.inbox.admit(msg) {
                return Some(refusal(&refused));
            }
            self.waiting.insert(id, key);
            if let Some(Conn::Http(conn)) = self.conns.get_mut(key) {
                conn.state = State::Waiting(id);
                conn.until = Instant::now() + Duration::from_secs(timeout + EVAL_QUEUE_WAIT);
            }
            return None;
        }
        Some(http::text(404, "not found"))
    }

    /* Hands each finished eval run to the caller parked on it. */
    fn answer(&mut self) {
        while let Ok((id, outcome)) = self.answers.try_recv() {
            let Some(key) = self.waiting.remove(&id) else { continue };
            let reply = match outcome {
                Ok(stdout) => http::json(200, &format!("{{\"ok\":true,\"stdout\":{}}}", http::json_str(&stdout))),
                Err(e) => http::json(500, &format!("{{\"ok\":false,\"error\":{}}}", http::json_str(&e))),
            };
            self.reply(key, reply);
        }
    }

    /* Starts writing a reply, the connection closing once it is out. */
    fn reply(&mut self, key: usize, reply: Vec<u8>) {
        let Some(Conn::Http(conn)) = self.conns.get_mut(key) else { return };
        conn.state = State::Writing(reply, 0);
        conn.until = Instant::now() + DEADLINE;
        if matches!(conn.flush(), Flow::Closed) {
            self.close(key);
        }
    }

    /* Cuts control connections past their deadline, an eval caller hearing that it timed out. */
    fn sweep(&mut self) {
        self.swept = Instant::now();
        let late: Vec<usize> = self.conns.iter().filter(|(_, c)| matches!(c, Conn::Http(h) if h.until <= self.swept)).map(|(k, _)| k).collect();
        for key in late {
            match self.conns[key] {
                Conn::Http(Http { state: State::Waiting(id), .. }) => {
                    self.waiting.remove(&id);
                    self.reply(key, http::text(504, "eval timed out"));
                }
                _ => self.close(key),
            }
        }
    }

    /* Drops a connection, then takes any a full table turned away. */
    fn close(&mut self, key: usize) {
        let mut stream = match self.conns.remove(key) {
            Conn::Line(line) => line.stream,
            Conn::Http(conn) => {
                if let State::Waiting(id) = conn.state {
                    self.waiting.remove(&id);
                }
                conn.stream
            }
        };
        let _ = self.poll.registry().deregister(&mut stream);
        self.accept(INGRESS);
        self.accept(CONTROL);
    }
}

impl Line {
    /* Queues each whole line, reading on until the socket runs dry or the inbox fills. */
    fn pump(&mut self, inbox: &Inbox) -> Flow {
        let mut chunk = [0u8; 64 << 10];
        loop {
            if let Some(msg) = self.held.take() {
                match inbox.admit(msg) {
                    Ok(()) => {}
                    Err(Refused::Full(msg)) => {
                        self.held = Some(msg);
                        return Flow::Stalled;
                    }
                    Err(Refused::Down) => return Flow::Down,
                }
            }
            if let Some(line) = self.line() {
                let Ok(line) = String::from_utf8(line) else { return Flow::Closed };
                if let Some((group, body)) = line.split_once(' ') {
                    self.held = Some(Message { group: group.to_string(), body: body.to_string(), attempts: 0, reply: None });
                }
                continue;
            }
            // Its whole lines queued, a closed socket or a line past the cap ends the connection.
            if self.eof || self.buf.len() - self.head > MAX_MESSAGE {
                return Flow::Closed;
            }
            self.buf.drain(..self.head);
            self.seen -= self.head;
            self.head = 0;
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.eof = true;
                    if !self.buf.is_empty() {
                        self.buf.push(b'\n');
                    }
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Flow::Open,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Flow::Closed,
            }
        }
    }

    /* The next whole line without its line ending, None until its newline arrives. */
    fn line(&mut self) -> Option<Vec<u8>> {
        let from = self.seen.max(self.head);
        let Some(at) = self.buf[from..].iter().position(|&b| b == b'\n') else {
            self.seen = self.buf.len();
            return None;
        };
        let line = &self.buf[self.head..from + at];
        let line = line.strip_suffix(b"\r").unwrap_or(line).to_vec();
        self.head = from + at + 1;
        self.seen = self.head;
        Some(line)
    }
}

impl Http {
    /* Reads what the socket holds, true once the client closed its side. */
    fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 64 << 10];
        // A full request never needs more, past it the parse answers or refuses.
        while self.buf.len() <= http::HEAD_MAX + MAX_MESSAGE {
            match self.stream.read(&mut chunk) {
                Ok(0) => return true,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return e.kind() != ErrorKind::WouldBlock,
            }
        }
        false
    }

    /* Writes what the socket takes of the reply, Closed once it is all out. */
    fn flush(&mut self) -> Flow {
        let State::Writing(reply, sent) = &mut self.state else { return Flow::Open };
        while *sent < reply.len() {
            match self.stream.write(&reply[*sent..]) {
                Ok(0) => return Flow::Closed,
                Ok(n) => *sent += n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Flow::Open,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Flow::Closed,
            }
        }
        Flow::Closed
    }
}

// A message the inbox turned away, 503 either way so the client tries again.
fn refusal(refused: &Refused) -> Vec<u8> {
    let why = match refused {
        Refused::Full(_) => "queue is full",
        Refused::Down => "actor is down",
    };
    http::text(503, why)
}
