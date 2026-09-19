use anyhow::{anyhow, bail, Context, Result};
use crate::pack::{Bundle, Entry};
use compiler::modules::{parse_integrity, scan_imports, ImportSpec};
use compiler::util::sha256::sha256;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::manifest::Manifest;

// Marks a standalone binary, its trailer holds the payload length before it.
const STANDALONE_MAGIC: &[u8] = b"EDGESFX\x01";

/* Packs the project as a standalone binary, this exe with the bundle and a trailer appended. */
pub fn standalone(manifest_path: &Path, out: PathBuf) -> Result<()> {
    let bundle = collect_bundle(manifest_path)?;
    let exe = std::env::current_exe().context("locating the edge binary")?;
    let mut image = fs::read(&exe).with_context(|| format!("reading {}", exe.display()))?;
    let payload = bundle.encode();
    image.extend_from_slice(&payload);
    image.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    image.extend_from_slice(STANDALONE_MAGIC);
    fs::write(&out, &image).with_context(|| format!("writing {}", out.display()))?;
    make_executable(&out)?;
    let run = out.display();
    crate::ui::packed(&out, bundle.files.len(), image.len() as u64,
        &format!("run  ./{run}   flags  --save-state --restore-state --preempt --events"));
    Ok(())
}

/* Packs the project as a lightweight .package for a pool that already has the CLI. */
pub fn bundle(manifest_path: &Path, out: PathBuf) -> Result<()> {
    let bundle = collect_bundle(manifest_path)?;
    let payload = bundle.encode();
    fs::write(&out, &payload).with_context(|| format!("writing {}", out.display()))?;
    let run = out.display();
    crate::ui::packed(&out, bundle.files.len(), payload.len() as u64,
        &format!("run  edge run {run}   or send it to an actor eval group"));
    Ok(())
}

