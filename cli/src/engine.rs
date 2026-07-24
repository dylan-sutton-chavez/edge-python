/*
The runtime engine: drive Edge Python in a headless browser, one-shot via `run` or persistent via `Session`.
*/

use anyhow::{anyhow, bail, Context, Result};
use headless_chrome::{Browser, LaunchOptions};
use serde::Deserialize;
use std::path::PathBuf;
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

/// A live session: each eval recompiles + reruns the accumulated history so imports/defs persist.
pub struct Session {
    _browser: Browser,
    tab: Arc<headless_chrome::Tab>,
    history: String,
    history_lines: usize,
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
        Ok(Self { _browser: browser, tab, history: String::new(), history_lines: 0 })
    }

    /// Recompile prior history with `src` so imports and defs persist; only new lines reach `on_line`.
    pub fn eval<F: FnMut(&str)>(&mut self, src: &str, mut on_line: F) -> Result<Outcome> {
        let full = if self.history.is_empty() {
            src.to_string()
        } else {
            format!("{}\n{}", self.history, src)
        };
        let literal = serde_json::to_string(&full)?;
        let expr = format!("__edgeRun({literal})");
        self.tab.evaluate(&expr, false).map_err(|e| anyhow!("starting eval: {e}"))?;

        let mut all: Vec<String> = Vec::new();
        let outcome = drain(&self.tab, &mut |line| all.push(line.to_string()))?;
        for line in all.iter().skip(self.history_lines) {
            on_line(line);
        }
        if outcome.err.is_none() {
            if !self.history.is_empty() { self.history.push('\n'); }
            self.history.push_str(src);
            self.history_lines = all.len();
        }
        Ok(outcome)
    }

    /// Wipe accumulated history and runtime modules; next eval starts in a fresh namespace.
    pub fn reset(&mut self) -> Result<()> {
        self.history.clear();
        self.history_lines = 0;
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
pub fn run(src: &str, manifest: &Manifest) -> Result<i32> {
    let mut session = Session::open(manifest)?;
    let outcome = session.eval(src, emit_chunk)?;
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

/// Prefer env override, then any Chrome on PATH, then a Playwright-cached Chromium.
fn resolve_chrome() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("EDGE_CHROME_PATH") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(p) = headless_chrome::browser::default_executable() {
        return Ok(p);
    }
    if let Some(p) = playwright_chrome() {
        return Ok(p);
    }
    bail!("no Chrome/Chromium found; re-run install.sh or set EDGE_CHROME_PATH");
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

/// Serve the harness at `/` and the manifest at `/packages.json` on a free loopback port. The thread is a daemon.
fn serve(packages: String) -> Result<u16> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("starting local server: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("local server has no TCP address"))?
        .port();

    thread::spawn(move || {
        for req in server.incoming_requests() {
            let path = req.url().split('?').next().unwrap_or("/");
            let resp = match path {
                "/" => Response::from_string(harness_page()).with_header(ctype("text/html; charset=utf-8")),
                "/packages.json" => Response::from_string(packages.clone()).with_header(ctype("application/json")),
                _ => Response::from_string("not found").with_status_code(404),
            };
            let _ = req.respond(resp);
        }
    });
    Ok(port)
}

/* Test hook: EDGE_FAKE_CONTRACT lands after element.js, so the fake wins. */
fn harness_page() -> String {
    match std::env::var("EDGE_FAKE_CONTRACT") {
        Ok(v) => {
            // Non-numeric sentinels inject as strings, still unequal to any contract.
            let lit = if v.parse::<i64>().is_ok() { v } else { format!("{v:?}") };
            HARNESS.replacen("const CONTRACT", &format!("window.__edgeContract = {lit}; const CONTRACT"), 1)
        }
        Err(_) => HARNESS.to_string(),
    }
}

fn ctype(value: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).expect("static header is valid")
}
