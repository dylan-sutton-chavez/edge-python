use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use compiler::util::ws::{accept_key, encode_frame, parse_frame};

// Network parity fixture, binds an ephemeral port then serves canned http, sse and a ws echo.
fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    // The launcher reads this first line to learn where to point the corpus.
    println!("{}", listener.local_addr().unwrap().port());
    let _ = std::io::stdout().flush();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // One thread per connection keeps concurrent fetches and the ws echo independent.
        std::thread::spawn(move || serve(stream));
    }
}

// Reads one request, dispatches on its path, then either closes or streams.
fn serve(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    // Drain headers, keeping the websocket key when the client is upgrading.
    let mut ws_key = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Sec-WebSocket-Key:").or_else(|| line.strip_prefix("sec-websocket-key:")) {
            ws_key = Some(v.trim().to_string());
        }
    }

    // The browser preflights cross-origin, these headers satisfy CORS for the fetch that follows.
    if method == "OPTIONS" {
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }

    match path.as_str() {
        "/text" => http(&mut stream, "text/plain", "hello from mock"),
        "/json" => http(&mut stream, "application/json", "{\"ok\":true}"),
        "/sse" => sse(&mut stream),
        "/ws" => {
            if let Some(key) = ws_key {
                ws_echo(stream, reader, &key);
            }
        }
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    }
}

// A closed-length response, the fetch client reads until EOF.
fn http(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

// A short event stream, three data events then the connection stays open.
fn sse(stream: &mut TcpStream) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for i in 1..=3 {
        if stream.write_all(format!("id: {i}\ndata: event {i}\n\n").as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
    }
    // Hold the socket so the client sees a live stream, the launcher kills the process.
    let mut sink = [0u8; 64];
    while let Ok(n) = stream.read(&mut sink) {
        if n == 0 {
            return;
        }
    }
}

// Completes the upgrade then echoes text frames, answering close and ping.
fn ws_echo(mut stream: TcpStream, mut reader: BufReader<TcpStream>, key: &str) {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    );
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        while let Some((opcode, payload, consumed)) = parse_frame(&buf) {
            buf.drain(..consumed);
            match opcode {
                // Echo text back unmasked, the server side never masks.
                0x1 => {
                    if stream.write_all(&encode_frame(0x1, &payload, None)).is_err() {
                        return;
                    }
                }
                0x8 => {
                    let _ = stream.write_all(&encode_frame(0x8, &payload, None));
                    return;
                }
                0x9 => {
                    let _ = stream.write_all(&encode_frame(0xA, &payload, None));
                }
                _ => {}
            }
        }
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}
