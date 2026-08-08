use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::task::{Context, Poll};

use super::{Async, TlsStream};

// A connection past its setup, plaintext for http and ws or TLS for https and wss.
pub enum Stream {
    // Written bytes are tracked so a partial write resumes at the right offset.
    Plain { sock: Async<TcpStream>, written: usize },
    Tls(Box<TlsStream>),
}

impl Stream {
    pub fn plain(sock: Async<TcpStream>) -> Self {
        Stream::Plain { sock, written: 0 }
    }

    // Reads decrypted or raw bytes, Pending until the socket has some.
    pub fn poll_read(&mut self, cx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self {
            Stream::Plain { sock, .. } => sock.poll_read_with(cx, |mut io| io.read(buf)),
            Stream::Tls(tls) => tls.poll_read(cx, buf),
        }
    }

    // Writes the whole buffer, encrypting for TLS, awaiting writability between partial writes.
    pub fn poll_write_all(&mut self, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<()>> {
        match self {
            Stream::Plain { sock, written } => {
                while *written < buf.len() {
                    match sock.poll_write_with(cx, |mut io| io.write(&buf[*written..])) {
                        Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                        Poll::Ready(Ok(n)) => *written += n,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                *written = 0;
                Poll::Ready(Ok(()))
            }
            Stream::Tls(tls) => tls.poll_write_all(cx, buf),
        }
    }
}
