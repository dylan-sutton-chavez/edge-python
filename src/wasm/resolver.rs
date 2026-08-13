use crate::packages::{NativeBinding, Resolved, Resolver, partition_bindings, parse_manifest, walk_up_dirs, dir_of, join_relative};
use crate::util::hash::FxHashSet;
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec};
use crate::s;

use super::{ModuleEntry, host_fetch_bytes, with_runtime};
use super::exports::wasm_free;
use crate::abi::ErrorKind;
use crate::bridge::{error_from_kind, get_val, put_val, release_handles, take_error, with_vm};
use crate::vm::types::{Val, VmErr};
use alloc::sync::Arc;

// Cap on packages.json `extends` chain, bounds attacker-crafted loops, 32 dwarfs real workspace depth.
const MAX_PACKAGES_HOPS: u32 = 32;

pub(super) struct WasmHostResolver { pub(super) dir: String }

impl Resolver for WasmHostResolver {
    fn resolve(&mut self, spec: &str) -> Result<Resolved, String> {
        if !spec.contains('/') {
            let dir = self.dir.clone();
            return self.resolve_bare(spec, &dir);
        }
        let canonical = if spec.contains("://") || spec.starts_with('/') {
            spec.to_string()
        } else if spec.starts_with("./") || spec.starts_with("../") {
            join_relative(&self.dir, spec)
        } else {
            // A dotted import anchors at the nearest manifest dir.
            let root = self.manifest_root(spec)?;
            join_relative(&root, spec)
        };
        self.resolve_canonical(&canonical)
    }

    fn fetch_bytes(&mut self,spec: &str,expected_hash: Option<[u8; 32]>) -> Result<Vec<u8>, String> {
        let mut len: u32 = 0;
        let hash_ptr = expected_hash.as_ref().map(|h| h.as_ptr()).unwrap_or(core::ptr::null());
        let ptr = unsafe {
            host_fetch_bytes(spec.as_ptr(), spec.len() as u32, hash_ptr, &mut len as *mut u32)
        };
        if ptr.is_null() {
            return Err(s!("no bytes cached by host for '", str spec, "'"));
        }
        // Host allocates via `wasm_alloc` (abi.md), copy into a guest Vec, then `wasm_free`. `Vec::from_raw_parts` would UB by freeing Box-laid memory through Vec's layout.
        let len = len as usize;
        let bytes: Vec<u8> = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
        unsafe { wasm_free(ptr, len as u32) };
        Ok(bytes)
    }

    fn child(&self, spec: &str) -> Box<dyn Resolver> {
        Box::new(WasmHostResolver { dir: dir_of(spec).to_string() })
    }
}

impl WasmHostResolver {
    fn resolve_bare(&mut self, name: &str, start_dir: &str) -> Result<Resolved, String> {
        let mut visited: FxHashSet<String> = FxHashSet::default();
        let mut search_dir = start_dir.to_string();
        let mut hops: u32 = 0;
        loop {
            if hops > MAX_PACKAGES_HOPS {
                return Err(s!(
                    "packages.json walk-up exceeded ",
                    int MAX_PACKAGES_HOPS as i64,
                    " hops resolving '", str name, "'"));
            }
            hops += 1;

            let mut hit: Option<(String, Option<String>, Option<String>)> = None;
            for dir in walk_up_dirs(&search_dir) {
                let m_spec = s!(str &dir, "packages.json");
                if let Some((target, ext)) = self.lookup_in_manifest(&m_spec, name)? {
                    hit = Some((dir, target, ext));
                    break;
                }
            }
            let Some((dir, target, ext)) = hit else {
                return Err(s!("no packages.json above '", str start_dir, "' declares '", str name, "'"));
            };
            if let Some(target) = target {
                let canonical = join_relative(&dir, &target);
                return self.resolve_canonical(&canonical);
            }
            let m_spec = s!(str &dir, "packages.json");
            if let Some(ext) = ext {
                if !visited.insert(m_spec) {
                    return Err(s!("circular extends chain in packages.json"));
                }
                let mut next = join_relative(&dir, &ext);
                if !next.ends_with('/') { next.push('/'); }
                search_dir = next;
                continue;
            }
            return Err(s!("alias '", str name, "' not declared in '", str &m_spec, "'\n", "help: declare it, add \"extends\": \"..\" to inherit, or use a relative import",
            ));
        }
    }

