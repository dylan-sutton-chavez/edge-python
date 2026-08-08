use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use rustls::{ClientConfig, ClientConnection, RootCertStore};

use super::{Async, Reactor};

// One shared client config, roots loaded once.
fn config() -> Arc<ClientConfig> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Arc::new(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
    })
    .clone()
}

// A TLS client over a reactor socket, drives the rustls state machine non-blocking.
pub struct TlsStream {
    conn: ClientConnection,
    sock: Async<TcpStream>,
}

impl TlsStream {
    pub fn connect(host: &str, sock: Async<TcpStream>) -> io::Result<Self> {
        let name = host.to_string().try_into().map_err(|_| io::Error::other("invalid dns name"))?;
        let conn = ClientConnection::new(config(), name).map_err(io::Error::other)?;
        Ok(TlsStream { conn, sock })
    }

    // Pump pending TLS writes to the socket, returns Pending when the socket blocks.
    fn poll_flush(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        while self.conn.wants_write() {
            match self.sock.poll_write_with(cx, |mut s| self.conn.write_tls(&mut s)) {
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    // Feed one socket read into the TLS engine and process it.
    fn poll_fill(&mut self, cx: &mut Context) -> Poll<io::Result<usize>> {
        match self.sock.poll_read_with(cx, |mut s| self.conn.read_tls(&mut s)) {
            Poll::Ready(Ok(n)) => {
                self.conn.process_new_packets().map_err(io::Error::other)?;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    // Drive the handshake to completion.
    pub fn poll_handshake(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        while self.conn.is_handshaking() {
            if self.poll_flush(cx)?.is_pending() {
                return Poll::Pending;
            }
            if self.conn.is_handshaking() {
                match self.poll_fill(cx) {
                    Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                    Poll::Ready(Ok(_)) => {}
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
        self.poll_flush(cx)
    }

    // Encrypt and send plaintext, flushing to the socket.
    pub fn poll_write_all(&mut self, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<()>> {
        self.conn.writer().write_all(buf)?;
        self.poll_flush(cx)
    }

    // Read decrypted plaintext, pulling more ciphertext from the socket when empty.
    pub fn poll_read(&mut self, cx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        loop {
            match self.conn.reader().read(buf) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Poll::Ready(Err(e)),
            }
            match self.poll_fill(cx) {
                Poll::Ready(Ok(0)) => return Poll::Ready(Ok(0)),
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub fn connect_tcp(addr: std::net::SocketAddr, reactor: &Reactor) -> io::Result<Async<TcpStream>> {
    let sock = TcpStream::connect(addr)?;
    Async::new(sock, reactor.clone())
}
