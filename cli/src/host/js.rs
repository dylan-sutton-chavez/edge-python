use super::{cache_root, cdn, Completion, MemoryCap, Printer, ORIGIN};
use bytes::Bytes;
use compiler::abi::WireValue;
use compiler::util::sha256::{hex_encode, sha256};
use http_body_util::{BodyExt, Full};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store, UpdateDeadline};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::bindings::sync::ProxyPre;
use wasmtime_wasi_http::{Error as HttpError, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView};

include!(concat!(env!("OUT_DIR"), "/js_runtime.rs"));

const BOOT: &str = include_str!("js/boot.js");
const LOADER: &str = include_str!("js/loader.js");
// The loader's private authority, requests to it never leave the host.
const INTERNAL: &str = "edge.internal";
// A synchronous export still running past this is treated as a runaway and its runtime stopped.
const REPLY_LIMIT: Duration = Duration::from_secs(30);
// Bounds the runtime download, the precompiled component is about 24 MB.
const MAX_RUNTIME_BYTES: u64 = 128 << 20;
// A runtime's linear memory stops growing here, a lower cap on the run wins.
const MAX_RUNTIME_MEMORY: usize = 1 << 30;
// A runtime counts as active this long after its last change, timers often follow a closed body.
const SETTLE_NS: u64 = 250_000_000;
// Where a packed artifact stores the runtime, beside the project files it carries.
pub const RUNTIME_KEY: &str = "js-runtime.cwasm";

static PACKED: OnceLock<Vec<u8>> = OnceLock::new();

type Done = Box<dyn Future<Output = Result<(), HttpError>> + Send>;

// A module's files by their path inside its directory, the entry among them.
pub type Tree = Vec<(String, Vec<u8>)>;

/* StarlingMonkey linked against WASI, taken from the artifact when one carries it. */
pub fn load(engine: &Engine) -> Result<ProxyPre<JsState>, String> {
    let component = match PACKED.get() {
        Some(bytes) => unsafe { Component::deserialize(engine, bytes) }
            .map_err(|e| format!("loading the packed JavaScript runtime failed, {e}"))?,
        None => cached(engine)?,
    };
    let mut linker = Linker::<JsState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
    let pre = linker.instantiate_pre(&component).map_err(|e| e.to_string())?;
    ProxyPre::new(pre).map_err(|e| e.to_string())
}

/* The runtime from the user cache, downloaded and verified the first time it is missing. */
fn cached(engine: &Engine) -> Result<Component, String> {
    let path = runtime_path()?;
    if !path.exists() {
        fetch_runtime(&path)?;
    }
    // The file was written only after its bytes matched the hash this build pins.
    unsafe { Component::deserialize_file(engine, &path) }.map_err(|e| {
        let _ = std::fs::remove_file(&path);
        format!("loading '{}' failed, {e}", path.display())
    })
}

fn runtime_path() -> Result<std::path::PathBuf, String> {
    Ok(cache_root()?.join("js-runtime").join(format!("{JS_RUNTIME_SHA256}.cwasm")))
}

/* Keeps the runtime a packed artifact carries, ahead of the cache and the download. */
pub fn use_packed(bytes: Vec<u8>) {
    let _ = PACKED.set(bytes);
}

/* The precompiled runtime bytes, so `edge build` can carry them inside an artifact. */
pub fn runtime_bytes() -> Result<Vec<u8>, String> {
    let path = runtime_path()?;
    if !path.exists() {
        fetch_runtime(&path)?;
    }
    std::fs::read(&path).map_err(|e| format!("reading '{}' failed, {e}", path.display()))
}

fn fetch_runtime(path: &Path) -> Result<(), String> {
    let url = cdn(&format!("{ORIGIN}/js-runtime/{JS_RUNTIME_SHA256}.cwasm"));
    let spinner = crate::ui::spinner("fetching the JavaScript runtime");
    let result = download(&url, path);
    match &result {
        Ok(()) => spinner.done("fetched the JavaScript runtime"),
        Err(_) => spinner.fail("could not fetch the JavaScript runtime"),
    }
    result.map_err(|e| format!("fetching '{url}' failed, {e}"))
}

