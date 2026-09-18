use super::{opt_str_arg, str_arg, text};
use compiler::abi::WireValue;
use compiler::devkit::escape;
use std::sync::atomic::{AtomicU64, Ordering};

/* Http from scripts, every export defers to a worker thread and answers like the browser module. */
pub const EXPORTS: [(&str, bool); 3] = [("fetch", true), ("fetch_text", true), ("fetch_json", true)];

// Request ids, the browser shape carries one even though nothing aborts here.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

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
                    let headers: Vec<String> = r.headers.iter().map(|(k, v)| format!("\"{}\":\"{}\"", escape(k), escape(v))).collect();
                    format!("{{\"id\":{id},\"ok\":{ok},\"status\":{},\"headers\":{{{}}},\"body\":\"{}\"}}", r.status, headers.join(","), escape(&r.body))
                }
                Err(e) => format!("{{\"id\":{id},\"ok\":false,\"status\":0,\"error\":\"{}\"}}", escape(&e)),
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
    let mut resp = agent.run(req).map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let headers = resp.headers().iter().map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(Reply { status, headers, body })
}
