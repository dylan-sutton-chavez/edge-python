/*
The runtime engine: drive Edge Python in a headless browser, one-shot via `run` or persistent via `Session`.
*/

use anyhow::{anyhow, bail, Context, Result};
use headless_chrome::{Browser, LaunchOptions};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server};

use crate::pkg::Manifest;

// Harness exposes `window.__edgeRun(src)` and `window.__edgeReady` once the worker boots.
const HARNESS: &str = include_str!("templates/harness.html");

// The runtime is async, so we poll state instead of blocking on one CDP call.
const POLL_JS: &str = "window.__edge ? JSON.stringify(window.__edge) : ''";
const READY_JS: &str = "!!window.__edgeReady";
const MISMATCH_JS: &str = "window.__edgeMismatch || ''";

// Hard ceiling so a hung script or a failed CDN fetch can't wedge the CLI.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
const EVAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize)]
struct State {
    lines: Vec<String>,
    done: bool,
    ok: bool,
    #[serde(default)]
    err: String,
    // Set when the script raised `SystemExit`; carries its exit code.
    #[serde(default)]
    code: Option<i32>,
}

/// Result of one eval: streamed lines went out via the `on_line` callback; only the error and exit code survive.
pub struct Outcome {
    pub err: Option<String>,
    pub exit_code: Option<i32>,
}

/// A live session: the worker keeps the interpreter alive, so each eval runs its input exactly once.
pub struct Session {
    _browser: Browser,
    tab: Arc<headless_chrome::Tab>,
}

impl Session {
    /// Boot the harness page and wait until the worker is ready to receive eval calls.
    pub fn open(manifest: &Manifest) -> Result<Self> {
        let packages = serde_json::to_string(manifest)?;
        let port = serve(packages)?;
        let url = format!("http://127.0.0.1:{port}/");

        // One spinner covers the whole bring-up: Chromium spawn + page load + worker boot.
        let _sp = crate::ui::spinner("starting chromium");
        let browser = launch().context("launching headless Chromium")?;
        let tab = browser.new_tab().map_err(|e| anyhow!("opening a tab: {e}"))?;
        tab.navigate_to(&url).map_err(|e| anyhow!("navigating to the harness: {e}"))?;
        tab.wait_until_navigated().map_err(|e| anyhow!("waiting for page load: {e}"))?;
        wait_ready(&tab)?;
        Ok(Self { _browser: browser, tab })
    }

    /// Run one input; state persists in the worker's interpreter between evals.
    /// `base` repositions relative imports; None means the project root.
    pub fn eval<F: FnMut(&str)>(&mut self, src: &str, base: Option<&str>, mut on_line: F) -> Result<Outcome> {
        let literal = serde_json::to_string(src)?;
        let base = serde_json::to_string(&base)?;
        let expr = format!("__edgeRun({literal}, {base})");
        self.tab.evaluate(&expr, false).map_err(|e| anyhow!("starting eval: {e}"))?;
        drain(&self.tab, &mut |line| on_line(line))
    }

    /// Wipe the runtime's interpreter and modules; next eval starts in a fresh namespace.
    pub fn reset(&mut self) -> Result<()> {
        self.tab.evaluate("__edgeReset()", false).map_err(|e| anyhow!("resetting runtime: {e}"))?;
        Ok(())
    }
}

/// Write a raw stdout chunk (byte-stream: it already carries its own `end`) and flush so streaming output appears at once.
pub fn emit_chunk(chunk: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(chunk.as_bytes());
    let _ = out.flush();
}

/// One-shot: open a session, eval `src`, stream stdout, tear down. Returns the process exit code (0 clean, 1 on error, or the script's `SystemExit` code).
pub fn run(src: &str, manifest: &Manifest, base: Option<&str>) -> Result<i32> {
    let mut session = Session::open(manifest)?;
    let outcome = session.eval(src, base, emit_chunk)?;
    if let Some(err) = outcome.err {
        crate::ui::traceback(&err);
        return Ok(1);
    }
    Ok(outcome.exit_code.unwrap_or(0))
}

