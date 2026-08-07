/*
`edge build`: vendor the runtime, compiler.wasm, used packages, and user scripts into a self-contained dist/.
*/

use anyhow::{anyhow, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::manifest::{Kind, Manifest};

// Production layout we mirror into dist/web/ and dist/.
const RUNTIME_BASE: &str = "https://cdn.edgepython.com/web/";
const COMPILER_WASM: &str = "https://cdn.edgepython.com/compiler.wasm";
const RUNTIME_FILES: &[&str] = &[
    "src/index.js",
    "src/element.js",
    "src/env.js",
    "src/fetch.js",
    "src/native.js",
    "src/prefetch.js",
    "src/rt.js",
    "src/specs.js",
    "src/defaults.js",
    "src/cache/idb.js",
    "src/cache/memory.js",
    "src/worker/worker.js",
    "src/worker/engine.js",
];

const INDEX_HTML: &str = include_str!("../templates/dist.html");

pub fn run(manifest_path: &Path, out_dir: PathBuf) -> Result<()> {
    let t0 = Instant::now();
    let manifest = Manifest::load(manifest_path)?;
    // `Path::parent` returns Some("") for a bare filename, so collapse that to "." explicitly.
    let project = match manifest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let sp = crate::ui::spinner("vendoring runtime");
    match vendor_runtime(&out_dir) {
        Ok(()) => sp.done("vendored runtime"),
        Err(e) => { sp.fail("failed to vendor runtime"); return Err(e); }
    }

    let sp = crate::ui::spinner("fetching compiler.wasm");
    // Test hook: local compiler instead of CDN.
    let compiler_result = match std::env::var("EDGE_COMPILER_WASM") {
        Ok(p) => fs::read(&p).with_context(|| format!("reading {p}")),
        Err(_) => fetch(COMPILER_WASM).context("fetching compiler.wasm"),
    };
    let compiler_bytes = match compiler_result {
        Ok(b) => b,
        Err(e) => { sp.fail("failed to fetch compiler.wasm"); return Err(e); }
    };
    fs::write(out_dir.join("compiler.wasm"), &compiler_bytes)?;
    sp.done("fetched compiler.wasm");

    let scripts = collect_scripts(&project, &out_dir);
    let imports = crawl_imports(&scripts);
    let sp = crate::ui::spinner("vendoring packages");
    let (vendored_imports, vendored_host) = match vendor_packages(&manifest, &imports, &out_dir) {
        Ok(v) => v,
        Err(e) => { sp.fail("failed to vendor packages"); return Err(e); }
    };
    sp.done("vendored packages");
    let script_count = copy_scripts(&scripts, &project, &out_dir)?;

    let rewritten = rewrite_manifest(&manifest, &vendored_imports, &vendored_host);
    let pretty = serde_json::to_string_pretty(&rewritten)?;
    fs::write(out_dir.join("packages.json"), format!("{pretty}\n"))?;

    let entry = find_entry(&scripts, &project);
    fs::write(out_dir.join("index.html"), index_html(&entry))?;

    crate::ui::build_report(
        &out_dir,
        RUNTIME_FILES.len(),
        vendored_imports.len() + vendored_host.len(),
        script_count,
        dir_size(&out_dir)?,
        t0.elapsed(),
    );
    Ok(())
}

/// Fetch the runtime JS modules into `dist/web/` mirroring their CDN layout.
fn vendor_runtime(out_dir: &Path) -> Result<()> {
    // Test hook: local runtime instead of CDN.
    let local = std::env::var("EDGE_RUNTIME_DIR").ok();
    for rel in RUNTIME_FILES {
        let bytes = match &local {
            Some(dir) => {
                let path = Path::new(dir).join(rel);
                fs::read(&path).with_context(|| format!("reading {}", path.display()))?
            }
            None => {
                let url = format!("{RUNTIME_BASE}{rel}");
                fetch(&url).with_context(|| format!("fetching {url}"))?
            }
        };
        let path = out_dir.join("web").join(rel);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(&path, bytes)?;
    }
    Ok(())
}

/// Walk the project for `.py` files; skip hidden dirs and the output directory itself.
fn collect_scripts(project: &Path, out_dir: &Path) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    walk(project, out_dir, &mut scripts);
    scripts
}

