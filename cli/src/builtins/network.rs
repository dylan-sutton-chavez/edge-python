use super::{int_arg, opt_str_arg, str_arg, text};
use compiler::abi::WireValue;
use crate::ws::{accept_key, base64_encode, encode_frame, parse_frame};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/* Http defers to a worker thread, a stream returns its handle at once and feeds `receive()`. */
pub const EXPORTS: [(&str, bool); 10] = [
    ("fetch", true),
    ("fetch_text", true),
    ("fetch_json", true),
    ("ws_open", false),
    ("ws_send", false),
    ("ws_close", false),
    ("ws_state", false),
    ("sse_open", false),
    ("sse_close", false),
    ("sse_state", false),
];

// Request ids, the browser shape carries one even though nothing aborts here.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

// How long a stream thread blocks on a read before it serves queued sends and closes.
const POLL: Duration = Duration::from_millis(20);

// Receives each event a stream produces, as the JSON line `receive()` hands the script.
pub type Emit = Box<dyn Fn(String) + Send>;

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

pub fn deferred(name: &str, args: &[WireValue]) -> Result<WireValue, String> {
    let who = format!("network.{name}");
    let url = str_arg(args, 0, &who)?;
    let options = opt_str_arg(args, 1, &who)?;
    match name {
        "fetch" => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            Ok(text(match request(&url, options.as_deref()) {
                Ok(r) => {
                    let ok = (200..300).contains(&r.status);
                    let headers: Vec<String> = r.headers.iter().map(|(k, v)| format!("{}:{}", quote(k), quote(v))).collect();
                    format!("{{\"id\":{id},\"ok\":{ok},\"status\":{},\"headers\":{{{}}},\"body\":{}}}", r.status, headers.join(","), quote(&r.body))
                }
                Err(e) => format!("{{\"id\":{id},\"ok\":false,\"status\":0,\"error\":{}}}", quote(&e)),
            }))
        }
        "fetch_text" | "fetch_json" => {
            let r = request(&url, options.as_deref())?;
            if !(200..300).contains(&r.status) {
                return Err(format!("HTTP {}", r.status));
            }
            Ok(text(r.body))
        }
        _ => Err(format!("{who} is not an export")),
    }
}

/* One request over ureq, `options` is the RequestInit subset method, headers and body. */
fn request(url: &str, options: Option<&str>) -> Result<Reply, String> {
    let opts: serde_json::Value = match options.filter(|o| !o.trim().is_empty()) {
        Some(json) => serde_json::from_str(json).map_err(|e| format!("invalid options json: {e}"))?,
        None => serde_json::Value::Null,
    };
    let method = opts.get("method").and_then(|m| m.as_str()).unwrap_or("GET").to_ascii_uppercase();
    let body = match opts.get("body") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::String(s)) => s.clone().into_bytes(),
        Some(other) => other.to_string().into_bytes(),
    };
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(url);
    if let Some(headers) = opts.get("headers").and_then(|h| h.as_object()) {
        for (k, v) in headers {
            let value = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            builder = builder.header(k.as_str(), value);
        }
    }
    let req = builder.body(body).map_err(|e| e.to_string())?;
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).build());
    let mut resp = agent.run(req).map_err(failure)?;
    let status = resp.status().as_u16();
    let headers = resp.headers().iter().map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let body = resp.body_mut().read_to_string().map_err(failure)?;
    Ok(Reply { status, headers, body })
}

// A socket error reads as the OS says it, without the transport prefix ureq adds.
fn failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Io(e) => e.to_string(),
        e => e.to_string(),
    }
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/* One ws or sse connection as scripts see it, its readyState plus the work its thread drains. */
#[derive(Default)]
struct Socket {
    state: i64,
    outgoing: Vec<String>,
    closing: bool,
}

fn sockets() -> MutexGuard<'static, Vec<Socket>> {
    static SOCKETS: Mutex<Vec<Socket>> = Mutex::new(Vec::new());
    SOCKETS.lock().unwrap_or_else(|e| e.into_inner())
}

