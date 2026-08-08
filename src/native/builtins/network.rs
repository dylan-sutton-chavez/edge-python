use crate::packages::NativeBinding;
use crate::vm::types::{HeapPool, Val, VmErr};

use super::{int_arg, opt_str_arg, str_arg};

/* Async http and event streams over the reactor, matching the web reply shapes. */
pub(super) fn bindings() -> Vec<NativeBinding> {
    vec![
        NativeBinding::from_fn("fetch", net_fetch, false),
        NativeBinding::from_fn("fetch_text", |h, a, k| fetch_body(h, a, k, "network.fetch_text"), false),
        NativeBinding::from_fn("fetch_json", |h, a, k| fetch_body(h, a, k, "network.fetch_json"), false),
        NativeBinding::from_fn("sse_open", sse_open, false),
        NativeBinding::from_fn("sse_close", sse_close, false),
        NativeBinding::from_fn("sse_state", sse_state, false),
        NativeBinding::from_fn("ws_open", ws_open, false),
        NativeBinding::from_fn("ws_send", ws_send, false),
        NativeBinding::from_fn("ws_close", ws_close, false),
        NativeBinding::from_fn("ws_state", ws_state, false),
    ]
}

/* Async fetch, suspends the coro and resolves to the web reply shape through the reactor. */
fn net_fetch(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let url = str_arg(heap, args, 0, "network.fetch")?;
    let _options = opt_str_arg(heap, args, 1, "network.fetch")?;
    spawn_fetch(url, "network.fetch", reply_shape)
}

/* Async body-only fetch, raises on non-2xx like the web host. */
fn fetch_body(heap: &mut HeapPool, args: &[Val], _: Option<Val>, who: &'static str) -> Result<Val, VmErr> {
    let url = str_arg(heap, args, 0, who)?;
    let _options = opt_str_arg(heap, args, 1, who)?;
    spawn_fetch(url, who, |_, raw| body_only(raw))
}

// Spawns a Fetch task formatting the raw response with fmt, then defers the calling coro.
fn spawn_fetch(url: String, who: &'static str, fmt: fn(u64, &[u8]) -> Result<String, String>) -> Result<Val, VmErr> {
    let id = crate::bridge::with_vm(|vm| vm.next_host_call_id).unwrap_or(0);
    let spawned = crate::native::io::with(|rt| {
        let completions = rt.completions.clone();
        let reactor = rt.reactor.clone();
        rt.executor.spawn(async move {
            let raw = crate::native::io::fetch(url, reactor).await;
            let reply = raw.map_err(|e| e.to_string()).and_then(|bytes| fmt(id, &bytes));
            completions.borrow_mut().done.push((id, reply));
        });
    });
    if spawned.is_none() {
        return Err(VmErr::Raised(format!("RuntimeError: {who} needs the native async runtime")));
    }
    Err(VmErr::HostCallDeferred)
}

// Builds the `{id, ok, status, headers, body}` shape the web host also returns.
fn reply_shape(id: u64, raw: &[u8]) -> Result<String, String> {
    let esc = crate::devkit::escape;
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    let Ok(httparse::Status::Complete(len)) = resp.parse(raw) else {
        return Ok(format!("{{\"id\":{id},\"ok\":false,\"status\":0,\"error\":\"incomplete http response\"}}"));
    };
    let status = resp.code.unwrap_or(0);
    let ok = (200..300).contains(&status);
    let hdrs: Vec<String> = resp.headers.iter()
        .map(|h| format!("\"{}\":\"{}\"", esc(h.name), esc(&String::from_utf8_lossy(h.value))))
        .collect();
    let body = String::from_utf8_lossy(&raw[len..]);
    Ok(format!("{{\"id\":{id},\"ok\":{ok},\"status\":{status},\"headers\":{{{}}},\"body\":\"{}\"}}", hdrs.join(","), esc(&body)))
}

// Returns just the body, erroring on a non-2xx status.
fn body_only(raw: &[u8]) -> Result<String, String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    let Ok(httparse::Status::Complete(len)) = resp.parse(raw) else {
        return Err("incomplete http response".into());
    };
    let status = resp.code.unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    Ok(String::from_utf8_lossy(&raw[len..]).into_owned())
}

