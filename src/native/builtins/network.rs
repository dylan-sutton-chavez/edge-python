use crate::packages::NativeBinding;
use crate::vm::types::{HeapObj, HeapPool, Val, VmErr};
use std::io::Read;
use std::sync::atomic::{AtomicI64, Ordering};

use super::{opt_str_arg, str_arg};

// Bounds a runaway response body, mirrors the resolver's download cap.
const MAX_BODY_BYTES: u64 = 64 << 20;

/* Blocking http with the web reply shapes, sockets and streams stay web-only. */
pub(super) fn bindings() -> Vec<NativeBinding> {
    vec![
        NativeBinding::from_fn("fetch", net_fetch, false),
        NativeBinding::from_fn("fetch_text", |h, a, k| fetch_body(h, a, k, "network.fetch_text"), false),
        NativeBinding::from_fn("fetch_json", |h, a, k| fetch_body(h, a, k, "network.fetch_json"), false),
    ]
}

struct Options {
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

/* RequestInit subset from the options json, method plus headers plus string body. */
fn parse_options(json: Option<String>, who: &'static str) -> Result<Options, VmErr> {
    let mut opts = Options { method: "GET".into(), headers: Vec::new(), body: None };
    let Some(json) = json else { return Ok(opts) };
    if json.trim().is_empty() {
        return Ok(opts);
    }
    let pairs = crate::devkit::parse_object(&json)
        .map_err(|e| VmErr::Raised(format!("RuntimeError: {who} options, {e}")))?;
    for (k, v) in pairs {
        match (k.as_str(), v) {
            ("method", crate::devkit::JsonVal::Str(m)) => opts.method = m,
            ("body", crate::devkit::JsonVal::Str(b)) => opts.body = Some(b),
            ("headers", crate::devkit::JsonVal::Map(h)) => opts.headers = h,
            _ => {}
        }
    }
    Ok(opts)
}

/* One status-tolerant agent, error replies keep their bodies and callers judge the code. */
fn send(url: &str, opts: &Options) -> Result<ureq::http::Response<ureq::Body>, Box<ureq::Error>> {
    let run = || -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let mut b = ureq::http::Request::builder().method(opts.method.as_str()).uri(url);
        for (k, v) in &opts.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let req = b.body(opts.body.clone().unwrap_or_default())?;
        agent.run(req)
    };
    run().map_err(Box::new)
}

fn read_body(resp: &mut ureq::http::Response<ureq::Body>) -> Result<String, VmErr> {
    let mut text = String::new();
    resp.body_mut().as_reader().take(MAX_BODY_BYTES).read_to_string(&mut text)
        .map_err(|e| VmErr::Raised(format!("RuntimeError: network body, {e}")))?;
    Ok(text)
}

/* Web reply shape, `{id, ok, status, headers, body}`, an `error` field replaces them on transport failure. */
fn net_fetch(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    static NEXT_ID: AtomicI64 = AtomicI64::new(0);
    let url = str_arg(heap, args, 0, "network.fetch")?;
    let options = opt_str_arg(heap, args, 1, "network.fetch")?;
    let opts = parse_options(options, "network.fetch")?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let esc = crate::devkit::escape;
    let json = match send(&url, &opts) {
        Ok(mut resp) => {
            let (ok, status) = (resp.status().is_success(), resp.status().as_u16());
            let headers: Vec<String> = resp.headers().iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", esc(k.as_str()), esc(v.to_str().unwrap_or(""))))
                .collect();
            let body = read_body(&mut resp)?;
            format!("{{\"id\":{id},\"ok\":{ok},\"status\":{status},\"headers\":{{{}}},\"body\":\"{}\"}}", headers.join(","), esc(&body))
        }
        Err(e) => format!("{{\"id\":{id},\"ok\":false,\"status\":0,\"error\":\"{}\"}}", esc(&e.to_string())),
    };
    heap.alloc(HeapObj::Str(json))
}

/* Body-only fetch, raises on non-2xx like the web host. */
fn fetch_body(heap: &mut HeapPool, args: &[Val], _: Option<Val>, who: &'static str) -> Result<Val, VmErr> {
    let url = str_arg(heap, args, 0, who)?;
    let options = opt_str_arg(heap, args, 1, who)?;
    let opts = parse_options(options, who)?;
    let mut resp = send(&url, &opts)
        .map_err(|e| VmErr::Raised(format!("RuntimeError: {who} {url}, {e}")))?;
    if !resp.status().is_success() {
        return Err(VmErr::Raised(format!("RuntimeError: HTTP {}", resp.status().as_u16())));
    }
    let body = read_body(&mut resp)?;
    heap.alloc(HeapObj::Str(body))
}
