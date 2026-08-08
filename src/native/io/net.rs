use std::io;
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::task::{Context, Poll};

use super::Reactor;

// A source registered in the reactor, deregisters itself on drop.
pub struct Async<T: AsRawFd> {
    io: T,
    key: usize,
    reactor: Reactor,
}

impl Async<TcpStream> {
    // Non-blocking stream registered with the reactor.
    pub fn new(io: TcpStream, reactor: Reactor) -> io::Result<Self> {
        io.set_nonblocking(true)?;
        let key = reactor.register(io.as_raw_fd())?;
        Ok(Async { io, key, reactor })
    }

    // True when connected, false while still pending, Err on a failed connect.
    fn connect_error(&self) -> io::Result<bool> {
        if let Some(e) = self.io.take_error()? {
            return Err(e);
        }
        match self.io.peer_addr() {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotConnected => Ok(false),
            Err(e) => Err(e),
        }
    }

    // Ready once the non-blocking connect settles, surfacing any connect error.
    pub fn poll_connected(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        match self.connect_error() {
            Ok(true) => Poll::Ready(Ok(())),
            Ok(false) => {
                self.reactor.poll_write(self.key, self.io.as_raw_fd(), cx.waker())?;
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<T: AsRawFd> Async<T> {
    // Used by tests, kept as part of the source handle surface.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_ref(&self) -> &T {
        &self.io
    }

    // Ready when the OS reports readability, arms interest and parks the waker otherwise.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn poll_readable(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.reactor.poll_read(self.key, self.io.as_raw_fd(), cx.waker())?;
        Poll::Pending
    }

    // Retry a non-blocking op, parking on readability when it would block.
    pub fn poll_read_with<R>(&self, cx: &mut Context, mut op: impl FnMut(&T) -> io::Result<R>) -> Poll<io::Result<R>> {
        match op(&self.io) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.reactor.poll_read(self.key, self.io.as_raw_fd(), cx.waker())?;
                Poll::Pending
            }
            res => Poll::Ready(res),
        }
    }

    pub fn poll_write_with<R>(&self, cx: &mut Context, mut op: impl FnMut(&T) -> io::Result<R>) -> Poll<io::Result<R>> {
        match op(&self.io) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.reactor.poll_write(self.key, self.io.as_raw_fd(), cx.waker())?;
                Poll::Pending
            }
            res => Poll::Ready(res),
        }
    }
}

impl<T: AsRawFd> Drop for Async<T> {
    fn drop(&mut self) {
        self.reactor.deregister(self.key, self.io.as_raw_fd());
    }
}

