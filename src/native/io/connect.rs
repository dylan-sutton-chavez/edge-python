use std::future::Future;
use std::io;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::{connect_tcp, Async, Reactor, Stream, TlsStream};

// Parsed url, secure picks TLS over plaintext, the rest addresses the socket.
pub(super) struct Url<'a> {
    pub host: &'a str,
    pub port: u16,
    pub path: &'a str,
    pub secure: bool,
}

pub(super) fn parse_url(url: &str) -> io::Result<Url<'_>> {
    // https and wss ride TLS, http and ws are plaintext for local sidecars and fixtures.
    let (rest, secure) = if let Some(r) = url.strip_prefix("https://") {
        (r, true)
    } else if let Some(r) = url.strip_prefix("wss://") {
        (r, true)
    } else if let Some(r) = url.strip_prefix("http://") {
        (r, false)
    } else if let Some(r) = url.strip_prefix("ws://") {
        (r, false)
    } else {
        return Err(io::Error::other("only http https ws and wss are supported"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| io::Error::other("bad port"))?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    Ok(Url { host, port, path, secure })
}

// Resolves to a ready stream past connect and any handshake, shared by fetch, sse and ws.
pub(super) struct Connect {
    state: ConnState,
    reactor: Reactor,
}

enum ConnState {
    Start { host: String, port: u16, secure: bool },
    Connecting { sock: Async<std::net::TcpStream>, host: String, secure: bool },
    Handshake { tls: Box<TlsStream> },
    Done,
}

impl Connect {
    pub fn new(host: &str, port: u16, secure: bool, reactor: Reactor) -> Self {
        Connect {
            state: ConnState::Start { host: host.to_string(), port, secure },
            reactor,
        }
    }
}

impl Future for Connect {
    type Output = io::Result<Stream>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                ConnState::Start { host, port, secure } => {
                    let addr = (host.as_str(), *port)
                        .to_socket_addrs()?
                        .next()
                        .ok_or_else(|| io::Error::other("dns returned no address"))?;
                    let sock = connect_tcp(addr, &this.reactor)?;
                    let (host, secure) = (core::mem::take(host), *secure);
                    this.state = ConnState::Connecting { sock, host, secure };
                }
                ConnState::Connecting { sock, .. } => {
                    match sock.poll_connected(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(e)) => { this.state = ConnState::Done; return Poll::Ready(Err(e)); }
                        Poll::Ready(Ok(())) => {}
                    }
                    let ConnState::Connecting { sock, host, secure } = core::mem::replace(&mut this.state, ConnState::Done) else {
                        unreachable!()
                    };
                    // Plaintext is ready at connect, TLS still needs its handshake.
                    if !secure {
                        return Poll::Ready(Ok(Stream::plain(sock)));
                    }
                    let tls = TlsStream::connect(&host, sock)?;
                    this.state = ConnState::Handshake { tls: Box::new(tls) };
                }
                ConnState::Handshake { tls } => {
                    match tls.poll_handshake(cx) {
                        Poll::Ready(Ok(())) => {
                            let ConnState::Handshake { tls } = core::mem::replace(&mut this.state, ConnState::Done) else {
                                unreachable!()
                            };
                            return Poll::Ready(Ok(Stream::Tls(tls)));
                        }
                        Poll::Ready(Err(e)) => { this.state = ConnState::Done; return Poll::Ready(Err(e)); }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                ConnState::Done => return Poll::Ready(Err(io::Error::other("polled after completion"))),
            }
        }
    }
}