fn download(url: &str, path: &Path) -> Result<(), String> {
    let mut resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().take(MAX_RUNTIME_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if hex_encode(&sha256(&bytes)) != JS_RUNTIME_SHA256 {
        return Err(format!("the bytes do not match sha256-{JS_RUNTIME_SHA256}"));
    }
    let dir = path.parent().ok_or("the cache path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Temp plus rename keeps a partial download out of the cache.
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/* Relative specifiers after `from` or `import`, the static imports and re-exports of an ES module. */
pub fn imports(src: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for keyword in ["from", "import"] {
        for (at, _) in src.match_indices(keyword) {
            if src[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.')) {
                continue;
            }
            let rest = src[at + keyword.len()..].trim_start();
            let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else { continue };
            let Some(end) = rest[1..].find(quote) else { continue };
            let spec = &rest[1..1 + end];
            if spec.starts_with("./") || spec.starts_with("../") {
                found.push(spec);
            }
        }
    }
    found
}

/* Resolves `spec` against the tree file `from`, None when it climbs out of the tree's directory. */
pub fn join(from: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            seg => parts.push(seg),
        }
    }
    Some(parts.join("/"))
}

/* The JS host's one environment rule, a global a module lacks names the missing Web API. */
fn host_error(label: &str, name: &str, message: &str) -> String {
    match message.strip_suffix(" is not defined") {
        Some(api) if name == "ReferenceError" && !api.is_empty() && api.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            format!("module '{label}' needs '{api}', missing in this runtime")
        }
        _ => message.to_string(),
    }
}

/* Everything the StarlingMonkey instance reaches, WASI, outgoing HTTP and the internal channel. */
pub struct JsState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    hooks: Hooks,
    limiter: MemoryCap,
}

impl WasiView for JsState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl WasiHttpView for JsState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView { ctx: &mut self.http, table: &mut self.table, hooks: &mut self.hooks }
    }
}

struct Hooks {
    token: String,
    // The host's message stream, handed out to the first authorized request only.
    calls: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>>,
    open: Arc<AtomicUsize>,
    last: Arc<AtomicU64>,
    // Untrusted runs reach no network, only the internal channel answers.
    offline: bool,
}

// One outgoing exchange in flight, counted until its response body is dropped.
struct Exchange(Arc<AtomicUsize>, Arc<AtomicU64>);