/// Launch headless Chromium against a system-installed browser. install.sh provisions it.
fn launch() -> Result<Browser> {
    let mut builder = LaunchOptions::default_builder();
    builder.sandbox(false); // headless under WSL/containers typically can't sandbox
    // Default is 30s and the REPL would drop CDP whenever the user stopped to think.
    builder.idle_browser_timeout(Duration::from_secs(60 * 60 * 24));
    builder.path(Some(resolve_chrome()?));
    let options = builder.build().map_err(|e| anyhow!("building launch options: {e}"))?;
    Browser::new(options).map_err(|e| anyhow!("{e}"))
}

/// Env override, bundled shell, system Chrome, Playwright.
fn resolve_chrome() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("EDGE_CHROME_PATH") {
        return Ok(PathBuf::from(p));
    }
    if let Some(p) = bundled_chrome() {
        return Ok(p);
    }
    if let Ok(p) = headless_chrome::browser::default_executable() {
        return Ok(p);
    }
    if let Some(p) = playwright_chrome() {
        return Ok(p);
    }
    bail!("no Chrome/Chromium found; re-run install.sh or set EDGE_CHROME_PATH");
}

/// Bundled chrome-headless-shell that install.sh downloads.
fn bundled_chrome() -> Option<PathBuf> {
    let root = match std::env::var_os("EDGE_CHROME_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".cache/edge"),
    };
    bundled_chrome_in(&root)
}

fn bundled_chrome_in(root: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if !name.starts_with("chrome-headless-shell-") { continue; }
        let candidate = entry.path().join("chrome-headless-shell");
        if candidate.is_file() { return Some(candidate); }
    }
    None
}

/// Best-effort lookup of a Playwright-installed Chromium under `~/.cache/ms-playwright/chromium-*/chrome-linux/chrome`.
fn playwright_chrome() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("HOME")?).join(".cache/ms-playwright");
    let mut best: Option<PathBuf> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if !name.starts_with("chromium-") { continue; }
        let candidate = entry.path().join("chrome-linux/chrome");
        if candidate.is_file() && best.as_ref().is_none_or(|b| candidate > *b) {
            best = Some(candidate);
        }
    }
    best
}

/// Block until the harness has set `window.__edgeReady = true` (worker created, ready for evals).
fn wait_ready(tab: &headless_chrome::Tab) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            bail!(
                "timed out after {}s waiting for the runtime to load\nhelp: the installed CLI may be out of sync with the CDN runtime; re-run install.sh (curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh)",
                READY_TIMEOUT.as_secs()
            );
        }
        let raw = tab.evaluate(READY_JS, false).map_err(|e| anyhow!("polling runtime ready: {e}"))?;
        if raw.value.as_ref().and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        // Mismatch is terminal, not a timeout.
        let mm = tab.evaluate(MISMATCH_JS, false).map_err(|e| anyhow!("polling runtime ready: {e}"))?;
        if let Some(msg) = mm.value.as_ref().and_then(|v| v.as_str())
            && !msg.is_empty()
        {
            bail!("{msg}");
        }
        thread::sleep(Duration::from_millis(60));
    }
}

/// Poll the page, stream each new raw stdout chunk, and resolve when the current eval finishes.
fn drain<F: FnMut(&str)>(tab: &headless_chrome::Tab, on_line: &mut F) -> Result<Outcome> {
    let mut printed = 0usize;
    let deadline = Instant::now() + EVAL_TIMEOUT;

    loop {
        if Instant::now() > deadline {
            bail!("timed out after {}s waiting for the script", EVAL_TIMEOUT.as_secs());
        }
        let raw = tab.evaluate(POLL_JS, false).map_err(|e| anyhow!("reading page state: {e}"))?;
        let json = raw.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
        if json.is_empty() {
            thread::sleep(Duration::from_millis(60));
            continue;
        }
        let state: State = serde_json::from_str(json).context("parsing page state")?;
        for line in state.lines.iter().skip(printed) {
            on_line(line);
        }
        printed = state.lines.len();
        if state.done {
            return Ok(Outcome { err: if state.ok { None } else { Some(state.err) }, exit_code: state.code });
        }
        thread::sleep(Duration::from_millis(60));
    }
}

