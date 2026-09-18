use super::{plugins, Native, Vm};
use crate::builtins;
use compiler::packages::{dir_of, join_relative, parse_integrity, parse_manifest, scan_imports, walk_up_dirs, ImportSpec};
use compiler::util::sha256::{hex_encode, sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;

// The pure Edge Python test package, embedded at build time.
const TEST_PY: &str = include_str!("../../../std/test/src/entry.py");
const TEST_SPEC: &str = "https://cdn.edgepython.com/std/test.py";
// Official system modules, a spec under it needs a browser unless the CLI builds it in.
const JS_BUILTINS_BASE: &str = "https://cdn.edgepython.com/js/builtins/";
// Bounds a runaway download, the largest module is well under a megabyte.
const MAX_FETCH_BYTES: u64 = 64 << 20;

/* Where a run's files come from and which bare names it may resolve. */
#[derive(Clone, Default)]
pub struct Project {
    pub entry_dir: String,
    pub packages: Option<String>,
    // An in-memory tree replaces the disk, untrusted runs always carry one.
    pub bundle: Option<Rc<HashMap<String, Vec<u8>>>>,
    pub untrusted: bool,
}

impl Project {
    pub fn disk(entry_dir: &str, packages: Option<&str>) -> Project {
        Project { entry_dir: entry_dir.to_string(), packages: packages.map(String::from), bundle: None, untrusted: false }
    }

    pub fn bundle(files: HashMap<String, Vec<u8>>, entry_dir: &str, untrusted: bool) -> Project {
        Project { entry_dir: entry_dir.to_string(), packages: None, bundle: Some(Rc::new(files)), untrusted }
    }
}

/* Registers every module `root_src` reaches, mirroring the lazy prefetch of the JS host. */
pub fn prefetch(vm: &mut Vm, root_src: &str) -> Result<(), String> {
    Walk::new(vm).run(root_src)
}

struct Walk<'a> {
    vm: &'a mut Vm,
    project: Project,
    // Bare name to spec, the nearest manifest wins.
    table: HashMap<String, String>,
    visited: HashSet<String>,
    queue: VecDeque<String>,
    failures: Vec<String>,
    // Bare names seen before a manifest declared them, retried after each merge.
    pending_bare: Vec<String>,
    // Root-relative imports waiting on their importer's manifest chain.
    pending_root: Vec<(String, String)>,
    manifest_dirs: HashSet<String>,
    missing: HashSet<String>,
}

impl<'a> Walk<'a> {
    fn new(vm: &'a mut Vm) -> Self {
        let project = vm.project.clone();
        Walk {
            vm,
            project,
            table: HashMap::new(),
            visited: HashSet::new(),
            queue: VecDeque::new(),
            failures: Vec::new(),
            pending_bare: Vec::new(),
            pending_root: Vec::new(),
            manifest_dirs: HashSet::new(),
            missing: HashSet::new(),
        }
    }

    fn run(mut self, root_src: &str) -> Result<(), String> {
        let entry_dir = self.project.entry_dir.clone();
        for imp in scan_imports(root_src) {
            self.enqueue_import(imp, &entry_dir);
        }
        self.enqueue_manifest_chain(&entry_dir);
        while let Some(spec) = self.queue.pop_front() {
            if !self.visited.insert(spec.clone()) {
                continue;
            }
            if let Some(name) = spec.strip_prefix("mt:") {
                self.system(name);
                continue;
            }
            if spec.ends_with("packages.json") {
                self.manifest(&spec);
                continue;
            }
            if let Some(name) = std_name(&spec) {
                if let Err(e) = plugins::register(self.vm, name, &spec) {
                    self.failures.push(e);
                }
                continue;
            }
            if let Some(name) = browser_module(&spec) {
                self.failures.push(format!("module '{name}' requires a browser"));
                continue;
            }
            if let Some(reason) = foreign(&spec) {
                self.failures.push(format!("module '{}' {reason}", target(&spec)));
                continue;
            }
            match self.fetch(&spec) {
                Ok(Some(bytes)) => self.module(&spec, bytes),
                Ok(None) => self.failures.push(format!("could not read module '{}'", target(&spec))),
                Err(e) => self.failures.push(e),
            }
        }
        if self.failures.is_empty() {
            return Ok(());
        }
        Err(self.failures.iter().map(|f| format!("error: {f}")).collect::<Vec<_>>().join("\n"))
    }