impl Exchange {
    fn new(open: Arc<AtomicUsize>, last: Arc<AtomicU64>) -> Exchange {
        open.fetch_add(1, Ordering::SeqCst);
        Exchange(open, last)
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        self.1.store(super::now_ns(), Ordering::SeqCst);
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Counted {
    body: WasiBody,
    _open: Exchange,
}

impl http_body::Body for Counted {
    type Data = Bytes;
    type Error = HttpError;

    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>, HttpError>>> {
        std::pin::Pin::new(&mut self.body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

// The internal response body, one length-prefixed frame per batch of host messages.
struct ChannelBody(tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>);

impl http_body::Body for ChannelBody {
    type Data = Bytes;
    type Error = HttpError;

    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>, HttpError>>> {
        self.0.poll_recv(cx).map(|batch| {
            batch.map(|b| {
                let mut framed = (b.len() as u32).to_le_bytes().to_vec();
                framed.extend_from_slice(&b);
                Ok(http_body::Frame::data(Bytes::from(framed)))
            })
        })
    }
}

fn respond(status: u16, body: WasiBody) -> http::Response<WasiBody> {
    http::Response::builder().status(status).body(body).expect("static response")
}

impl WasiHttpHooks for Hooks {
    fn send_request(&mut self, request: http::Request<WasiBody>, options: Option<RequestOptions>, fut: Done) -> Box<dyn Future<Output = Result<(http::Response<WasiBody>, Done), HttpError>> + Send> {
        if request.uri().host() == Some(INTERNAL) {
            let authorized = request.headers().get("x-edge").is_some_and(|v| v.as_bytes() == self.token.as_bytes());
            let calls = if authorized && request.uri().path() == "/stream" { self.calls.take() } else { None };
            return Box::new(async move {
                let done: Done = Box::new(async { Ok(()) });
                let response = match calls {
                    Some(calls) => respond(200, ChannelBody(calls).boxed_unsync()),
                    None => respond(403, Full::new(Bytes::new()).map_err(|never| match never {}).boxed_unsync()),
                };
                Ok((response, done))
            });
        }
        drop(fut);
        if self.offline {
            return Box::new(async { Err(HttpError::HttpRequestDenied) });
        }
        let open = Exchange::new(self.open.clone(), self.last.clone());
        Box::new(async move {
            let (res, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            let res = res.map(|body| Counted { body: body.boxed_unsync(), _open: open }.boxed_unsync());
            Ok((res, Box::new(io) as Done))
        })
    }
}

/* What a call's waiter hears first, the rest of a pending call arrives as a completion. */
enum Reply {
    Value(WireValue),
    Pending,
    Bound(Vec<String>),
    Failed(String),
}

struct Route {
    reply: Option<mpsc::Sender<Reply>>,
    // The interpreter channel and call id a pending result settles into.
    done: Option<(Sender<Completion>, u32)>,
}

/* Delivers each runtime message to the call waiting on it or to the interpreter bound to it. */
struct Router {
    label: String,
    calls: HashMap<i128, Route>,
    events: HashMap<i128, Sender<Completion>>,
    busy: Arc<AtomicUsize>,
    last: Arc<AtomicU64>,
}

fn text(value: Option<&WireValue>) -> String {
    match value {
        Some(WireValue::Bytes(b)) => String::from_utf8_lossy(b).into_owned(),
        Some(other) => format!("{other:?}"),
        None => String::new(),
    }
}

fn int(value: Option<&WireValue>) -> i128 {
    match value {
        Some(WireValue::Int(i)) => *i,
        _ => -1,
    }
}

impl Router {
    fn route(&mut self, message: Vec<WireValue>) {
        let kind = text(message.first());
        let id = int(message.get(1));
        match kind.as_str() {
            "event" => {
                self.last.store(super::now_ns(), Ordering::SeqCst);
                if let Some(tx) = self.events.get(&id) {
                    let _ = tx.send(Completion::Event(text(message.get(2))));
                }
            }
            "busy" => {
                self.last.store(super::now_ns(), Ordering::SeqCst);
                self.busy.store(usize::try_from(id).unwrap_or(0), Ordering::SeqCst);
            }
            "pending" => {
                if let Some(tx) = self.calls.get_mut(&id).and_then(|route| route.reply.take()) {
                    let _ = tx.send(Reply::Pending);
                }
            }
            "ok" | "bound" | "settle" | "error" => {
                let Some(route) = self.calls.remove(&id) else { return };
                let outcome = match kind.as_str() {
                    "error" => Err(host_error(&self.label, &text(message.get(2)), &text(message.get(3)))),
                    "bound" => Ok(Reply::Bound(match message.get(2) {
                        Some(WireValue::List(items)) => items.iter().map(|n| text(Some(n))).collect(),
                        _ => Vec::new(),
                    })),
                    _ => Ok(Reply::Value(message.into_iter().nth(2).unwrap_or(WireValue::None))),
                };
                match (route.reply, route.done) {
                    (Some(tx), _) => {
                        let _ = tx.send(outcome.unwrap_or_else(Reply::Failed));
                    }
                    (None, Some((tx, call))) => {
                        let _ = tx.send(match outcome {
                            Ok(Reply::Value(value)) => Completion::Value { id: call, value },
                            Ok(_) => Completion::Value { id: call, value: WireValue::None },
                            Err(msg) => Completion::Error { id: call, msg },
                        });
                    }
                    (None, None) => {}
                }
            }
            _ => {}
        }
    }

    /* The runtime is gone, a pending call fails and every waiter hears its channel close. */
    fn close(&mut self, why: &str) {
        for (_, route) in self.calls.drain() {
            if let (None, Some((tx, call))) = (route.reply, route.done) {
                let _ = tx.send(Completion::Error { id: call, msg: why.to_string() });
            }
        }
        self.events.clear();
    }
}

fn lock(router: &Mutex<Router>) -> MutexGuard<'_, Router> {
    router.lock().unwrap_or_else(|e| e.into_inner())
}

// Guest stdout, token-prefixed lines are runtime messages and the rest is module console output.
#[derive(Clone)]
struct LineSink {
    prefix: Vec<u8>,
    router: Arc<Mutex<Router>>,
    printer: Printer,
}

struct LineWriter {
    sink: LineSink,
    line: Vec<u8>,
}

impl LineWriter {
    fn take(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b != b'\n' {
                self.line.push(b);
                continue;
            }
            let line = std::mem::take(&mut self.line);
            let body = line.strip_prefix(b"Log: ".as_slice()).unwrap_or(&line);
            match body.strip_prefix(self.sink.prefix.as_slice()) {
                Some(b64) => {
                    let batch = crate::pack::base64_decode(&String::from_utf8_lossy(b64)).and_then(|frame| decode(&frame));
                    if let Some(WireValue::List(messages)) = batch {
                        let mut router = lock(&self.sink.router);
                        // Pending work lands before the events of its batch, a woken receive() sees both.
                        let (busy, rest): (Vec<_>, Vec<_>) = messages.into_iter().filter_map(|m| match m {
                            WireValue::List(parts) => Some(parts),
                            _ => None,
                        }).partition(|parts| text(parts.first()) == "busy");
                        for parts in busy.into_iter().chain(rest) {
                            router.route(parts);
                        }
                    }
                }
                None => {
                    if let Ok(mut print) = self.sink.printer.lock() {
                        print(&format!("{}\n", String::from_utf8_lossy(body)));
                    }
                }
            }
        }
    }
}

fn decode(bytes: &[u8]) -> Option<WireValue> {
    let tag = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    let len = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
    WireValue::decode_body(tag, bytes.get(8..8usize.checked_add(len)?)?)
}

impl wasmtime_wasi::cli::IsTerminal for LineSink {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl wasmtime_wasi::cli::StdoutStream for LineSink {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(LineWriter { sink: self.clone(), line: Vec::new() })
    }

    // Written synchronously on the runtime thread, so a reply never overtakes the output before it.
    fn p2_stream(&self) -> Box<dyn wasmtime_wasi::p2::OutputStream> {
        Box::new(LineWriter { sink: self.clone(), line: Vec::new() })
    }
}

#[wasmtime_wasi::async_trait]
impl wasmtime_wasi::p2::Pollable for LineWriter {
    async fn ready(&mut self) {}
}

impl wasmtime_wasi::p2::OutputStream for LineWriter {
    fn write(&mut self, bytes: Bytes) -> wasmtime_wasi::p2::StreamResult<()> {
        self.take(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> wasmtime_wasi::p2::StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> wasmtime_wasi::p2::StreamResult<usize> {
        Ok(1 << 20)
    }
}

impl tokio::io::AsyncWrite for LineWriter {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        self.take(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/* One running StarlingMonkey instance on its own thread. */
struct Live {
    calls: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    router: Arc<Mutex<Router>>,
    kill: Arc<AtomicBool>,
    open: Arc<AtomicUsize>,
    busy: Arc<AtomicUsize>,
    last: Arc<AtomicU64>,
    stderr: MemoryOutputPipe,
    thread: Option<std::thread::JoinHandle<()>>,
    next: i128,
}

impl Live {
    fn send(&self, message: Vec<WireValue>) -> bool {
        let mut frame = Vec::new();
        WireValue::List(vec![WireValue::List(message)]).encode_node(&mut frame);
        self.calls.send(frame).is_ok()
    }
}

/* Called export outcome, a value at once or a Promise that settles into the interpreter later. */
pub enum Called {
    Value(WireValue),
    Pending,
}

/* What a runtime inherits from its compiler instance, the print sink and the bounds of the run. */
pub struct Scope {
    pub printer: Printer,
    pub memory: usize,
    // Untrusted runs get no outgoing network.
    pub offline: bool,
}

/* A JavaScript module's runtime, one per module per compiler instance, rebuilt after it stops. */
pub struct JsRuntime {
    label: String,
    entry: String,
    tree: Arc<Tree>,
    pre: ProxyPre<JsState>,
    engine: Engine,
    scope: Scope,
    live: Option<Live>,
    bound: HashSet<u32>,
}

impl JsRuntime {
    pub fn new(label: &str, entry: String, tree: Tree, pre: ProxyPre<JsState>, engine: Engine, scope: Scope) -> JsRuntime {
        JsRuntime { label: label.to_string(), entry, tree: Arc::new(tree), pre, engine, scope, live: None, bound: HashSet::new() }
    }

    pub fn is_bound(&self, slot: u32) -> bool {
        self.bound.contains(&slot)
    }

    /* Response bodies still open plus timers and fetches still pending, any may push another event. */
    pub fn activity(&self) -> usize {
        let Some(live) = &self.live else { return 0 };
        let pending = live.open.load(Ordering::SeqCst) + live.busy.load(Ordering::SeqCst);
        let settling = super::now_ns().saturating_sub(live.last.load(Ordering::SeqCst)) < SETTLE_NS;
        pending.max(settling as usize)
    }

    /* Binds the module's factory for one interpreter slot, the names are its exports. */
    pub fn bind(&mut self, slot: u32, events: Sender<Completion>, deadline: Option<u64>) -> Result<Vec<String>, String> {
        let (tx, rx) = mpsc::channel();
        let live = self.running()?;
        let id = live.next;
        live.next += 1;
        {
            let mut router = lock(&live.router);
            router.events.insert(slot as i128, events);
            router.calls.insert(id, Route { reply: Some(tx), done: None });
        }
        live.send(vec![s("bind"), WireValue::Int(id), WireValue::Int(slot as i128)]);
        match self.wait(&rx, deadline)? {
            Reply::Bound(names) => {
                self.bound.insert(slot);
                Ok(names)
            }
            Reply::Failed(msg) => Err(msg),
            _ => Err(format!("module '{}' answered a bind out of order", self.label)),
        }
    }

    /* Calls an export for `slot`, a pending result later settles as call `call` on `events`. */
    pub fn call(&mut self, slot: u32, name: &str, args: Vec<WireValue>, events: Sender<Completion>, call: u32, deadline: Option<u64>) -> Result<Called, String> {
        if !self.bound.contains(&slot) || self.live.is_none() {
            self.bind(slot, events.clone(), deadline)?;
        }
        let (tx, rx) = mpsc::channel();
        let live = self.running()?;
        let id = live.next;
        live.next += 1;
        lock(&live.router).calls.insert(id, Route { reply: Some(tx), done: Some((events, call)) });
        live.send(vec![s("call"), WireValue::Int(id), WireValue::Int(slot as i128), s(name), WireValue::List(args)]);
        match self.wait(&rx, deadline)? {
            Reply::Value(value) => Ok(Called::Value(value)),
            Reply::Pending => Ok(Called::Pending),
            Reply::Failed(msg) => Err(msg),
            Reply::Bound(_) => Err(format!("module '{}' answered a call out of order", self.label)),
        }
    }

    /* Forgets the factory instance of a slot that is gone. */
    pub fn unbind(&mut self, slot: u32) {
        if !self.bound.remove(&slot) {
            return;
        }
        if let Some(live) = &self.live {
            lock(&live.router).events.remove(&(slot as i128));
            live.send(vec![s("unbind"), WireValue::Int(slot as i128)]);
        }
    }

    fn running(&mut self) -> Result<&mut Live, String> {
        if self.live.is_none() {
            self.bound.clear();
            self.live = Some(self.start()?);
        }
        Ok(self.live.as_mut().expect("runtime started"))
    }

    /* Waits for the first answer, bounded by the run's deadline, a silent runtime is stopped. */
    fn wait(&mut self, rx: &mpsc::Receiver<Reply>, deadline: Option<u64>) -> Result<Reply, String> {
        let left = deadline.map_or(REPLY_LIMIT, |d| Duration::from_nanos(d.saturating_sub(super::now_ns())).min(REPLY_LIMIT));
        match rx.recv_timeout(left) {
            Ok(reply) => Ok(reply),
            Err(RecvTimeoutError::Timeout) => {
                self.stop();
                Err(format!("module '{}' did not answer within {} s", self.label, left.as_secs_f64().ceil()))
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.stopped()),
        }
    }

    /* Why the runtime ended, a failure while evaluating the module names its cause. */
    fn stopped(&mut self) -> String {
        let Some(mut live) = self.live.take() else { return format!("module '{}' stopped", self.label) };
        if let Some(thread) = live.thread.take() {
            let _ = thread.join();
        }
        let log = String::from_utf8_lossy(&live.stderr.contents()).into_owned();
        let mut lines = log.lines().skip_while(|l| !l.contains("Exception while evaluating top-level script"));
        let cause = lines.nth(1).and_then(|l| l.split_once(' ')).and_then(|(_, rest)| rest.split_once(": "));
        match cause {
            Some((name, message)) if message.ends_with(" is not defined") => host_error(&self.label, name.trim(), message.trim()),
            Some((name, message)) => format!("module '{}' failed to load, {}: {}", self.label, name.trim(), message.trim()),
            None => match log.lines().rev().find(|l| !l.trim().is_empty()) {
                Some(last) => format!("module '{}' stopped, {}", self.label, last.trim()),
                None => format!("module '{}' stopped", self.label),
            },
        }
    }

    fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            live.kill.store(true, Ordering::SeqCst);
            lock(&live.router).close(&format!("module '{}' stopped", self.label));
            drop(live.calls);
            // A guest spinning in JavaScript only checks the kill flag on an epoch change.
            self.engine.increment_epoch();
            self.engine.increment_epoch();
        }
        self.bound.clear();
    }

    fn start(&self) -> Result<Live, String> {
        let token = token();
        let root = tempfile::Builder::new().prefix("edge-js-").tempdir().map_err(|e| e.to_string())?;
        let quote = |s: &str| serde_json::to_string(s).expect("string encodes");
        let loader = LOADER.replace("__ENTRY__", &quote(&format!("./m/{}", self.entry))).replace("__TOKEN__", &quote(&token)).replace("__LABEL__", &quote(&self.label));
        let write = |rel: &str, bytes: &[u8]| -> Result<(), String> {
            let path = root.path().join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, bytes).map_err(|e| e.to_string())
        };
        write("boot.js", BOOT.as_bytes())?;
        write("loader.js", loader.as_bytes())?;
        for (rel, bytes) in self.tree.iter() {
            write(&format!("m/{rel}"), bytes)?;
        }

        let (calls, call_rx) = tokio::sync::mpsc::unbounded_channel();
        let (open, busy, kill) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let last = Arc::new(AtomicU64::new(0));
        let router = Arc::new(Mutex::new(Router { label: self.label.clone(), calls: HashMap::new(), events: HashMap::new(), busy: busy.clone(), last: last.clone() }));
        let stderr = MemoryOutputPipe::new(64 << 10);
        let sink = LineSink { prefix: format!("\u{1}{token}").into_bytes(), router: router.clone(), printer: self.scope.printer.clone() };
        let mut wasi = WasiCtxBuilder::new();
        wasi.env("STARLINGMONKEY_CONFIG", "/loader.js").stdout(sink).stderr(stderr.clone()).allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
        wasi.preopened_dir(root.path(), "/", FsPerms::ReadOnly).map_err(|e| e.to_string())?;
        let state = JsState {
            wasi: wasi.build(),
            http: WasiHttpCtx::new(),
            table: ResourceTable::new(),
            hooks: Hooks { token, calls: Some(call_rx), open: open.clone(), last: last.clone(), offline: self.scope.offline },
            limiter: MemoryCap { max: self.scope.memory.min(MAX_RUNTIME_MEMORY) },
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s: &mut JsState| &mut s.limiter as &mut dyn wasmtime::ResourceLimiter);
        store.set_epoch_deadline(1);
        let flag = kill.clone();
        store.epoch_deadline_callback(move |_| {
            if flag.load(Ordering::SeqCst) {
                return Err(wasmtime::format_err!("the JavaScript runtime was stopped"));
            }
            Ok(UpdateDeadline::Continue(1))
        });
        let proxy = self.pre.instantiate(&mut store).map_err(|e| e.to_string())?;
        let body = Full::new(Bytes::new()).map_err(|never| -> HttpError { match never {} });
        let request = http::Request::builder().uri(format!("http://{INTERNAL}/")).body(body).map_err(|e| e.to_string())?;
        let request = store.data_mut().http().new_incoming_request(Scheme::Http, request).map_err(|e| e.to_string())?;
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let response = store.data_mut().http().new_response_outparam(response_tx).map_err(|e| e.to_string())?;
        let (closing, label) = (router.clone(), self.label.clone());
        let thread = std::thread::Builder::new()
            .name(format!("js {label}"))
            .spawn(move || {
                // The tree stays on disk exactly as long as the runtime runs.
                let _root = root;
                let _response = response_rx;
                let _ = proxy.wasi_http_incoming_handler().call_handle(&mut store, request, response);
                lock(&closing).close(&format!("module '{label}' stopped"));
            })
            .map_err(|e| e.to_string())?;
        Ok(Live { calls, router, kill, open, busy, last, stderr, thread: Some(thread), next: 1 })
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn s(text: &str) -> WireValue {
    WireValue::Bytes(text.as_bytes().to_vec())
}

/* An unguessable per-runtime secret, only the generated loader carries it. */
fn token() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let mut words = [0u64; 2];
    for (i, word) in words.iter_mut().enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(i);
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        *word = hasher.finish();
    }
    format!("{:016x}{:016x}", words[0], words[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_global_names_the_web_api() {
        assert_eq!(host_error("net", "ReferenceError", "WebSocket is not defined"), "module 'net' needs 'WebSocket', missing in this runtime");
        assert_eq!(host_error("net", "TypeError", "x is not defined"), "x is not defined");
        assert_eq!(host_error("net", "ReferenceError", "a b is not defined"), "a b is not defined");
    }

    #[test]
    fn relative_imports_stay_inside_the_tree() {
        assert_eq!(imports("import a from './a.js';\nexport { b } from \"../b.js\";\nimport x from 'x';"), vec!["./a.js", "../b.js"]);
        assert_eq!(join("sub/index.js", "./util.js").as_deref(), Some("sub/util.js"));
        assert_eq!(join("index.js", "../up.js"), None);
    }
}
