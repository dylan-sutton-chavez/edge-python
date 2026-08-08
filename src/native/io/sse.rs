use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;

use super::connect::{parse_url, Connect};
use super::http::{find_headers_end, write_all};
use super::runtime::{set_state, Completions};
use super::Reactor;

// Streams an event source, pushing each event onto the shared queue tagged with msg.
pub async fn sse(url: String, msg: String, handle: usize, reactor: Reactor, sink: Rc<RefCell<Completions>>) {
    let esc = crate::devkit::escape;
    let push = |json: String| sink.borrow_mut().events.push(json);
    if run_sse(&url, &msg, handle, reactor, &sink, &push, esc).await.is_err() {
        push(format!("{{\"msg\":\"{}\",\"type\":\"error\"}}", esc(&msg)));
    }
    // A finished stream closes its slot so sse_state reports closed.
    set_state(&sink, handle, 2);
}

async fn run_sse(
    url: &str,
    msg: &str,
    handle: usize,
    reactor: Reactor,
    sink: &Rc<RefCell<Completions>>,
    push: &dyn Fn(String),
    esc: fn(&str) -> String,
) -> std::io::Result<()> {
    let (host, port, secure, request) = {
        let u = parse_url(url)?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: edge-python\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
            u.path, u.host
        );
        (u.host.to_string(), u.port, u.secure, req.into_bytes())
    };

    let mut stream = Connect::new(&host, port, secure, reactor).await?;
    write_all(&mut stream, &request).await?;
    // Three buffers, raw bytes off the wire, chunked stripped, and per-event drained.
    let mut raw = Vec::new();
    let mut wire = Vec::new();
    let mut events_buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut chunked = false;
    let mut past_headers = false;
    let mut opened = false;
    loop {
        // A read wakes on bytes, sse_close wakes it early to end the stream.
        let n = std::future::poll_fn(|cx| {
            if let Some(Some(b)) = sink.borrow_mut().sockets.get_mut(handle) {
                if b.closing {
                    return Poll::Ready(Ok(0));
                }
                b.waker = Some(cx.waker().clone());
            }
            stream.poll_read(cx, &mut chunk)
        })
        .await?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&chunk[..n]);
        if !past_headers {
            let Some(hdr_end) = find_headers_end(&raw) else { continue };
            let (status, is_chunked) = parse_response_head(&raw[..hdr_end])?;
            if !(200..300).contains(&status) {
                return Err(std::io::Error::other(format!("HTTP {status}")));
            }
            chunked = is_chunked;
            wire.extend_from_slice(&raw[hdr_end..]);
            raw.clear();
            past_headers = true;
        } else {
            wire.extend_from_slice(&chunk[..n]);
        }
        if !opened {
            push(format!("{{\"msg\":\"{}\",\"type\":\"open\"}}", esc(msg)));
            set_state(sink, handle, 1);
            opened = true;
        }
        if chunked {
            let consumed = dechunk(&wire, &mut events_buf);
            wire.drain(..consumed);
        } else {
            events_buf.append(&mut wire);
        }
        drain_events(&mut events_buf, msg, push, esc);
    }
}

// Reads status code and whether transfer-encoding is chunked from response headers.
fn parse_response_head(head: &[u8]) -> std::io::Result<(u16, bool)> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    match resp.parse(head) {
        Ok(_) => {}
        Err(e) => return Err(std::io::Error::other(format!("bad response head, {e}"))),
    }
    let status = resp.code.unwrap_or(0);
    let chunked = resp.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("transfer-encoding")
            && String::from_utf8_lossy(h.value).to_ascii_lowercase().contains("chunked")
    });
    Ok((status, chunked))
}

// Consumes as many complete chunks as possible, appending payload to out, returns bytes consumed.
fn dechunk(buf: &[u8], out: &mut Vec<u8>) -> usize {
    let mut i = 0;
    while i < buf.len() {
        // A chunk begins with a hex size line ending in CRLF.
        let Some(nl) = buf[i..].windows(2).position(|w| w == b"\r\n") else { break };
        let size_str = std::str::from_utf8(&buf[i..i + nl]).unwrap_or("");
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        let start = i + nl + 2;
        // Full chunk needs size bytes plus a trailing CRLF.
        if buf.len() < start + size + 2 {
            break;
        }
        if size == 0 {
            return start + 2;
        }
        out.extend_from_slice(&buf[start..start + size]);
        i = start + size + 2;
    }
    i
}

// Splits complete events off the front of buf, emitting the web message shape for each.
fn drain_events(buf: &mut Vec<u8>, msg: &str, push: &dyn Fn(String), esc: fn(&str) -> String) {
    while let Some(end) = find_event_end(buf) {
        let raw = String::from_utf8_lossy(&buf[..end]).into_owned();
        buf.drain(..end);
        let mut data = String::new();
        let mut event_id = String::new();
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() { data.push('\n'); }
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            } else if let Some(v) = line.strip_prefix("id:") {
                event_id = v.strip_prefix(' ').unwrap_or(v).to_string();
            }
        }
        if data.is_empty() {
            continue;
        }
        let id_field = if event_id.is_empty() { String::new() } else { format!(",\"event_id\":\"{}\"", esc(&event_id)) };
        push(format!("{{\"msg\":\"{}\",\"type\":\"message\",\"data\":\"{}\"{}}}", esc(msg), esc(&data), id_field));
    }
}

// Byte index just past the first blank-line event boundary, if any.
fn find_event_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 2)
        .or_else(|| buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4))
}