    // A code module registers, then its own imports queue so transitive deps stay lazy.
    fn module(&mut self, spec: &str, bytes: Vec<u8>) {
        if let Err(e) = self.vm.register_code(spec, &bytes) {
            self.failures.push(e);
            return;
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.vm.store.data_mut().fetched.insert(spec.to_string(), bytes);
        let dir = dir_of(spec).to_string();
        for imp in scan_imports(&text) {
            self.enqueue_import(imp, &dir);
        }
        self.enqueue_manifest_chain(&dir);
    }

    /* Registers a built-in capability under `mt:<name>`, names that need a browser fail here. */
    fn system(&mut self, name: &str) {
        if self.project.untrusted && matches!(name, "actor" | "network") {
            self.failures.push(format!("module '{name}' is not available to untrusted eval runs"));
            return;
        }
        let Some((module, exports)) = builtins::exports(name) else {
            self.failures.push(format!("module '{name}' requires a browser"));
            return;
        };
        let spec = format!("mt:{name}");
        let known = self.vm.store.data().registered.get(&spec).cloned();
        let (base, names) = match known {
            Some(entry) => entry,
            None => {
                let state = self.vm.store.data_mut();
                let base = state.natives.len();
                let mut names = Vec::new();
                for (export, deferred) in exports {
                    names.push(export.to_string());
                    state.natives.push(Native::Capability { module, name: export.to_string(), deferred });
                }
                state.registered.insert(spec.clone(), (base, names.clone()));
                (base, names)
            }
        };
        if let Err(e) = self.vm.register_native(&spec, &names, base) {
            self.failures.push(e);
        }
    }

    /* Merges a manifest into the table and serves it to the compiler as written. */
    fn manifest(&mut self, spec: &str) {
        let bytes = match self.read_manifest(spec) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                self.missing.insert(spec.to_string());
                self.retry_root();
                return;
            }
            Err(e) => {
                self.failures.push(e);
                return;
            }
        };
        let parsed = match parse_manifest(&bytes) {
            Ok(m) => m,
            Err(e) => {
                self.failures.push(format!("packages.json at '{spec}': {e}"));
                return;
            }
        };
        let dir = dir_of(spec).to_string();
        self.manifest_dirs.insert(dir.clone());
        for (name, target) in &parsed.imports {
            self.table.entry(name.clone()).or_insert_with(|| join_relative(&dir, target));
        }
        self.vm.store.data_mut().fetched.insert(spec.to_string(), bytes);
        self.retry_pending();
        self.retry_root();
        if let Some(ext) = &parsed.extends {
            let mut next = join_relative(&dir, ext);
            if !next.ends_with('/') {
                next.push('/');
            }
            self.queue.push_back(format!("{next}packages.json"));
        }
    }

    /* The root manifest always exists, a `--packages` override is then the only manifest. */
    fn read_manifest(&mut self, spec: &str) -> Result<Option<Vec<u8>>, String> {
        let root = spec == "packages.json";
        if let Some(path) = self.project.packages.clone() {
            if !root {
                return Ok(None);
            }
            return match std::fs::read(&path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Some(b"{}".to_vec())),
                Err(e) => Err(format!("reading {path}: {e}")),
            };
        }
        let bytes = self.fetch(spec).unwrap_or(None);
        if root && bytes.is_none() {
            return Ok(Some(b"{}".to_vec()));
        }
        Ok(bytes)
    }

    fn enqueue_import(&mut self, imp: ImportSpec, dir: &str) {
        match imp {
            ImportSpec::Relative(path) => self.queue.push_back(join_relative(dir, &path)),
            ImportSpec::Root(path) => self.enqueue_root(path, dir.to_string()),
            ImportSpec::Bare(name) => match self.table.get(&name) {
                Some(spec) => self.queue.push_back(spec.clone()),
                None => self.pending_bare.push(name),
            },
        }
    }

    fn retry_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending_bare);
        for name in pending {
            match self.table.get(&name) {
                Some(spec) => self.queue.push_back(spec.clone()),
                None => self.pending_bare.push(name),
            }
        }
    }

    // Probes every ancestor manifest, mirroring the compiler walk-up.
    fn enqueue_manifest_chain(&mut self, dir: &str) {
        let chain: Vec<String> = walk_up_dirs(dir).collect();
        for d in chain {
            let m = format!("{d}packages.json");
            if !self.missing.contains(&m) {
                self.queue.push_back(m);
            }
        }
    }

    /* Nearest manifest dir at or above `dir`, None while probes pend, Some(None) once probed bare. */
    fn root_for(&self, dir: &str) -> Option<Option<String>> {
        for d in walk_up_dirs(dir) {
            if self.manifest_dirs.contains(&d) {
                return Some(Some(d));
            }
            let m = format!("{d}packages.json");
            if !self.visited.contains(&m) && !self.missing.contains(&m) {
                return None;
            }
        }
        Some(None)
    }

    fn enqueue_root(&mut self, spec: String, dir: String) {
        match self.root_for(&dir) {
            None => self.pending_root.push((spec, dir)),
            Some(Some(root)) => self.queue.push_back(join_relative(&root, &spec)),
            // No manifest anywhere, the compiler reports it.
            Some(None) => {}
        }
    }

    fn retry_root(&mut self) {
        let pending = std::mem::take(&mut self.pending_root);
        for (spec, dir) in pending {
            self.enqueue_root(spec, dir);
        }
    }

    /* Bytes for a spec, the bundle, a pinned download or the disk, None when absent. */
    fn fetch(&mut self, spec: &str) -> Result<Option<Vec<u8>>, String> {
        let (target, pin) = parse_integrity(spec)?;
        if target == TEST_SPEC {
            return Ok(Some(TEST_PY.as_bytes().to_vec()));
        }
        let bytes = if let Some(files) = &self.project.bundle {
            // Bundle paths are plain, a joined spec may still carry the importer's leading dot.
            files.get(target.strip_prefix("./").unwrap_or(target)).cloned()
        } else if target.contains("://") {
            Some(fetch_cached(target, pin)?)
        } else {
            match std::fs::read(target) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(format!("cannot read module '{target}': {e}")),
            }
        };
        if let (Some(bytes), Some(pin)) = (&bytes, pin)
            && !target.contains("://")
        {
            check_pin(target, bytes, None, Some(pin))?;
        }
        Ok(bytes)
    }
}

