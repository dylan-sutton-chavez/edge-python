use std::io;

use super::connect::{parse_url, Connect};
use super::Reactor;

// Fetches one GET, resolving to the raw response bytes over plaintext or TLS.
pub async fn fetch(url: String, reactor: Reactor) -> io::Result<Vec<u8>> {
    let (host, port, secure, request) = {
        let u = parse_url(&url)?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: edge-python\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            u.path, u.host
        );
        (u.host.to_string(), u.port, u.secure, req.into_bytes())
    };

    let mut stream = Connect::new(&host, port, secure, reactor).await?;
    write_all(&mut stream, &request).await?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = std::future::poll_fn(|cx| stream.poll_read(cx, &mut chunk)).await?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

// Sends the whole buffer, awaiting writability between partial writes.
pub(super) async fn write_all(stream: &mut super::Stream, buf: &[u8]) -> io::Result<()> {
    std::future::poll_fn(|cx| stream.poll_write_all(cx, buf)).await
}

// Byte index just past the blank line that ends the response headers, if present.
pub(super) fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}