/* ws_open and sse_open return a handle at once, the connection runs on its own thread. */
pub fn open(name: &str, args: &[WireValue], emit: Emit) -> Result<WireValue, String> {
    let who = format!("network.{name}");
    let url = str_arg(args, 0, &who)?;
    let msg = str_arg(args, 1, &who)?;
    let handle = {
        let mut table = sockets();
        table.push(Socket::default());
        table.len() - 1
    };
    let ws = name == "ws_open";
    std::thread::spawn(move || {
        let result = if ws { run_ws(&url, &msg, handle, &emit) } else { run_sse(&url, &msg, handle, &emit) };
        if result.is_err() {
            emit(event(&msg, "error", ""));
        }
        // A finished stream reports closed, 3 for a socket and 2 for an event source.
        set_state(handle, if ws { 3 } else { 2 });
    });
    Ok(WireValue::Int(handle as i128))
}

/* ws_send, ws_close, ws_state, sse_close and sse_state against a live handle. */
pub fn call(name: &str, args: &[WireValue]) -> Result<WireValue, String> {
    let who = format!("network.{name}");
    let handle = int_arg(args, 0, &who)?;
    match name {
        "ws_send" => {
            let data = str_arg(args, 1, &who)?;
            with_socket(handle, &who, |s| s.outgoing.push(data))
        }
        "ws_close" | "sse_close" => with_socket(handle, &who, |s| {
            s.closing = true;
            s.state = 2;
        }),
        "ws_state" => Ok(WireValue::Int(state_of(handle, 3).into())),
        "sse_state" => Ok(WireValue::Int(state_of(handle, 2).into())),
        _ => Err(format!("{who} is not an export")),
    }
}

fn with_socket(handle: i64, who: &str, f: impl FnOnce(&mut Socket)) -> Result<WireValue, String> {
    let mut table = sockets();
    match usize::try_from(handle).ok().and_then(|h| table.get_mut(h)) {
        Some(socket) => {
            f(socket);
            Ok(WireValue::None)
        }
        None => Err(format!("{who} invalid socket handle {handle}")),
    }
}

// An unknown handle reads as closed, the way a finished stream does.
fn state_of(handle: i64, closed: i64) -> i64 {
    usize::try_from(handle).ok().and_then(|h| sockets().get(h).map(|s| s.state)).unwrap_or(closed)
}

fn set_state(handle: usize, state: i64) {
    if let Some(socket) = sockets().get_mut(handle) {
        socket.state = state;
    }
}

// The sends queued since the last pass and whether the script asked to close.
fn take_work(handle: usize) -> (Vec<String>, bool) {
    sockets().get_mut(handle).map_or((Vec::new(), true), |s| (std::mem::take(&mut s.outgoing), s.closing))
}

fn event(msg: &str, kind: &str, extra: &str) -> String {
    format!("{{\"msg\":{},\"type\":\"{kind}\"{extra}}}", quote(msg))
}

/* An http, https, ws or wss url split into what the socket and the request line need. */
struct Target<'a> {
    host: &'a str,
    port: u16,
    path: &'a str,
    secure: bool,
}

fn parse_url(url: &str) -> io::Result<Target<'_>> {
    let (rest, secure) = [("https://", true), ("wss://", true), ("http://", false), ("ws://", false)]
        .into_iter()
        .find_map(|(scheme, secure)| url.strip_prefix(scheme).map(|rest| (rest, secure)))
        .ok_or_else(|| io::Error::other("only http https ws and wss are supported"))?;
    let (authority, path) = rest.find('/').map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().map_err(|_| io::Error::other("bad port"))?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    Ok(Target { host, port, path, secure })
}

/* A plaintext or TLS socket whose reads give up after POLL so the thread stays responsive. */
enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

fn connect(target: &Target) -> io::Result<Conn> {
    let addr = (target.host, target.port).to_socket_addrs()?.next().ok_or_else(|| io::Error::other("dns returned no address"))?;
    let mut tcp = TcpStream::connect(addr)?;
    if !target.secure {
        tcp.set_read_timeout(Some(POLL))?;
        return Ok(Conn::Plain(tcp));
    }
    let name = rustls::pki_types::ServerName::try_from(target.host.to_string()).map_err(|_| io::Error::other("invalid dns name"))?;
    let mut tls = rustls::ClientConnection::new(tls_config(), name).map_err(io::Error::other)?;
    // The handshake runs blocking, the poll timeout only applies once records flow.
    while tls.is_handshaking() {
        tls.complete_io(&mut tcp)?;
    }
    tcp.set_read_timeout(Some(POLL))?;
    Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(tls, tcp))))
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let builder = rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("ring supports the default TLS versions");
            Arc::new(builder.with_root_certificates(roots).with_no_client_auth())
        })
        .clone()
}