    /* Nearest ancestor dir holding a packages.json, probed live like the bare-name walk-up. */
    fn manifest_root(&mut self, spec: &str) -> Result<String, String> {
        let start = self.dir.clone();
        for dir in walk_up_dirs(&start) {
            let m_spec = s!(str &dir, "packages.json");
            let cached = with_runtime(|rt| rt.manifests.iter().any(|(s, _)| s == &m_spec));
            if cached || self.fetch_bytes(&m_spec, None).is_ok() {
                return Ok(dir);
            }
        }
        Err(s!("no packages.json above '", str &self.dir, "' to resolve '", str spec, "'"))
    }

    #[allow(clippy::type_complexity)]
    fn lookup_in_manifest(&mut self, m_spec: &str, name: &str) -> Result<Option<(Option<String>, Option<String>)>, String> {
        if let Some(hit) = with_runtime(|rt| {
            rt.manifests.iter()
                .find(|(s, _)| s == m_spec)
                .map(|(_, m)| (m.imports.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()), m.extends.clone()))
        }) {
            return Ok(Some(hit));
        }
        // Walk-up fetch, manifests aren't pinned by URL fragment, so no hash.
        let bytes = match self.fetch_bytes(m_spec, None) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        let parsed = parse_manifest(&bytes).map_err(|e| s!("packages.json at '", str m_spec, "': ", str &e))?;
        let target = parsed.imports.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        let ext = parsed.extends.clone();
        with_runtime(|rt| rt.manifests.push((m_spec.to_string(), parsed)));
        Ok(Some((target, ext)))
    }

    fn resolve_canonical(&self, spec: &str) -> Result<Resolved, String> {
        let entry = with_runtime(|rt| {
            rt.registry.iter().find(|(s, _)| s == spec).map(|(s, e)| {
                let cloned = match e {
                    ModuleEntry::Code(src) => ModuleEntry::Code(src.clone()),
                    ModuleEntry::Native(funcs) => ModuleEntry::Native(funcs.clone()),
                };
                (s.clone(), cloned)
            })
        }).ok_or_else(|| s!("module '", str spec, "' not registered (host did not pre-fetch / register before run())"))?;
        match entry.1 {
            ModuleEntry::Code(src) => Ok(Resolved::Code {
                src,
                canonical: spec.to_string(),
            }),
            ModuleEntry::Native(funcs) => {
                let all: Vec<NativeBinding> = funcs.iter().map(|(n, id)| make_native_binding(n.clone(), *id)).collect();
                let (bindings, classes, consts) = partition_bindings(all);
                Ok(Resolved::Native { bindings, classes, consts, canonical: spec.to_string() })
            }
        }
    }
}

/* Builds a NativeBinding that marshals handles around `host_call_native`. Lives here so the bridge stays host-import-free. */
fn make_native_binding(name: String, id: u32) -> NativeBinding {
    let closure = move |_: &mut crate::vm::types::HeapPool, args: &[Val], kwargs: Option<Val>| -> Result<Val, VmErr> {
        /* 1. Register positional args as handles the guest will see, append the kwargs handle (0 means no kwargs). */
        let mut argv: Vec<u32> = args.iter().map(|v| put_val(*v)).collect();
        argv.push(kwargs.map_or(0, put_val));
        let mut out_handle: u32 = 0;

        // call_id is what call_extern will park with on defer, lets the host route the result back.
        let call_id = with_vm(|vm| vm.next_host_call_id as u32).unwrap_or(0);
        let status = unsafe {
            super::host_call_native(
                id, call_id,
                argv.as_ptr(), argv.len() as u32,
                &mut out_handle as *mut u32,
            )
        };

        /* 3. Read result BEFORE releasing argv, a returned input would point into slots we're about to free. */
        // Status 2 = DEFERRED, handler has captured what it needs, release argv and park the VM.
        if status == 2 {
            release_handles(&argv);
            return Err(VmErr::HostCallDeferred);
        }
        if status != 0 {
            release_handles(&argv);
            let (kind, msg) = take_error()
                .unwrap_or((ErrorKind::Runtime as u32, String::from("native call failed")));
            return Err(error_from_kind(kind, msg));
        }
        let result = get_val(out_handle).ok_or(VmErr::Runtime("native returned invalid handle"))?;
        argv.push(out_handle);
        release_handles(&argv);
        Ok(result)
    };
    NativeBinding { name, func: Arc::new(closure), pure: false }
}

