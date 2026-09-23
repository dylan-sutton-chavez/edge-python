use anyhow::{anyhow, bail, Context, Result};
use headless_chrome::{Browser, FetcherOptions, LaunchOptions};
use serde::Deserialize;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server};

use crate::cmd::serve::content_type;
use crate::web::{COMPILER_WASM, JS_HOST};

const HARNESS: &str = include_str!("../templates/harness.html");

// What the page publishes, read rather than awaited so a long run streams instead of blocking one call.
const POLL_JS: &str = "window.__edge ? JSON.stringify(window.__edge) : ''";

const POLL_EVERY: Duration = Duration::from_millis(60);

// A wedged page cannot hold the CLI forever, and the engine's own budget is what bounds a hung script.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize)]
struct State {
    out: String,
    done: bool,
    ok: bool,
    #[serde(default)]
    code: i32,
    #[serde(default)]
    err: String,
}

/// Run `src` on the browser host and return the code it exited with.
pub fn run(src: &str, manifest: Option<&Path>) -> Result<i32> {
    let page = HARNESS.replace("__EDGE_SRC__", &embed(src)?);
    // The declared modules keep their own addresses, only the host and the engine come from here.
    let imports = match manifest {
        Some(path) if path.exists() => std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        _ => b"{}".to_vec(),
    };

    let port = serve(page, imports)?;
    let browser = launch().context("launching headless Chromium")?;
    let tab = browser.new_tab().map_err(|e| anyhow!("opening a tab: {e}"))?;

    tab.navigate_to(&format!("http://127.0.0.1:{port}/")).map_err(|e| anyhow!("navigating to the harness: {e}"))?;
    tab.wait_until_navigated().map_err(|e| anyhow!("waiting for page load: {e}"))?;

    drain(&tab)
}

/* The source as a JS string literal. JSON leaves `<` alone, so a script carrying a closing script tag would end the harness block and run as markup, and escaping it keeps the program's own text intact. */
fn embed(src: &str) -> Result<String> {
    Ok(serde_json::to_string(src)?.replace('<', "\\u003c"))
}

/// Where a browser edge downloaded lives, which `edge uninstall` offers to remove.
pub fn chrome_dir() -> Result<PathBuf> {
    crate::host::data_root().map(|root| root.join("chromium")).map_err(|e| anyhow!("{e}"))
}

/// The browser to drive, `None` when the launcher should fetch one into `chrome_dir`.
pub fn resolve_chrome() -> Result<Option<PathBuf>> {
    if let Some(path) = std::env::var_os("EDGE_CHROME_PATH") {
        return Ok(Some(PathBuf::from(path)));
    }
    if let Ok(found) = headless_chrome::browser::default_executable() {
        return Ok(Some(found));
    }
    // A past download is already in that dir, and the launcher reads it before reaching for the network.
    if fetched()? {
        return Ok(None);
    }
    match agreed()? {
        true => Ok(None),
        false => bail!("--web needs Chrome or Chromium, install one or set EDGE_CHROME_PATH to a binary you have")
    }
}

/// True when edge has already downloaded a browser.
pub fn fetched() -> Result<bool> {
    let dir = chrome_dir()?;
    Ok(std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_some()))
}

/* Every download lands in one dir edge owns, never a shared one, so removing it takes nothing else with it. */
fn fetching() -> Result<FetcherOptions> {
    Ok(FetcherOptions::default()
        .with_install_dir(Some(chrome_dir()?))
        .with_allow_standard_dirs(false)
        .with_allow_download(true))
}

/* Asks once before downloading a browser, and refuses outright without a terminal so a pipeline never hangs on an answer nobody is there to give. */
fn agreed() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "--web needs Chrome or Chromium and found none\n\
             help: install one, set EDGE_CHROME_PATH, or run this in a terminal to be offered a download"
        );
    }

    print!("No headless Chromium found. Download one? [y/N] ");
    std::io::stdout().flush().ok();

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).map_err(|e| anyhow!("reading the answer: {e}"))?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes" | "YES"))
}

/* Headless with the sandbox off, since a container or WSL usually cannot open one and the engine's own limits are what bound the script. */
fn launch() -> Result<Browser> {
    let found = resolve_chrome()?;
    if found.is_none() && !fetched()? {
        println!("downloading a headless Chromium into {}, this happens once", chrome_dir()?.display());
    }

    let mut builder = LaunchOptions::default_builder();
    builder.sandbox(false).path(found).fetcher_options(fetching()?);
    let options = builder.build().map_err(|e| anyhow!("building launch options: {e}"))?;
    Browser::new(options).map_err(|e| anyhow!("{e}"))
}

