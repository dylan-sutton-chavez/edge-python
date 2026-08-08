use std::cell::RefCell;
use std::rc::Rc;

use rt::Executor;

use super::{PollPark, Reactor};

// Completed host calls waiting for the driver to inject them, keyed by call id.
pub struct Completions {
    pub done: Vec<(u64, Result<String, String>)>,
    // Streamed events (ws and sse) waiting to be pushed into the VM event queue.
    pub events: Vec<String>,
    // Per-socket mailboxes, ws_send and ws_close post here and the task drains them.
    pub sockets: Vec<Option<SocketBox>>,
}

// A live stream mailbox, the task reads outgoing and the binding writes it.
#[derive(Default)]
pub struct SocketBox {
    // Text frames queued by ws_send waiting for the task to encode them.
    pub outgoing: Vec<String>,
    // Set by ws_close or sse_close, the task closes and ends.
    pub closing: bool,
    // Mirrors the web readyState, 0 connecting 1 open 2 closing 3 closed.
    pub state: i64,
    // The task waker, woken when a binding posts to this mailbox.
    pub waker: Option<std::task::Waker>,
}

impl Completions {
    // Reserves a stream slot, returns its handle for the send close and state bindings.
    pub fn alloc_socket(&mut self) -> usize {
        self.sockets.push(Some(SocketBox { state: 0, ..SocketBox::default() }));
        self.sockets.len() - 1
    }
}

// Sets a stream's readyState mirror when its slot is still live.
pub fn set_state(sink: &Rc<RefCell<Completions>>, handle: usize, state: i64) {
    if let Some(Some(b)) = sink.borrow_mut().sockets.get_mut(handle) {
        b.state = state;
    }
}

// Per-run async runtime, the reactor plus the executor that drives fetch tasks.
pub struct NetRuntime {
    pub reactor: Reactor,
    pub executor: Executor,
    pub completions: Rc<RefCell<Completions>>,
}

thread_local! {
    static NET: RefCell<Option<NetRuntime>> = const { RefCell::new(None) };
}

// Install a runtime for the current run, replacing any previous one.
pub fn install() -> std::io::Result<()> {
    let reactor = Reactor::new()?;
    let executor = Executor::new(Box::new(PollPark::new(reactor.clone())));
    let rt = NetRuntime {
        reactor,
        executor,
        completions: Rc::new(RefCell::new(Completions { done: Vec::new(), events: Vec::new(), sockets: Vec::new() })),
    };
    NET.with(|slot| *slot.borrow_mut() = Some(rt));
    Ok(())
}

// Run f against the installed runtime, None when nothing is installed.
pub fn with<R>(f: impl FnOnce(&NetRuntime) -> R) -> Option<R> {
    NET.with(|slot| slot.borrow().as_ref().map(f))
}

// True while any spawned task has not completed.
pub fn has_pending() -> bool {
    with(|rt| rt.executor.alive() > 0).unwrap_or(false)
}
