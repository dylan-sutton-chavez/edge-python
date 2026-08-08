mod connect;
mod http;
mod net;
mod park;
pub mod runtime;
mod sse;
mod stream;
mod tls;
mod ws;

pub use http::fetch;
pub use net::Async;
pub use park::PollPark;
pub use runtime::{has_pending, install, with};
pub use sse::sse;
pub use stream::Stream;
pub use tls::{connect_tcp, TlsStream};
pub use ws::ws;

use std::cell::RefCell;
use std::rc::Rc;
use std::task::Waker;

use polling::{Event, Events, PollMode, Poller};
use slab::Slab;

// Shared reactor, single-threaded like the VM so an Rc plus RefCell suffices.
#[derive(Clone)]
pub struct Reactor(Rc<Inner>);

struct Inner {
    poller: Poller,
    // Registered sources keyed by poll key, holding the waker to fire on readiness.
    sources: RefCell<Slab<Registration>>,
    events: RefCell<Events>,
}

struct Registration {
    read: Option<Waker>,
    write: Option<Waker>,
}

impl Reactor {
    pub fn new() -> std::io::Result<Self> {
        Ok(Reactor(Rc::new(Inner {
            poller: Poller::new()?,
            sources: RefCell::new(Slab::new()),
            events: RefCell::new(Events::new()),
        })))
    }

    // Register a raw source in level mode so readiness reports until the data is drained.
    pub fn register(&self, source: std::os::fd::RawFd) -> std::io::Result<usize> {
        let key = self.0.sources.borrow_mut().insert(Registration { read: None, write: None });
        unsafe { self.0.poller.add_with_mode(source, Event::none(key), PollMode::Level)? };
        Ok(key)
    }

    // Arm read interest and stash the caller's waker, keeping any pending write interest.
    pub fn poll_read(&self, key: usize, source: std::os::fd::RawFd, waker: &Waker) -> std::io::Result<()> {
        self.0.sources.borrow_mut()[key].read = Some(waker.clone());
        self.rearm(key, source)
    }

    pub fn poll_write(&self, key: usize, source: std::os::fd::RawFd, waker: &Waker) -> std::io::Result<()> {
        self.0.sources.borrow_mut()[key].write = Some(waker.clone());
        self.rearm(key, source)
    }

    // Re-declare combined interest, level mode reports readiness until drained.
    fn rearm(&self, key: usize, source: std::os::fd::RawFd) -> std::io::Result<()> {
        let sources = self.0.sources.borrow();
        let reg = &sources[key];
        let mut event = Event::none(key);
        event.readable = reg.read.is_some();
        event.writable = reg.write.is_some();
        self.0.poller.modify_with_mode(unsafe { as_source(source) }, event, PollMode::Level)
    }

    pub fn deregister(&self, key: usize, source: std::os::fd::RawFd) {
        let _ = self.0.poller.delete(unsafe { as_source(source) });
        self.0.sources.borrow_mut().try_remove(key);
    }

    // One reactor tick, blocks until an event or notify, then wakes the ready sources.
    pub fn tick(&self, timeout: Option<std::time::Duration>) {
        let mut events = self.0.events.borrow_mut();
        events.clear();
        if self.0.poller.wait(&mut events, timeout).is_err() {
            return;
        }
        let mut sources = self.0.sources.borrow_mut();
        for ev in events.iter() {
            if let Some(reg) = sources.get_mut(ev.key) {
                if ev.readable && let Some(w) = reg.read.take() {
                    w.wake();
                }
                if ev.writable && let Some(w) = reg.write.take() {
                    w.wake();
                }
            }
        }
    }

    // Interrupt a tick blocked in wait, safe from any context.
    pub fn notify(&self) {
        let _ = self.0.poller.notify();
    }
}

// polling wants an AsFd, wrap a raw fd borrowed for the call.
unsafe fn as_source(fd: std::os::fd::RawFd) -> std::os::fd::BorrowedFd<'static> {
    unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll};

    // Waits for readability then reads one byte, proving the reactor wakes an rt task.
    struct ReadOne {
        stream: Rc<Async<TcpStream>>,
        got: Rc<std::cell::Cell<Option<u8>>>,
    }

    impl Future for ReadOne {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
            let mut buf = [0u8; 1];
            match self.stream.get_ref().try_clone().and_then(|mut s| std::io::Read::read(&mut s, &mut buf)) {
                Ok(n) if n > 0 => {
                    self.got.set(Some(buf[0]));
                    Poll::Ready(())
                }
                _ => {
                    let _ = self.stream.poll_readable(cx);
                    Poll::Pending
                }
            }
        }
    }

    #[test]
    fn reactor_wakes_a_task_on_readability() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        let reactor = Reactor::new().unwrap();
        let ex = rt::Executor::new(Box::new(PollPark::new(reactor.clone())));
        let stream = Rc::new(Async::new(client, reactor).unwrap());
        let got = Rc::new(std::cell::Cell::new(None));

        ex.spawn(ReadOne { stream: stream.clone(), got: got.clone() });
        ex.poll();
        assert_eq!(got.get(), None);

        // A write from the peer makes the client readable, the reactor tick wakes the task.
        server.write_all(&[42]).unwrap();
        ex.run();
        assert_eq!(got.get(), Some(42));
    }

    // Hits the real network, ignored by default, run with --ignored to validate the full stack.
    #[test]
    #[ignore]
    fn fetch_reaches_a_real_endpoint() {
        let reactor = Reactor::new().unwrap();
        let ex = rt::Executor::new(Box::new(PollPark::new(reactor.clone())));
        let out = Rc::new(std::cell::RefCell::new(None));
        let o = out.clone();
        ex.spawn(async move {
            let raw = super::fetch("https://example.com/".into(), reactor).await;
            *o.borrow_mut() = Some(raw);
        });
        ex.run();
        let raw = out.borrow_mut().take().unwrap().expect("fetch failed");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("Example Domain"), "response was: {}", &text[..text.len().min(200)]);
    }
}