/* Streams whatever the page has printed since the last read, then reports how the run ended. */
fn drain(tab: &headless_chrome::Tab) -> Result<i32> {
    let stdout = std::io::stdout();
    let mut written = 0usize;
    let deadline = Instant::now() + RUN_TIMEOUT;

    loop {
        if Instant::now() > deadline {
            bail!("timed out after {}s waiting for the page", RUN_TIMEOUT.as_secs());
        }

        let raw = tab.evaluate(POLL_JS, false).map_err(|e| anyhow!("reading page state: {e}"))?;
        let json = raw.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
        if json.is_empty() {
            thread::sleep(POLL_EVERY);
            continue;
        }

        let state: State = serde_json::from_str(json).context("parsing page state")?;
        if let Some(fresh) = state.out.get(written..) {
            let _ = write!(stdout.lock(), "{fresh}");
            let _ = stdout.lock().flush();
            written = state.out.len();
        }

        if state.done {
            if !state.ok {
                crate::ui::traceback(&state.err);
                return Ok(if state.code == 0 { 1 } else { state.code });
            }
            return Ok(state.code);
        }
        thread::sleep(POLL_EVERY);
    }
}

/* Serves the harness, the project's manifest and the embedded host on a loopback port, so nothing but the declared modules leaves the machine. */
fn serve(page: String, imports: Vec<u8>) -> Result<u16> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("starting the local server: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("the local server has no TCP address"))?
        .port();

    thread::spawn(move || {
        for req in server.incoming_requests() {
            let path = req.url().split('?').next().unwrap_or("/").trim_start_matches('/').to_string();
            let served = match path.as_str() {
                "" => Some((page.as_bytes().to_vec(), "text/html")),
                "edge.json" => Some((imports.clone(), content_type(Path::new("edge.json")))),
                "compiler.wasm" => Some((COMPILER_WASM.to_vec(), content_type(Path::new("compiler.wasm")))),
                _ => host_file(&path),
            };
            let _ = match served {
                Some((bytes, kind)) => req.respond(Response::from_data(bytes).with_header(header(kind))),
                None => req.respond(Response::from_string("not found").with_status_code(404)),
            };
        }
    });

    Ok(port)
}

/* The embedded host answers under the same `js/` prefix a dist and the CDN use, so the harness imports read the same everywhere. */
fn host_file(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.strip_prefix("js/")?;
    let (_, bytes) = JS_HOST.iter().find(|(key, _)| *key == rel)?;
    Some((bytes.to_vec(), content_type(Path::new(rel))))
}

fn header(value: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).expect("a static header is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The override answers before anything is searched for, so a build farm can pin one browser and never be asked.
    #[test]
    fn an_explicit_browser_wins() {
        // SAFETY: single-threaded test, nothing else reads the environment here.
        unsafe { std::env::set_var("EDGE_CHROME_PATH", "/tmp/not-a-browser") };
        assert_eq!(resolve_chrome().unwrap(), Some(PathBuf::from("/tmp/not-a-browser")));
        assert!(chrome_dir().unwrap().ends_with("chromium"));
        unsafe { std::env::remove_var("EDGE_CHROME_PATH") };
    }

    /* The source crosses into the page as a JSON string, so a script carrying a closing script tag or a quote cannot break out of the harness. */
    #[test]
    fn the_harness_cannot_be_escaped_by_a_script() {
        let hostile = "print('</script><script>alert(1)</script>')";
        let page = HARNESS.replace("__EDGE_SRC__", &embed(hostile).unwrap());

        assert!(!page.contains("<script>alert(1)"));
        assert!(page.contains("\\u003c/script"));
        // The program still reads as it was written once the page decodes the literal.
        assert_eq!(serde_json::from_str::<String>(&embed(hostile).unwrap()).unwrap(), hostile);
    }

    #[test]
    fn the_host_answers_under_the_js_prefix() {
        assert!(host_file("js/src/element.js").is_some());
        assert!(host_file("js/src/nope.js").is_none());
        assert!(host_file("src/element.js").is_none());
    }
}
