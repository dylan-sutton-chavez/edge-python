use crate::packages::{dir_of, join_relative, parse_manifest, parse_integrity, partition_bindings, walk_up_dirs, Resolved, Resolver};
use crate::util::sha256::{hex_encode, sha256};
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

// Cap on the packages.json `extends` chain, 32 dwarfs real workspace depth.
const MAX_PACKAGES_HOPS: u32 = 32;

// Bounds a runaway CDN response, the largest std .so is about 1 MB.
const MAX_FETCH_BYTES: u64 = 64 << 20;

pub struct FileResolver {
    dir: String,
    packages: Option<String>,
}

impl FileResolver {
    pub fn new(dir: &str, packages: Option<&str>) -> Self {
        Self { dir: dir.to_string(), packages: packages.map(String::from) }
    }
}

impl Resolver for FileResolver {
    fn resolve(&mut self, spec: &str) -> Result<Resolved, String> {
        if !spec.contains('/') {
            let dir = self.dir.clone();
            return self.resolve_bare(spec, &dir);
        }
        let canonical = if spec.contains("://") || spec.starts_with('/') {
            spec.to_string()
        } else {
            join_relative(&self.dir, spec)
        };
        self.resolve_canonical(&canonical)
    }

    fn fetch_bytes(&mut self, spec: &str, expected_hash: Option<[u8; 32]>) -> Result<Vec<u8>, String> {
        let (target, frag_hash) = parse_integrity(spec)?;
        let bytes = read_spec(target)?;
        check_pin(target, &bytes, None, expected_hash.or(frag_hash))?;
        Ok(bytes)
    }

    fn child(&self, spec: &str) -> Box<dyn Resolver> {
        Box::new(FileResolver { dir: dir_of(spec).to_string(), packages: self.packages.clone() })
    }
}

impl FileResolver {
    /* Bare names try the manifest first, then built-in host modules, then std defaults. */
    fn resolve_bare(&mut self, name: &str, start_dir: &str) -> Result<Resolved, String> {
        if let Some((dir, target)) = self.lookup_bare(name, start_dir)? {
            let canonical = join_relative(&dir, &target);
            return self.resolve_canonical(&canonical);
        }
        if let Some(resolved) = super::builtins::resolve(name) {
            return Ok(resolved);
        }
        if let Some(url) = std_default(name) {
            return self.resolve_canonical(&url);
        }
        // Browser-bound host packages fail with the canonical marker instead of a manifest hint.
        if crate::devkit::HOST_PACKAGES.contains(&name) {
            return Err(format!("module '{name}' requires the web runtime (run with --web)"));
        }
        Err(format!("no packages.json above '{start_dir}' declares '{name}'\nhelp: declare it, or use a quoted path"))
    }

    /* Returns (manifest dir, target) following `extends`, None when no manifest declares `name`. */
    fn lookup_bare(&mut self, name: &str, start_dir: &str) -> Result<Option<(String, String)>, String> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut search: Option<String> = None;
        let mut hops: u32 = 0;
        loop {
            if hops > MAX_PACKAGES_HOPS {
                return Err(format!("packages.json walk-up exceeded {MAX_PACKAGES_HOPS} hops resolving '{name}'"));
            }
            hops += 1;
            let packages = self.packages.clone();
            let hit = match (&packages, &search) {
                // A --packages override replaces the walk-up entirely, later hops follow its `extends`.
                (Some(m), None) => self.lookup_in_manifest(m, name)?.map(|(t, e)| (dir_of(m).to_string(), t, e)),
                _ => {
                    let from = search.clone().unwrap_or_else(|| start_dir.to_string());
                    let mut found = None;
                    for dir in walk_up_dirs(&from) {
                        let m_spec = format!("{dir}packages.json");
                        if let Some((target, ext)) = self.lookup_in_manifest(&m_spec, name)? {
                            found = Some((dir, target, ext));
                            break;
                        }
                    }
                    found
                }
            };
            let Some((dir, target, ext)) = hit else { return Ok(None) };
            if let Some(target) = target {
                return Ok(Some((dir, target)));
            }
            let Some(ext) = ext else { return Ok(None) };
            let m_spec = format!("{dir}packages.json");
            if !visited.insert(m_spec) {
                return Err("circular extends chain in packages.json".to_string());
            }
            let mut next = join_relative(&dir, &ext);
            if !next.ends_with('/') { next.push('/'); }
            search = Some(next);
        }
    }

    /* One manifest probe, Ok(None) when the file is absent, inner Options are (target, extends). */
    #[allow(clippy::type_complexity)]
    fn lookup_in_manifest(&mut self, m_spec: &str, name: &str) -> Result<Option<(Option<String>, Option<String>)>, String> {
        let Ok(bytes) = std::fs::read(m_spec) else { return Ok(None) };
        let parsed = parse_manifest(&bytes).map_err(|e| format!("packages.json at '{m_spec}': {e}"))?;
        let target = parsed.imports.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        Ok(Some((target, parsed.extends)))
    }

    fn resolve_canonical(&mut self, spec: &str) -> Result<Resolved, String> {
        let (target, frag_hash) = parse_integrity(spec)?;
        if target.ends_with(".py") {
            let bytes = self.fetch_bytes(spec, None)?;
            let src = String::from_utf8(bytes).map_err(|_| format!("module '{target}' is not valid UTF-8"))?;
            return Ok(Resolved::Code { src, canonical: spec.to_string() });
        }
        if target.ends_with(".so") || target.ends_with(".dylib") {
            // dlopen runs arbitrary code, so the cache pins a downloaded plugin on first fetch.
            let path = if target.contains("://") {
                fetch_cached(target, frag_hash)?
            } else {
                let path = PathBuf::from(target);
                if frag_hash.is_some() {
                    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read module '{target}': {e}"))?;
                    check_pin(target, &bytes, None, frag_hash)?;
                }
                path
            };
            let all = super::loader::load(&path)?;
            let (bindings, classes, consts) = partition_bindings(all);
            return Ok(Resolved::Native { bindings, classes, consts, canonical: spec.to_string() });
        }
        if target.ends_with(".wasm") {
            // `edge add` .wasm specs swap to their native twin, dropping a fragment that digests the .wasm.
            if let Some(name) = target.strip_prefix("https://cdn.edgepython.com/std/").and_then(|r| r.strip_suffix(".wasm")) {
                return self.resolve_canonical(&native_std_spec(name));
            }
            return Err(format!("module '{target}' is a .wasm build; the native engine loads native plugins (or run with --web)"));
        }
        Err(format!("module '{target}' has an unsupported extension (expected .py, .so, or .dylib)"))
    }
}