fn walk(dir: &Path, out_dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == out_dir { continue; }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if path.is_dir() {
            walk(&path, out_dir, found);
        } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
            found.push(path);
        }
    }
}

/// Cheap import scanner: regex-free, picks up `import X` and `from X import …` at the top level of a line.
fn crawl_imports(scripts: &[PathBuf]) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    for path in scripts {
        let Ok(text) = fs::read_to_string(path) else { continue };
        for line in text.lines() {
            let line = line.trim();
            let rest = if let Some(r) = line.strip_prefix("from ") {
                r
            } else if let Some(r) = line.strip_prefix("import ") {
                r
            } else {
                continue;
            };
            for tok in rest.split(|c: char| c == ',' || c.is_whitespace()) {
                if tok.is_empty() { continue; }
                let name = tok.split('.').next().unwrap_or(tok);
                if !name.is_empty() { imports.insert(name.to_string()); }
                break; // first token after `import`/`from` is the module
            }
        }
    }
    imports
}

/// For each imported name, resolve via the shared registry, fetch, and stash under dist/vendor/.
fn vendor_packages(
    manifest: &Manifest,
    imports: &BTreeSet<String>,
    out_dir: &Path,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let mut std_local = BTreeMap::new();
    let mut host_local = BTreeMap::new();

    for name in imports {
        // Unknown names are project-local .py modules; let the runtime resolve them at run time.
        let Some((kind, url)) = crate::manifest::resolve(name, manifest) else { continue };
        let bytes = fetch(&url).with_context(|| format!("fetching {url}"))?;
        let local = match kind {
            // std packages are .wasm, except pure-Python ones (test) served as .py; preserve the real extension.
            Kind::Std => format!("vendor/{name}.{}", if url.ends_with(".py") { "py" } else { "wasm" }),
            Kind::Host => format!("vendor/{name}/index.js"),
        };
        write_under(out_dir, &local, &bytes)?;
        match kind {
            Kind::Std => { std_local.insert(name.clone(), local); }
            Kind::Host => { host_local.insert(name.clone(), local); }
        }
    }
    Ok((std_local, host_local))
}

fn write_under(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(rel);
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(&path, bytes)?;
    Ok(())
}

/// Copy each `.py` preserving its path under the project root.
fn copy_scripts(scripts: &[PathBuf], project: &Path, out_dir: &Path) -> Result<usize> {
    let mut count = 0usize;
    for s in scripts {
        let rel = s.strip_prefix(project).unwrap_or(s);
        let dest = out_dir.join(rel);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(s, &dest)?;
        count += 1;
    }
    Ok(count)
}

/// Overlay vendored entries on top of the user's manifest; vendored paths win.
fn rewrite_manifest(
    manifest: &Manifest,
    vendored_imports: &BTreeMap<String, String>,
    vendored_host: &BTreeMap<String, String>,
) -> Manifest {
    let mut out = Manifest::default();
    for (k, v) in &manifest.imports { out.imports.insert(k.clone(), v.clone()); }
    for (k, v) in &manifest.host { out.host.insert(k.clone(), v.clone()); }
    for (k, v) in vendored_imports { out.imports.insert(k.clone(), v.clone()); }
    for (k, v) in vendored_host { out.host.insert(k.clone(), v.clone()); }
    out
}

/// Pick `main.py`/`app.py`/`index.py` if present; otherwise the first script found.
fn find_entry(scripts: &[PathBuf], project: &Path) -> String {
    for c in ["main.py", "app.py", "index.py"] {
        if scripts.iter().any(|s| s.file_name().and_then(|n| n.to_str()) == Some(c)) {
            return c.to_string();
        }
    }
    scripts
        .first()
        .and_then(|s| s.strip_prefix(project).ok())
        .and_then(|p| p.to_str())
        .map(String::from)
        .unwrap_or_else(|| "main.py".to_string())
}

fn dir_size(dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let m = entry.metadata()?;
        if m.is_dir() {
            total = total.saturating_add(dir_size(&entry.path())?);
        } else {
            total = total.saturating_add(m.len());
        }
    }
    Ok(total)
}

fn index_html(entry: &str) -> String {
    INDEX_HTML.replace("__EDGE_ENTRY__", entry)
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url).call().map_err(|e| anyhow!("HTTP error: {e}"))?;
    resp.body_mut().read_to_vec().map_err(|e| anyhow!("reading body: {e}"))
}
