use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;

use super::connect::{parse_url, Connect};
use super::http::{find_headers_end, write_all};
use super::runtime::{set_state, Completions};
use super::Reactor;
use crate::util::ws::{accept_key, base64_encode, encode_frame, parse_frame};

// Streams a websocket, pushing open message close events tagged with msg.
pub async fn ws(url: String, msg: String, handle: usize, reactor: Reactor, sink: Rc<RefCell<Completions>>) {
    let esc = crate::devkit::escape;
    let push = |json: String| sink.borrow_mut().events.push(json);
    if run_ws(&url, &msg, handle, reactor, &sink, &push, esc).await.is_err() {
        push(format!("{{\"msg\":\"{}\",\"type\":\"error\"}}", esc(&msg)));
    }
    // A finished task closes its slot so ws_state reports closed.
    set_state(&sink, handle, 3);
}

// What the wait resolved to, more bytes to parse or a pending send to flush.
enum Wake {
    Read(usize),
    Flush,
}

async fn run_ws(
    url: &str,
    msg: &str,
    handle: usize,
    reactor: Reactor,
    sink: &Rc<RefCell<Completions>>,
    push: &dyn Fn(String),
    esc: fn(&str) -> String,
) -> std::io::Result<()> {
    let (host, port, secure, request, key) = {
        let u = parse_url(url)?;
        let key = ws_key();
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: edge-python\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            u.path, u.host, key
        );
        (u.host.to_string(), u.port, u.secure, req.into_bytes(), key)
    };

    let mut stream = Connect::new(&host, port, secure, reactor).await?;
    write_all(&mut stream, &request).await?;

    // Read the handshake response, verifying the accept key before framing.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let hdr_end = loop {
        let n = std::future::poll_fn(|cx| stream.poll_read(cx, &mut chunk)).await?;
        if n == 0 {
            return Err(std::io::Error::other("closed during handshake"));
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find_headers_end(&buf) {
            break end;
        }
    };
    verify_handshake(&buf[..hdr_end], &key)?;
    buf.drain(..hdr_end);

    push(format!("{{\"msg\":\"{}\",\"type\":\"open\"}}", esc(msg)));
    set_state(sink, handle, 1);

    loop {
        // Either the socket became readable or ws_send queued an outgoing frame.
        let wake = std::future::poll_fn(|cx| {
            let mut c = sink.borrow_mut();
            if let Some(Some(b)) = c.sockets.get_mut(handle) {
                if !b.outgoing.is_empty() || b.closing {
                    return Poll::Ready(Ok(Wake::Flush));
                }
                b.waker = Some(cx.waker().clone());
            }
            drop(c);
            match stream.poll_read(cx, &mut chunk) {
                Poll::Ready(Ok(n)) => Poll::Ready(Ok(Wake::Read(n))),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await?;

        match wake {
            Wake::Flush => {
                let (frames, closing) = {
                    let mut c = sink.borrow_mut();
                    match c.sockets.get_mut(handle) {
                        Some(Some(b)) => (core::mem::take(&mut b.outgoing), b.closing),
                        _ => return Ok(()),
                    }
                };
                for text in frames {
                    let frame = encode_frame(0x1, text.as_bytes(), Some(mask_key()));
                    write_all(&mut stream, &frame).await?;
                }
                if closing {
                    let frame = encode_frame(0x8, &1000u16.to_be_bytes(), Some(mask_key()));
                    let _ = write_all(&mut stream, &frame).await;
                    push(format!("{{\"msg\":\"{}\",\"type\":\"close\",\"code\":1000,\"reason\":\"\",\"was_clean\":true}}", esc(msg)));
                    return Ok(());
                }
            }
            Wake::Read(0) => return Ok(()),
            Wake::Read(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if !drain_frames(&mut buf, &mut stream, msg, handle, sink, push, esc).await? {
                    return Ok(());
                }
            }
        }
    }
}

// Parses complete frames off buf, false when a close frame ends the stream.
async fn drain_frames(
    buf: &mut Vec<u8>,
    stream: &mut super::Stream,
    msg: &str,
    handle: usize,
    sink: &Rc<RefCell<Completions>>,
    push: &dyn Fn(String),
    esc: fn(&str) -> String,
) -> std::io::Result<bool> {
    while let Some((opcode, payload, consumed)) = parse_frame(buf) {
        buf.drain(..consumed);
        match opcode {
            0x1 => {
                let text = String::from_utf8_lossy(&payload);
                push(format!("{{\"msg\":\"{}\",\"type\":\"message\",\"data\":\"{}\"}}", esc(msg), esc(&text)));
            }
            0x2 => {
                push(format!("{{\"msg\":\"{}\",\"type\":\"message\",\"binary\":true}}", esc(msg)));
            }
            0x8 => {
                let code = if payload.len() >= 2 { u16::from_be_bytes([payload[0], payload[1]]) } else { 1005 };
                let reason = if payload.len() > 2 { String::from_utf8_lossy(&payload[2..]).into_owned() } else { String::new() };
                let _ = write_all(stream, &encode_frame(0x8, &[], Some(mask_key()))).await;
                push(format!("{{\"msg\":\"{}\",\"type\":\"close\",\"code\":{},\"reason\":\"{}\",\"was_clean\":true}}", esc(msg), code, esc(&reason)));
                set_state(sink, handle, 3);
                return Ok(false);
            }
            0x9 => {
                let _ = write_all(stream, &encode_frame(0xA, &payload, Some(mask_key()))).await;
            }
            _ => {}
        }
    }
    Ok(true)
}

// Confirms a 101 upgrade and that the accept key matches the sent key.
fn verify_handshake(head: &[u8], key: &str) -> std::io::Result<()> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    resp.parse(head).map_err(|e| std::io::Error::other(format!("bad handshake, {e}")))?;
    if resp.code != Some(101) {
        return Err(std::io::Error::other(format!("handshake status {:?}", resp.code)));
    }
    let want = accept_key(key);
    let got = resp.headers.iter().find(|h| h.name.eq_ignore_ascii_case("sec-websocket-accept"));
    match got {
        Some(h) if h.value == want.as_bytes() => Ok(()),
        _ => Err(std::io::Error::other("accept key mismatch")),
    }
}

// A base64 nonce for Sec-WebSocket-Key, derived from the clock, sufficient per spec.
fn ws_key() -> String {
    let mut bytes = [0u8; 16];
    fill_random(&mut bytes);
    base64_encode(&bytes)
}

// A four byte masking key, only needs to vary not to be secret.
fn mask_key() -> [u8; 4] {
    let mut m = [0u8; 4];
    fill_random(&mut m);
    m
}

// Fills bytes from a splitmix over the nanosecond clock, cheap and dependency free.
fn fill_random(out: &mut [u8]) {
    let mut x = crate::native::now_ns() ^ 0x9e3779b97f4a7c15u64.wrapping_mul(out.len() as u64 + 1);
    for b in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
}