/* The official name a CDN system module spec carries, the facade and its JS twin alike. */
fn browser_module(spec: &str) -> Option<&str> {
    target(spec).strip_prefix(JS_BUILTINS_BASE)?.split('/').next().filter(|n| !n.is_empty())
}

fn target(spec: &str) -> &str {
    spec.split_once('#').map_or(spec, |(t, _)| t)
}

/* An official std spec names a built-in package, the fragment is left to the caller. */
fn std_name(spec: &str) -> Option<&'static str> {
    let name = target(spec).strip_prefix(plugins::STD_BASE)?.strip_suffix(".wasm")?;
    ["json", "re", "math", "struct"].into_iter().find(|n| *n == name)
}

/* Why a binary spec cannot load here, plugins need the JS host and native libraries nothing. */
fn foreign(spec: &str) -> Option<&'static str> {
    let t = target(spec);
    if t.ends_with(".wasm") {
        return Some("requires the JS host");
    }
    if t.ends_with(".so") || t.ends_with(".dylib") {
        return Some("is not supported, ship a .wasm");
    }
    None
}

/* Downloads once into the user cache, a `.lock` sidecar pins the digest like the JS host lockfile. */
fn fetch_cached(url: &str, expected: Option<[u8; 32]>) -> Result<Vec<u8>, String> {
    let dir = cache_dir()?;
    let ext = url.rsplit('.').next().unwrap_or("bin");
    let file = dir.join(format!("{}.{ext}", hex_encode(&sha256(url.as_bytes()))));
    let lock = file.with_extension(format!("{ext}.lock"));
    if file.exists() {
        let bytes = std::fs::read(&file).map_err(|e| format!("cannot read cached '{url}': {e}"))?;
        // A stale blob under a new explicit pin is a miss, so refetch instead of failing.
        match check_pin(url, &bytes, std::fs::read_to_string(&lock).ok(), expected) {
            Ok(_) => return Ok(bytes),
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
    std::fs::write(&lock, &got).map_err(|e| format!("writing cache for '{url}': {e}"))?;
    Ok(bytes)
}

/* Hashes `bytes` against an explicit pin, else the sidecar record, unpinned bytes set the pin. */
fn check_pin(spec: &str, bytes: &[u8], locked: Option<String>, expected: Option<[u8; 32]>) -> Result<String, String> {
    let got = hex_encode(&sha256(bytes));
    if let Some(want) = expected.map(|h| hex_encode(&h)) {
        if want != got {
            return Err(format!("integrity check failed for '{spec}'\n expected sha256-{want}\n got sha256-{got}"));
        }
    } else if let Some(want) = locked
        && want != got
    {
        return Err(format!("integrity drift for '{spec}'\n  locked: sha256-{want}\n  remote: sha256-{got}"));
    }
    Ok(got)
}

fn cache_dir() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(x).join("edge").join("modules"));
    }
    let home = std::env::var("HOME").map_err(|_| "cannot locate a cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home).join(".cache").join("edge").join("modules"))
}