// A read that timed out carries no error, the thread just loops again.
fn idle(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/* Reads the response head, returns it plus any bytes that arrived after the blank line. */
fn read_head(conn: &mut Conn) -> io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        match conn.read(&mut chunk) {
            Ok(0) => return Err(io::Error::other("closed before the response head")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if idle(&e) => {}
            Err(e) => return Err(e),
        }
    }
}

fn status_of(head: &str) -> Option<u16> {
    head.split("\r\n").next()?.split(' ').nth(1)?.parse().ok()
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.split("\r\n").skip(1).filter_map(|line| line.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

fn host_header(target: &Target) -> String {
    let default = if target.secure { 443 } else { 80 };
    if target.port == default { target.host.to_string() } else { format!("{}:{}", target.host, target.port) }
}

/* A websocket client, open message and close events tagged with `msg`. */
fn run_ws(url: &str, msg: &str, handle: usize, emit: &Emit) -> io::Result<()> {
    let target = parse_url(url)?;
    let mut conn = connect(&target)?;
    let key = base64_encode(&random::<16>());
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: edge-python\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        target.path,
        host_header(&target)
    );
    conn.write_all(request.as_bytes())?;
    let (head, mut buf) = read_head(&mut conn)?;
    if status_of(&head) != Some(101) {
        return Err(io::Error::other(format!("handshake status {:?}", status_of(&head))));
    }
    if header(&head, "sec-websocket-accept") != Some(accept_key(&key).as_str()) {
        return Err(io::Error::other("accept key mismatch"));
    }
    set_state(handle, 1);
    emit(event(msg, "open", ""));
    let mut chunk = [0u8; 8192];
    loop {
        let (frames, closing) = take_work(handle);
        for data in frames {
            conn.write_all(&encode_frame(0x1, data.as_bytes(), Some(random())))?;
        }
        if closing {
            let _ = conn.write_all(&encode_frame(0x8, &1000u16.to_be_bytes(), Some(random())));
            set_state(handle, 3);
            emit(event(msg, "close", ",\"code\":1000,\"reason\":\"\",\"was_clean\":true"));
            return Ok(());
        }
        if !drain_frames(&mut buf, &mut conn, msg, handle, emit)? {
            return Ok(());
        }
        match conn.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if idle(&e) => {}
            Err(e) => return Err(e),
        }
    }
}

// Handles every complete frame in `buf`, false once a close frame ends the stream.
fn drain_frames(buf: &mut Vec<u8>, conn: &mut Conn, msg: &str, handle: usize, emit: &Emit) -> io::Result<bool> {
    while let Some((opcode, payload, used)) = parse_frame(buf) {
        buf.drain(..used);
        match opcode {
            0x1 => emit(event(msg, "message", &format!(",\"data\":{}", quote(&String::from_utf8_lossy(&payload))))),
            0x2 => emit(event(msg, "message", ",\"binary\":true")),
            0x8 => {
                let code = payload.get(..2).map_or(1005, |c| u16::from_be_bytes([c[0], c[1]]));
                let reason = payload.get(2..).map(String::from_utf8_lossy).unwrap_or_default();
                let _ = conn.write_all(&encode_frame(0x8, &[], Some(random())));
                set_state(handle, 3);
                emit(event(msg, "close", &format!(",\"code\":{code},\"reason\":{},\"was_clean\":true", quote(&reason))));
                return Ok(false);
            }
            0x9 => conn.write_all(&encode_frame(0xA, &payload, Some(random())))?,
            _ => {}
        }
    }
    Ok(true)
}

/* An event source client, open and message events tagged with `msg`. */
fn run_sse(url: &str, msg: &str, handle: usize, emit: &Emit) -> io::Result<()> {
    let target = parse_url(url)?;
    let mut conn = connect(&target)?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: edge-python\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        target.path,
        host_header(&target)
    );
    conn.write_all(request.as_bytes())?;
    let (head, mut wire) = read_head(&mut conn)?;
    let status = status_of(&head).unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(io::Error::other(format!("HTTP {status}")));
    }
    let chunked = header(&head, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    set_state(handle, 1);
    emit(event(msg, "open", ""));
    let mut events = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if chunked {
            let used = dechunk(&wire, &mut events);
            wire.drain(..used);
        } else {
            events.append(&mut wire);
        }
        drain_events(&mut events, msg, emit);
        if take_work(handle).1 {
            return Ok(());
        }
        match conn.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => wire.extend_from_slice(&chunk[..n]),
            Err(e) if idle(&e) => {}
            Err(e) => return Err(e),
        }
    }
}

