/*
`edge serve`: static dev server with live reload. Sync, on tiny_http; reloads the page on any file change.
*/

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tiny_http::{Header, Response, Server};

pub fn run(dir: PathBuf, host: &str, port: u16, open: bool) -> Result<()> {
    let server = Server::http((host, port)).map_err(|e| anyhow!("could not bind {host}:{port}: {e}"))?;

    // Bumped by the watcher; the injected client polls it and reloads on change.
    let version = Arc::new(AtomicU64::new(0));
    spawn_watcher(dir.clone(), version.clone());

    // A LAN bind also advertises the network URL.
    let lan = match host {
        "127.0.0.1" | "localhost" => None,
        "0.0.0.0" => lan_ip(),
        h => Some(h.to_string()),
    };
    crate::ui::serve_banner(port, &dir, lan.as_deref());
    if open {
        let _ = open_url(&format!("http://localhost:{port}"));
    }

    for req in server.incoming_requests() {
        let url = req.url().split('?').next().unwrap_or("/").to_string();
        if url == "/__livereload" {
            let _ = req.respond(Response::from_string(version.load(Ordering::Relaxed).to_string()));
        } else {
            serve_file(req, &dir, &url);
        }
    }
    Ok(())
}

fn serve_file(req: tiny_http::Request, dir: &Path, url: &str) {
    let rel = url.trim_start_matches('/');
    let mut path = dir.join(rel);
    if rel.is_empty() || path.is_dir() {
        path = path.join("index.html");
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = content_type(&path);
            let resp = if ct == "text/html" {
                let html = inject_livereload(&String::from_utf8_lossy(&bytes));
                Response::from_string(html).with_header(header("Content-Type", ct))
            } else {
                Response::from_data(bytes).with_header(header("Content-Type", ct))
            };
            let _ = req.respond(resp);
        }
        Err(_) => {
            let _ = req.respond(Response::from_string("404 not found").with_status_code(404));
        }
    }
}

pub(crate) fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("js") | Some("mjs") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("css") => "text/css",
        Some("py") => "text/plain",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn header(key: &str, value: &str) -> Header {
    Header::from_bytes(key.as_bytes(), value.as_bytes()).expect("static header is valid")
}

const LIVERELOAD: &str = r#"<script>
let __v;
setInterval(async () => {
  try {
    const n = await (await fetch("/__livereload")).text();
    if (__v === undefined) __v = n;
    else if (n !== __v) location.reload();
  } catch {}
}, 500);
</script>"#;

fn inject_livereload(html: &str) -> String {
    match html.rfind("</body>") {
        Some(i) => format!("{}{}{}", &html[..i], LIVERELOAD, &html[i..]),
        None => format!("{html}{LIVERELOAD}"),
    }
}

/// Outbound interface IP via a connected UDP probe; no packets are sent.
fn lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Bump `version` whenever any file under `dir` changes. Mtime poll, no watcher dependency.
fn spawn_watcher(dir: PathBuf, version: Arc<AtomicU64>) {
    std::thread::spawn(move || {
        let mut last = fingerprint(&dir);
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let now = fingerprint(&dir);
            if now != last {
                last = now;
                version.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Cheap directory fingerprint: a rolling sum of file mtimes and sizes.
fn fingerprint(dir: &Path) -> u64 {
    fn walk(dir: &Path, acc: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, acc);
            } else if let Ok(meta) = entry.metadata() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                *acc = acc.wrapping_add(mtime).wrapping_add(meta.len());
            }
        }
    }
    let mut acc = 0;
    walk(dir, &mut acc);
    acc
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ())
}

#[cfg(target_os = "windows")]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd").args(["/C", "start", url]).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
}