/// Serve the harness at `/`, the manifest at `/packages.json`, and project files on a free loopback port. The thread is a daemon.
fn serve(packages: String) -> Result<u16> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("starting local server: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("local server has no TCP address"))?
        .port();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    thread::spawn(move || {
        for req in server.incoming_requests() {
            let path = req.url().split('?').next().unwrap_or("/");
            let resp = match path {
                "/" => Response::from_string(harness_page()).with_header(ctype("text/html; charset=utf-8")),
                "/packages.json" => Response::from_string(packages.clone()).with_header(ctype("application/json")),
                _ => match local_stack_file(path).or_else(|| project_file(&root, path)) {
                    Some((body, mime)) => Response::from_data(body).with_header(ctype(mime)),
                    None => Response::from_string("not found").with_status_code(404),
                },
            };
            let _ = req.respond(resp);
        }
    });
    Ok(port)
}

/// Read a project file so the worker can import it.
fn project_file(root: &Path, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.split('/').any(|seg| seg == "..") { return None; }
    let full = root.join(rel);
    Some((std::fs::read(&full).ok()?, crate::serve::content_type(&full)))
}

/// Directory of `file` as an eval base, when inside the project.
pub fn base_dir(file: &Path) -> Option<String> {
    let parent = file.parent()?.to_str()?;
    // A leading ./ would fork the spec-space with phantom dirs.
    let parent = parent.trim_start_matches("./");
    if parent.is_empty() || parent == "." || parent.starts_with("..") || parent.starts_with('/') { return None; }
    Some(format!("{parent}/"))
}

/* Test hooks: EDGE_FAKE_CONTRACT lands after element.js, so the fake wins; EDGE_RUNTIME_DIR swaps the CDN runtime and compiler for locally served copies. */
fn harness_page() -> String {
    let mut page = HARNESS.to_string();
    if let Ok(v) = std::env::var("EDGE_FAKE_CONTRACT") {
        // Non-numeric sentinels inject as strings, still unequal to any contract.
        let lit = if v.parse::<i64>().is_ok() { v } else { format!("{v:?}") };
        page = page.replacen("const CONTRACT", &format!("window.__edgeContract = {lit}; const CONTRACT"), 1);
    }
    if std::env::var("EDGE_RUNTIME_DIR").is_ok() {
        page = page
            .replacen("https://cdn.edgepython.com/runtime/src/element.js", "/runtime/src/element.js", 1)
            // The worker resolves no relative URLs, so the wasm override must be absolute.
            .replacen(
                "<script type=\"module\">",
                "<script>document.getElementById(\"ep\").setAttribute(\"wasm\", location.origin + \"/compiler.wasm\");</script><script type=\"module\">",
                1,
            );
    }
    page
}

/* Test hook: with EDGE_RUNTIME_DIR set, the harness swaps the CDN for these local routes. */
fn local_stack_file(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let dir = std::env::var("EDGE_RUNTIME_DIR").ok()?;
    if path == "/compiler.wasm" {
        let wasm = std::env::var("EDGE_COMPILER_WASM").ok()?;
        return Some((std::fs::read(wasm).ok()?, "application/wasm"));
    }
    let rel = path.strip_prefix("/runtime/")?;
    if rel.contains("..") { return None; }
    let body = std::fs::read(std::path::Path::new(&dir).join(rel)).ok()?;
    let mime = if rel.ends_with(".js") { "text/javascript" } else { "application/octet-stream" };
    Some((body, mime))
}

fn ctype(value: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).expect("static header is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_probe_finds_headless_shell() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("chrome-headless-shell-linux64/chrome-headless-shell");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "").unwrap();
        assert_eq!(bundled_chrome_in(dir.path()), Some(bin));
        assert_eq!(bundled_chrome_in(&dir.path().join("missing")), None);
    }
}