// Opens an event source, streaming events through receive() tagged with msg, returns a handle.
fn sse_open(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let url = str_arg(heap, args, 0, "network.sse_open")?;
    let msg = str_arg(heap, args, 1, "network.sse_open")?;
    spawn_stream("network.sse_open", |handle, reactor, sink| crate::native::io::sse(url, msg, handle, reactor, sink))
}

// Marks the stream closing, the task ends it and drops the connection.
fn sse_close(_heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let handle = int_arg(args, 0, "network.sse_close")? as usize;
    with_socket(handle, "network.sse_close", |b| { b.closing = true; b.state = 2; })
}

// Reports the readyState, 0 connecting 1 open 2 closed.
fn sse_state(_heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    Ok(Val::int(socket_state(int_arg(args, 0, "network.sse_state")? as usize, 2)))
}

// Opens a websocket, events arrive through receive() tagged with msg, returns a handle.
fn ws_open(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let url = str_arg(heap, args, 0, "network.ws_open")?;
    let msg = str_arg(heap, args, 1, "network.ws_open")?;
    spawn_stream("network.ws_open", |handle, reactor, sink| crate::native::io::ws(url, msg, handle, reactor, sink))
}

// Queues a text frame on the socket mailbox, the task encodes and sends it.
fn ws_send(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let handle = int_arg(args, 0, "network.ws_send")? as usize;
    let data = str_arg(heap, args, 1, "network.ws_send")?;
    with_socket(handle, "network.ws_send", |b| b.outgoing.push(data))
}

// Marks the socket closing, the task sends a close frame and ends the stream.
fn ws_close(_heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let handle = int_arg(args, 0, "network.ws_close")? as usize;
    with_socket(handle, "network.ws_close", |b| { b.closing = true; b.state = 2; })
}

// Reports the readyState, 0 connecting 1 open 2 closing 3 closed.
fn ws_state(_heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    Ok(Val::int(socket_state(int_arg(args, 0, "network.ws_state")? as usize, 3)))
}

// Spawns a stream task on a fresh socket slot, defers to the shared runtime for the handle.
fn spawn_stream<F, T>(who: &'static str, task: F) -> Result<Val, VmErr>
where
    F: FnOnce(usize, crate::native::io::Reactor, std::rc::Rc<std::cell::RefCell<crate::native::io::runtime::Completions>>) -> T,
    T: std::future::Future<Output = ()> + 'static,
{
    let handle = crate::native::io::with(|rt| {
        let handle = rt.completions.borrow_mut().alloc_socket();
        rt.executor.spawn(task(handle, rt.reactor.clone(), rt.completions.clone()));
        handle
    });
    match handle {
        Some(h) => Ok(Val::int(h as i64)),
        None => Err(VmErr::Raised(format!("RuntimeError: {who} needs the native async runtime"))),
    }
}

// Reads a stream's readyState, closed when the slot is gone.
fn socket_state(handle: usize, closed: i64) -> i64 {
    crate::native::io::with(|rt| {
        rt.completions.borrow().sockets.get(handle).and_then(|s| s.as_ref()).map(|b| b.state)
    })
    .flatten()
    .unwrap_or(closed)
}

// Runs f against a live socket mailbox, waking its task, errors on a stale handle.
fn with_socket(handle: usize, who: &'static str, f: impl FnOnce(&mut crate::native::io::runtime::SocketBox)) -> Result<Val, VmErr> {
    let ok = crate::native::io::with(|rt| {
        let mut c = rt.completions.borrow_mut();
        if let Some(Some(b)) = c.sockets.get_mut(handle) {
            f(b);
            if let Some(w) = b.waker.take() { w.wake(); }
            true
        } else {
            false
        }
    });
    match ok {
        Some(true) => Ok(Val::none()),
        _ => Err(VmErr::Raised(format!("RuntimeError: {who} invalid socket handle {handle}"))),
    }
}