// Moves every complete chunk's payload into `out`, returns the bytes consumed.
fn dechunk(buf: &[u8], out: &mut Vec<u8>) -> usize {
    let mut i = 0;
    while let Some(nl) = buf[i..].windows(2).position(|w| w == b"\r\n") {
        let line = std::str::from_utf8(&buf[i..i + nl]).unwrap_or("");
        let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        let start = i + nl + 2;
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

// Emits one message per complete event in `buf`, the data lines joined and the id when sent.
fn drain_events(buf: &mut Vec<u8>, msg: &str, emit: &Emit) {
    while let Some(end) = event_end(buf) {
        let raw = String::from_utf8_lossy(&buf[..end]).into_owned();
        buf.drain(..end);
        let mut data = String::new();
        let mut id = String::new();
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            } else if let Some(v) = line.strip_prefix("id:") {
                id = v.strip_prefix(' ').unwrap_or(v).to_string();
            }
        }
        if data.is_empty() {
            continue;
        }
        let id = if id.is_empty() { String::new() } else { format!(",\"event_id\":{}", quote(&id)) };
        emit(event(msg, "message", &format!(",\"data\":{}{id}", quote(&data))));
    }
}

fn event_end(buf: &[u8]) -> Option<usize> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 2);
    lf.or_else(|| buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4))
}

// Nonce bytes from a xorshift over the clock, keys and masks only need to vary.
fn random<const N: usize>() -> [u8; N] {
    static SEED: AtomicU64 = AtomicU64::new(0);
    let mut x = (crate::host::now_ns() ^ SEED.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)) | 1;
    let mut out = [0u8; N];
    for b in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver};

    #[test]
    fn chunked_bodies_decode_across_reads() {
        let mut out = Vec::new();
        let wire = b"5\r\nhello\r\n3\r\nabc\r\n";
        assert_eq!(dechunk(&wire[..9], &mut out), 0);
        assert_eq!(dechunk(wire, &mut out), wire.len());
        assert_eq!(out, b"helloabc");
    }

    #[test]
    fn urls_default_their_port_by_scheme() {
        let t = parse_url("wss://example.com/chat?x=1").unwrap();
        assert_eq!((t.host, t.port, t.path, t.secure), ("example.com", 443, "/chat?x=1", true));
        let t = parse_url("ws://127.0.0.1:9000").unwrap();
        assert_eq!((t.host, t.port, t.path, t.secure), ("127.0.0.1", 9000, "/", false));
        assert!(parse_url("ftp://example.com/").is_err());
    }

    #[test]
    fn fetch_errors_carry_no_transport_prefix() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = request(&format!("http://127.0.0.1:{port}/"), None).err().unwrap();
        assert!(!err.starts_with("io: "), "error was {err}");
    }

    #[test]
    fn fetch_replies_are_json_with_escaped_bodies() {
        let body = "line\n\u{f1} \"q\" \u{1}";
        let port = serve_once(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Note: a \"q\"\r\n\r\n{body}", body.len()));
        let reply = deferred("fetch", &[text(format!("http://127.0.0.1:{port}/"))]).unwrap();
        let WireValue::Bytes(json) = reply else { panic!("fetch returns a str") };
        let parsed: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(parsed["body"], body);
        assert_eq!(parsed["headers"]["x-note"], "a \"q\"");
        assert_eq!(parsed["status"], 200);
    }

    fn serve_once(response: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut head = [0u8; 4096];
            let _ = sock.read(&mut head);
            sock.write_all(response.as_bytes()).unwrap();
        });
        port
    }

    fn collect() -> (Emit, Receiver<String>) {
        let (tx, rx) = channel();
        (Box::new(move |line| drop(tx.send(line))), rx)
    }

    fn next(rx: &Receiver<String>) -> serde_json::Value {
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).expect("an event")).unwrap()
    }

    #[test]
    fn a_refused_stream_reports_an_error_event() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (emit, rx) = collect();
        open("ws_open", &[text(format!("ws://127.0.0.1:{port}/")), text("x".into())], emit).unwrap();
        assert_eq!(next(&rx)["type"], "error");
        assert!(call("ws_send", &[WireValue::Int(1 << 40), text("x".into())]).unwrap_err().contains("invalid socket handle"));
    }
}
