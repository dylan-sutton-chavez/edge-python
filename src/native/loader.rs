use crate::abi::{ErrorKind, EDGE_ABI_VERSION};
use crate::bridge::{error_from_kind, get_val, put_val, release_handles, take_error};
use crate::packages::NativeBinding;
use crate::vm::types::{HeapPool, Val, VmErr};
use std::path::Path;
use std::sync::Arc;

type PluginFn = unsafe extern "C" fn(*const u32, u32, *mut u32) -> i32;

// A dlopen'd plugin resolves these against the host binary, exported via -rdynamic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_op(op: u32, recv: u32, name_ptr: *const u8, name_len: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32 {
    unsafe { crate::bridge::host_edge_op(op, recv, name_ptr, name_len, argv_ptr, argc, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_encode(tag: u32, ptr: *const u8, len: u32) -> u32 {
    unsafe { crate::bridge::host_edge_encode(tag, ptr, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_decode(h: u32, out_tag: *mut u32, dst: *mut u8, dst_max: u32) -> i32 {
    unsafe { crate::bridge::host_edge_decode(h, out_tag, dst, dst_max) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_release(h: u32) {
    unsafe { crate::bridge::host_edge_release(h) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_throw(kind: u32, msg_ptr: *const u8, msg_len: u32) {
    unsafe { crate::bridge::host_edge_throw(kind, msg_ptr, msg_len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn edge_take_error(out_kind: *mut u32, dst: *mut u8, dst_max: u32) -> i32 {
    unsafe { crate::bridge::host_edge_take_error(out_kind, dst, dst_max) }
}

/* Opens the plugin, checks its ABI version, and binds every convention export by raw name. */
pub(super) fn load(path: &Path) -> Result<Vec<NativeBinding>, String> {
    // Touching the exports keeps them linked into the final binary for dlopen to resolve.
    std::hint::black_box([edge_op as *const (), edge_encode as *const (), edge_decode as *const (), edge_release as *const (), edge_throw as *const (), edge_take_error as *const ()]);
    let names = candidate_exports(path)?;
    let lib: &'static libloading::Library = Box::leak(Box::new(
        unsafe { libloading::Library::new(path) }.map_err(|e| format!("dlopen '{}': {}", path.display(), dl_detail(&e)))?,
    ));
    let version: unsafe extern "C" fn() -> u32 = *unsafe { lib.get(b"__edge_abi_version") }
        .map_err(|_| format!("'{}' is missing __edge_abi_version", path.display()))?;
    let got = unsafe { version() };
    if got != EDGE_ABI_VERSION {
        return Err(format!("'{}' speaks ABI v{got}, this engine expects v{EDGE_ABI_VERSION}", path.display()));
    }
    let mut bindings = Vec::new();
    for name in names {
        // dlsym is the ground truth, a candidate that fails to resolve was string noise.
        let Ok(sym) = (unsafe { lib.get::<PluginFn>(name.as_bytes()) }) else { continue };
        let f: PluginFn = *sym;
        bindings.push(bind(name, f));
    }
    if bindings.is_empty() {
        return Err(format!("'{}' exports no __fn_/__class_/__const_ symbols", path.display()));
    }
    Ok(bindings)
}

/* libloading 0.9 renders a bare "dlopen failed", the dlerror text it wraps is the actionable part. */
fn dl_detail(e: &libloading::Error) -> String {
    match std::error::Error::source(e) {
        Some(src) => src.to_string(),
        None => e.to_string(),
    }
}

/* Marshals handles around one plugin call, mirroring the wasm-side extern dispatch. */
fn bind(name: String, f: PluginFn) -> NativeBinding {
    let closure = move |_: &mut HeapPool, args: &[Val], kwargs: Option<Val>| -> Result<Val, VmErr> {
        let mut argv: Vec<u32> = args.iter().map(|v| put_val(*v)).collect();
        argv.push(kwargs.map_or(0, put_val));
        let mut out: u32 = 0;
        let status = unsafe { f(argv.as_ptr(), argv.len() as u32, &mut out) };
        // Status 2 means deferred, the scheduler parks the calling coroutine.
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
        let result = get_val(out).ok_or(VmErr::Runtime("native returned invalid handle"))?;
        argv.push(out);
        release_handles(&argv);
        Ok(result)
    };
    NativeBinding { name, func: Arc::new(closure), pure: false }
}

/* Scans raw bytes for convention-prefixed names, format-agnostic since exported names must exist verbatim. */
fn candidate_exports(path: &Path) -> Result<Vec<String>, String> {
    let b = std::fs::read(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let mut names: Vec<String> = Vec::new();
    for prefix in [b"__fn_".as_slice(), b"__class_", b"__const_"] {
        let mut at = 0;
        while let Some(hit) = b[at..].windows(prefix.len()).position(|w| w == prefix) {
            let start = at + hit;
            let end = start + b[start..].iter().take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_').count();
            at = end;
            if end > start + prefix.len()
                && let Ok(name) = core::str::from_utf8(&b[start..end]) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}