/* Official std packages resolvable by bare name with no manifest, mirrors web/src/defaults.js. */
fn std_default(name: &str) -> Option<String> {
    match name {
        "json" | "re" | "math" | "struct" => Some(native_std_spec(name)),
        "test" => Some(match local_std_dir() {
            Some(d) => format!("{d}/std/test.py"),
            None => "https://cdn.edgepython.com/std/test.py".to_string(),
        }),
        _ => None,
    }
}

/* Native twin of an official std package: local under EDGE_STD_DIR, contract-versioned CDN otherwise. */
fn native_std_spec(name: &str) -> String {
    let file = format!("{name}-{}.{}", std::env::consts::ARCH, std::env::consts::DLL_EXTENSION);
    match local_std_dir() {
        Some(d) => format!("{d}/native/{file}"),
        None => format!("https://cdn.edgepython.com/native/{}/{file}", crate::devkit::RUNTIME_CONTRACT),
    }
}

// Test hook mirroring the web engine's EDGE_RUNTIME_DIR, std assets come from a local dir.
fn local_std_dir() -> Option<String> {
    std::env::var("EDGE_STD_DIR").ok()
}

/* Spec bytes, a cached download for URLs, a plain read for paths. */
fn read_spec(target: &str) -> Result<Vec<u8>, String> {
    if target.contains("://") {
        let path = fetch_cached(target, None)?;
        return std::fs::read(&path).map_err(|e| format!("cannot read cached '{target}': {e}"));
    }
    std::fs::read(target).map_err(|e| format!("cannot read module '{target}': {e}"))
}

/* Downloads once into the user cache, a `.lock` sidecar pins the digest like the runtime lockfile. */
pub fn fetch_cached(url: &str, expected: Option<[u8; 32]>) -> Result<PathBuf, String> {
    let dir = cache_dir()?;
    let ext = url.rsplit('.').next().unwrap_or("bin");
    let file = dir.join(format!("{}.{ext}", hex_encode(&sha256(url.as_bytes()))));
    let lock = file.with_extension(format!("{ext}.lock"));
    if file.exists() {
        let bytes = std::fs::read(&file).map_err(|e| format!("cannot read cached '{url}': {e}"))?;
        // A stale blob under a new explicit pin is a miss, so refetch instead of failing.
        match check_pin(url, &bytes, std::fs::read_to_string(&lock).ok(), expected) {
            Ok(_) => return Ok(file),
            Err(e) if expected.is_none() => return Err(e),
            Err(_) => {}
        }
    }
    let mut resp = ureq::get(url).call().map_err(|e| format!("fetching '{url}': {e}"))?;
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().take(MAX_FETCH_BYTES).read_to_end(&mut bytes).map_err(|e| format!("reading '{url}': {e}"))?;
    let got = check_pin(url, &bytes, None, expected)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating cache dir: {e}"))?;
    // Temp plus rename keeps a truncated download out of the shared cache.
    let tmp = file.with_extension(format!("{ext}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("writing cache for '{url}': {e}"))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("writing cache for '{url}': {e}"))?;
    // Written after the bytes land, so a crash leaves no pin claiming an absent file.
    std::fs::write(&lock, &got).map_err(|e| format!("writing cache for '{url}': {e}"))?;
    Ok(file)
}

/* Hashes `bytes` against an explicit pin, else the sidecar record. Unpinned bytes set the pin. */
fn check_pin(spec: &str, bytes: &[u8], locked: Option<String>, expected: Option<[u8; 32]>) -> Result<String, String> {
    let got = hex_encode(&sha256(bytes));
    // A source fragment is a stated expectation, a sidecar record means upstream moved.
    if let Some(want) = expected.map(|h| hex_encode(&h)) {
        if want != got {
            return Err(format!("integrity check failed for '{spec}'\n expected sha256-{want}\n got sha256-{got}"));
        }
    } else if let Some(want) = locked
        && want != got {
        return Err(format!("integrity drift for '{spec}'\n  locked: sha256-{want}\n  remote: sha256-{got}"));
    }
    Ok(got)
}

fn cache_dir() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(x).join("edge-native"));
    }
    let home = std::env::var("HOME").map_err(|_| "cannot locate a cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home).join(".cache").join("edge-native"))
}