/* Reads every project .py plus its edge.json into a bundle, std resolves by name at run time. */
fn collect_bundle(manifest_path: &Path) -> Result<Bundle> {
    let project = match manifest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let scripts = collect_scripts(&project, Path::new(""));
    if scripts.is_empty() {
        return Err(anyhow!("no .py files found under {}", project.display()));
    }
    let mut files = Vec::new();
    for s in &scripts {
        let rel = s.strip_prefix(&project).unwrap_or(s).to_string_lossy().replace('\\', "/");
        files.push(Entry { path: rel, bytes: fs::read(s).with_context(|| format!("reading {}", s.display()))? });
    }
    if manifest_path.exists() {
        files.push(Entry { path: "edge.json".to_string(), bytes: fs::read(manifest_path)? });
    }
    Ok(Bundle { entry: find_entry(&scripts, &project), files })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).with_context(|| format!("setting mode on {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/* The bundle in this exe's trailer, only the tail is read so plain runs stay cheap. */
pub fn embedded_payload() -> Option<Vec<u8>> {
    trailer_payload(&std::env::current_exe().ok()?)
}

/* Bundle carried by a file, either a raw .package or a standalone .edge with a trailer. */
pub fn file_payload(path: &Path) -> Option<Vec<u8>> {
    let mut head = [0u8; 8];
    if let Ok(mut f) = fs::File::open(path) {
        use std::io::Read;
        if f.read_exact(&mut head).is_ok() && head.starts_with(crate::pack::MAGIC) {
            return fs::read(path).ok();
        }
    }
    trailer_payload(path)
}

/* Reads the appended payload of a standalone binary, None when the trailer is absent. */
fn trailer_payload(path: &Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let trailer = (8 + STANDALONE_MAGIC.len()) as u64;
    let total = file.seek(SeekFrom::End(0)).ok()?;
    if total < trailer {
        return None;
    }
    file.seek(SeekFrom::End(-(trailer as i64))).ok()?;
    let mut buf = [0u8; 8 + 8];
    file.read_exact(&mut buf[..trailer as usize]).ok()?;
    if &buf[8..trailer as usize] != STANDALONE_MAGIC {
        return None;
    }
    let len = u64::from_le_bytes(buf[..8].try_into().ok()?);
    let start = total.checked_sub(trailer + len)?;
    let mut payload = vec![0u8; len as usize];
    file.seek(SeekFrom::Start(start)).ok()?;
    file.read_exact(&mut payload).ok()?;
    Some(payload)
}

// Production layout we mirror into dist/js/ and dist/.
const JS_BASE: &str = "https://cdn.edgepython.com/js/";
const COMPILER_WASM: &str = "https://cdn.edgepython.com/compiler.wasm";
// Official package trees, the test hooks read them from a checkout instead.
const BUILTINS_BASE: &str = "https://cdn.edgepython.com/js/builtins/";
const STD_BASE: &str = "https://cdn.edgepython.com/std/";
const JS_FILES: &[&str] = &[
    "src/index.js",
    "src/element.js",
    "src/env.js",
    "src/fetch.js",
    "src/native.js",
    "src/prefetch.js",
    "src/rt.js",
    "src/specs.js",
    "src/util.js",
    "src/cache/idb.js",
    "src/cache/memory.js",
    "src/worker/worker.js",
    "src/worker/engine.js",
];

const INDEX_HTML: &str = include_str!("../templates/dist.html");

/// Pack the project as a browser dist/, vendoring the JS host, compiler and packages.
pub fn run(manifest_path: &Path, out_dir: PathBuf) -> Result<()> {
    let t0 = Instant::now();
    let mut manifest = Manifest::load(manifest_path)?;
    // `Path::parent` returns Some("") for a bare filename, so collapse that to "." explicitly.
    let project = match manifest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let sp = crate::ui::spinner("vendoring the JS host");
    match vendor_js(&out_dir) {
        Ok(()) => sp.done("vendored the JS host"),
        Err(e) => { sp.fail("failed to vendor the JS host"); return Err(e); }
    }

    let sp = crate::ui::spinner("fetching compiler.wasm");
    // Test hook, a local compiler instead of the CDN.
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
    let sp = crate::ui::spinner("vendoring packages");
    let vendored = match vendor_packages(&manifest, &out_dir) {
        Ok(v) => v,
        Err(e) => { sp.fail("failed to vendor packages"); return Err(e); }
    };
    sp.done("vendored packages");
    let script_count = copy_scripts(&scripts, &project, &out_dir)?;

    // Vendored paths replace their urls, every other entry and key stays as written.
    let packages = vendored.len();
    manifest.imports.extend(vendored);
    manifest.save(&out_dir.join("edge.json"))?;

    let entry = find_entry(&scripts, &project);
    fs::write(out_dir.join("index.html"), index_html(&entry))?;

    crate::ui::build_report(
        &out_dir,
        JS_FILES.len(),
        packages,
        script_count,
        dir_size(&out_dir)?,
        t0.elapsed(),
    );
    Ok(())
}

/// Fetch the JS host modules into `dist/js/` mirroring their CDN layout.
fn vendor_js(out_dir: &Path) -> Result<()> {
    // Test hook, a local JS host instead of the CDN.
    let local = std::env::var("EDGE_JS_DIR").ok();
    for rel in JS_FILES {
        let bytes = match &local {
            Some(dir) => {
                let path = Path::new(dir).join(rel.replacen("src/", "dist/", 1));
                fs::read(&path).with_context(|| format!("reading {}", path.display()))?
            }
            None => {
                let url = format!("{JS_BASE}{rel}");
                fetch(&url).with_context(|| format!("fetching {url}"))?
            }
        };
        let path = out_dir.join("js").join(rel);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(&path, bytes)?;
    }
    Ok(())
}

/// Walk the project for `.py` files, skipping hidden dirs and the output directory itself.
fn collect_scripts(project: &Path, out_dir: &Path) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    let out_dir = fs::canonicalize(out_dir).unwrap_or_else(|_| out_dir.to_path_buf());
    walk(project, &out_dir, &mut scripts);
    scripts
}

fn walk(dir: &Path, out_dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // read_dir yields "./dist" while out_dir is "dist", so compare canonical forms.
        if path.is_dir()
            && fs::canonicalize(&path).ok().as_deref() == Some(out_dir) {
                continue;
            }
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

/// Every url the manifest declares lands in dist/vendor/<name>/ with the files it reaches beside it.
fn vendor_packages(manifest: &Manifest, out_dir: &Path) -> Result<BTreeMap<String, String>> {
    let mut local = BTreeMap::new();
    for (name, spec) in manifest.imports.iter().filter(|(_, spec)| spec.contains("://")) {
        let (url, pin) = parse_integrity(spec).map_err(|e| anyhow!(e))?;
        let path = url.split('?').next().unwrap_or(url);
        let (base, entry) = path.rsplit_once('/').ok_or_else(|| anyhow!("'{url}' names no file"))?;
        let bytes = read_package(url)?.ok_or_else(|| anyhow!("fetching {url}: not found"))?;
        if pin.is_some_and(|want| sha256(&bytes) != want) {
            bail!("integrity check failed for '{url}'");
        }
        let dest = format!("vendor/{name}");
        vendor_tree(base, entry, bytes, out_dir, &dest)?;
        local.insert(name.clone(), format!("./{dest}/{entry}"));
    }
    Ok(local)
}

/* Copies `entry` into `dest` with every file its imports reach, each kept at its relative path. */
fn vendor_tree(base: &str, entry: &str, bytes: Vec<u8>, out_dir: &Path, dest: &str) -> Result<()> {
    let mut queue = vec![(entry.to_string(), Some(bytes), true)];
    let mut seen = HashSet::new();
    while let Some((rel, bytes, required)) = queue.pop() {
        if !seen.insert(rel.clone()) {
            continue;
        }
        let url = format!("{base}/{rel}");
        let bytes = match bytes.map_or_else(|| read_package(&url), |b| Ok(Some(b)))? {
            Some(bytes) => bytes,
            None if required => bail!("fetching {url}: not found"),
            None => continue,
        };
        for (spec, needed) in file_deps(&rel, &bytes)? {
            let clean = spec.split(['?', '#']).next().unwrap_or(&spec);
            let dep = join(&rel, clean).ok_or_else(|| anyhow!("'{url}' imports '{spec}' from outside its directory, which a web build cannot vendor"))?;
            queue.push((dep, None, needed));
        }
        write_under(out_dir, &format!("{dest}/{rel}"), &bytes)?;
    }
    Ok(())
}

/* What a vendored file pulls in beside it, each flagged when the build fails without it. */
fn file_deps(rel: &str, bytes: &[u8]) -> Result<Vec<(String, bool)>> {
    let text = String::from_utf8_lossy(bytes);
    let ext = Path::new(rel).extension().and_then(|e| e.to_str()).unwrap_or("");
    Ok(match ext {
        "js" | "mjs" => js_imports(&text).into_iter().map(|spec| (spec.to_string(), true)).collect(),
        "json" => {
            let manifest: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("parsing {rel}"))?;
            let imports = manifest.get("imports").and_then(|i| i.as_object()).into_iter().flatten();
            imports
                .filter_map(|(_, target)| target.as_str())
                .filter(|target| !target.contains("://") && !target.starts_with('/'))
                .map(|target| (target.to_string(), true))
                .collect()
        }
        "wasm" => Vec::new(),
        _ if bytes.starts_with(b"\0asm") => Vec::new(),
        // Python source, its relative imports plus the manifest beside it when one is served.
        _ => scan_imports(&text)
            .into_iter()
            .filter_map(|imp| match imp {
                ImportSpec::Relative(path) => Some((path, true)),
                _ => None,
            })
            .chain([("./edge.json".to_string(), false)])
            .collect(),
    })
}

/* Relative specifiers after `from` or `import`, the static imports and re-exports of an ES module. */
fn js_imports(src: &str) -> Vec<&str> {
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

/* Resolves `spec` against the vendored file `from`, None when it climbs out of the package directory. */
fn join(from: &str, spec: &str) -> Option<String> {
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

/* A package file's bytes, None when the host has no such file. */
fn read_package(url: &str) -> Result<Option<Vec<u8>>> {
    if let Some(path) = local_copy(url) {
        return match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow!("reading {}: {e}", path.display())),
        };
    }
    match ureq::get(url).call() {
        Ok(mut resp) => Ok(Some(resp.body_mut().read_to_vec().map_err(|e| anyhow!("reading {url}: {e}"))?)),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(anyhow!("fetching {url}: {e}")),
    }
}

// Test hooks, EDGE_JS_DIR serves the official JavaScript libraries and EDGE_STD_DIR the std packages.
fn local_copy(url: &str) -> Option<PathBuf> {
    if let (Some(rest), Ok(dir)) = (url.strip_prefix(BUILTINS_BASE), std::env::var("EDGE_JS_DIR")) {
        let (lib, file) = rest.split_once('/')?;
        return Some(Path::new(&dir).join("builtins").join(lib).join("src").join(file));
    }
    if let (Some(file), Ok(dir)) = (url.strip_prefix(STD_BASE), std::env::var("EDGE_STD_DIR")) {
        return Some(Path::new(&dir).join(file));
    }
    None
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

/// Pick `main.py`/`app.py`/`index.py` if present, otherwise the first script found.
fn find_entry(scripts: &[PathBuf], project: &Path) -> String {
    let rel = |s: &PathBuf| s.strip_prefix(project).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
    for c in ["main.py", "app.py", "index.py"] {
        if let Some(s) = scripts.iter().find(|s| s.file_name().and_then(|n| n.to_str()) == Some(c)) {
            return rel(s).unwrap_or_else(|| c.to_string());
        }
    }
    scripts
        .first()
        .and_then(rel)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn nested_main_is_the_entry_with_its_path() {
        let scripts = paths(&["./sub/main.py"]);
        assert_eq!(find_entry(&scripts, Path::new(".")), "sub/main.py");
    }

    #[test]
    fn root_main_wins_over_nested_candidates() {
        let scripts = paths(&["./sub/app.py", "./main.py"]);
        assert_eq!(find_entry(&scripts, Path::new(".")), "main.py");
    }

    #[test]
    fn nested_app_is_the_entry_with_its_path() {
        let scripts = paths(&["./util.py", "./sub/app.py"]);
        assert_eq!(find_entry(&scripts, Path::new(".")), "sub/app.py");
    }

    #[test]
    fn without_candidates_the_first_script_is_the_entry() {
        let scripts = paths(&["./sub/tool.py"]);
        assert_eq!(find_entry(&scripts, Path::new(".")), "sub/tool.py");
    }

    #[test]
    fn empty_project_falls_back_to_main() {
        assert_eq!(find_entry(&[], Path::new(".")), "main.py");
    }

    #[test]
    fn walk_skips_the_output_dir_in_any_path_form() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        fs::write(project.join("main.py"), "").unwrap();
        fs::create_dir(project.join("dist")).unwrap();
        fs::write(project.join("dist/stale.py"), "").unwrap();
        // The CLI hands over a bare relative "dist", so pass a non-normalized form here.
        fs::create_dir(project.join("sub")).unwrap();
        let out_dir = project.join("sub/..").join("dist");
        let scripts = collect_scripts(project, &out_dir);
        assert_eq!(scripts, paths(&[project.join("main.py").to_str().unwrap()]));
    }

    #[test]
    fn walk_keeps_a_deeper_dir_named_like_the_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        fs::create_dir_all(project.join("sub/dist")).unwrap();
        fs::write(project.join("sub/dist/keep.py"), "").unwrap();
        fs::create_dir(project.join("dist")).unwrap();
        let scripts = collect_scripts(project, &project.join("dist"));
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].ends_with("sub/dist/keep.py"));
    }
}

