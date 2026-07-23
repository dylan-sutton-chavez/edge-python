/*
WASM bridge: wires parser/VM to host via the handle ABI.
Wire contract lives in `crate::abi`; extend there, never here.
*/

use lol_alloc::{AssumeSingleThreaded, FreeListAllocator};
use crate::abi::{ErrorStash, HandleTable};
use crate::modules::vm::VM;
use crate::modules::vm::types::{Val, VmErr};
use crate::modules::packages::Manifest;
use alloc::{boxed::Box, string::{String, ToString}, vec::Vec};
use core::ptr::NonNull;

mod abi_bridge;
mod errors;
mod exports;
mod resolver;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    pub(super) fn host_print(ptr: *const u8, len: usize);

    /* CallExtern dispatch for register_native_module. Host owns argv; guest writes return into out. `call_id` correlates a deferred result back to its coro via `set_host_result_by_id`. */
    pub(super) fn host_call_native(id: u32, call_id: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32;

    /* Host-cached bytes for `spec`. Non-null `hash_ptr` is a 32-byte expected sha-256. */
    pub(super) fn host_fetch_bytes(spec_ptr: *const u8, spec_len: u32, hash_ptr: *const u8, out_len: *mut u32) -> *mut u8;

    /* Wall-clock in nanoseconds. WASM hosts wire to `Date.now() * 1_000_000`; native hosts to `Instant::now().as_nanos()`. Without this hook the VM falls back to `virtual_clock_ns` which advances deterministically for tests. */
    pub(super) fn host_now_ns() -> u64;
}

pub(super) fn stream_print(s: &str) {
    unsafe { host_print(s.as_ptr(), s.len()); }
}

/* `set_time_hook` wants a `fn() -> u64`. The host import itself is `unsafe extern "C"` so we wrap it in a safe pointer here, the same pattern as `stream_print`. */
pub(super) fn now_ns_host() -> u64 {
    unsafe { host_now_ns() }
}

/* Free-list (not leaking): reclaims Rust allocs so long-lived embeds don't grow monotonically. VM GC only recycles its own Python heap. */
#[global_allocator]
static A: AssumeSingleThreaded<FreeListAllocator> = unsafe { AssumeSingleThreaded::new(FreeListAllocator::new()) };

/* Best-effort panic-to-stash so the host gets a typed message instead of an opaque trap. Re-entry during the format alloc falls through to unreachable(), same trap as before. */
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let msg = alloc::format!("internal panic: {}", info.message());
    with_runtime(|rt| {
        rt.error_stash.set(crate::abi::ErrorKind::Runtime as u32, msg);
    });
    core::arch::wasm32::unreachable()
}

pub(super) const SZ: usize = 1 << 20;

pub(super) enum ModuleEntry {
    Code(String),
    Native(Vec<(String, u32)>),
}

/* VM suspended on `VmErr::HostYield`, kept across `run_start` -> `run_resume` for cooperative resume. */
pub(super) struct PausedRun {
    /* Option so `step_vm` can `take()` for re-entry and stash back without a dummy VM. */
    pub vm: Option<VM<'static>>,
    /* Earliest wake-up deadline (ns) from the last yield; zero for `PendingFrame` / `PendingEvent`. */
    pub last_yield_deadline_ns: u64,
}

/* All mutable WASM-bridge state in one struct so every accessor funnels through `with_runtime`, the sole `unsafe` point, instead of N independent statics. */
pub(super) struct WasmRuntime {
    pub src: [u8; SZ],
    pub out: [u8; SZ],
    pub inp: [u8; SZ],
    pub inp_len: usize,
    pub registry: Vec<(String, ModuleEntry)>,
    pub manifests: Vec<(String, Manifest)>,
    pub handles: HandleTable,
    pub error_stash: ErrorStash,
    /* Last `save_state` blob, read via `snapshot_ptr`. */
    pub snapshot: Vec<u8>,
    /* Set/cleared exclusively by `VmGuard`. The `'static` is storage-only, the pointer is dereferenced only inside the `run()` scope that built it. */
    pub current_vm: Option<NonNull<VM<'static>>>,
    /* Owned across `run_start` / `run_resume`; mutually exclusive with `current_vm`. */
    pub paused_run: Option<Box<PausedRun>>,
    /* Back-edges between preempt yields; 0 disables. */
    pub preempt_every: usize,
}

impl WasmRuntime {
    const fn new() -> Self {
        Self {
            src: [0; SZ],
            out: [0; SZ],
            inp: [0; SZ],
            inp_len: 0,
            registry: Vec::new(),
            manifests: Vec::new(),
            handles: HandleTable::new(),
            error_stash: ErrorStash::new(),
            snapshot: Vec::new(),
            current_vm: None,
            paused_run: None,
            preempt_every: 0,
        }
    }
}

static mut RUNTIME: WasmRuntime = WasmRuntime::new();

// SAFETY: single-threaded WASM; re-entrant callers route through `with_vm` to drop the borrow first.
pub(super) fn with_runtime<R>(f: impl FnOnce(&mut WasmRuntime) -> R) -> R {
    unsafe { f(&mut *core::ptr::addr_of_mut!(RUNTIME)) }
}

pub(super) fn put_val(v: Val) -> u32 { with_runtime(|rt| rt.handles.put(v.0)) }
pub(super) fn get_val(h: u32) -> Option<Val> { with_runtime(|rt| rt.handles.get(h).map(Val)) }

/* RAII publisher for the live VM pointer. Holding the guard across `run()` ensures a panic or early return cannot leave a stale pointer for later `host_edge_op` calls. */
pub(super) struct VmGuard;

impl VmGuard {
    pub(super) fn new(vm: &mut VM<'_>) -> Self {
        // 'static is storage-only, deref only inside the `run()` frame holding the guard.
        let ptr: NonNull<VM<'static>> = NonNull::from(vm).cast();
        with_runtime(|rt| rt.current_vm = Some(ptr));
        Self
    }
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        with_runtime(|rt| rt.current_vm = None);
    }
}

pub(super) fn with_vm<R>(f: impl FnOnce(&mut VM<'static>) -> R) -> Option<R> {
    // Drop the runtime borrow before `f`, VM dispatch re-enters `with_runtime`.
    let ptr = with_runtime(|rt| rt.current_vm)?;
    Some(f(unsafe { &mut *ptr.as_ptr() }))
}

/* Builds a `&[u8]` from an FFI `(ptr, len)`, empty on null or zero length, `from_raw_parts` would UB on either. */
pub(super) unsafe fn safe_bytes<'a>(ptr: *const u8, len: u32) -> &'a [u8] {
    if ptr.is_null() || len == 0 { return &[]; }
    unsafe { core::slice::from_raw_parts(ptr, len as usize) }
}

/* Same for `&[u32]` argv arrays. */
pub(super) unsafe fn safe_handles<'a>(ptr: *const u32, len: u32) -> &'a [u32] {
    if ptr.is_null() || len == 0 { return &[]; }
    unsafe { core::slice::from_raw_parts(ptr, len as usize) }
}

/* Owned UTF-8 string from an FFI `(ptr, len)`; empty on null or invalid UTF-8. */
pub(super) unsafe fn safe_str_owned(ptr: *const u8, len: u32) -> String {
    core::str::from_utf8(unsafe { safe_bytes(ptr, len) }).unwrap_or("").to_string()
}

// Release a batch of handles in one runtime borrow.
pub(super) fn release_handles(handles: &[u32]) {
    with_runtime(|rt| for &h in handles { rt.handles.release(h); });
}

/* `with_vm` that errors when called outside run(). */
pub(super) fn in_vm(err: &'static str, f: impl FnOnce(&mut VM<'static>) -> Result<Val, VmErr>) -> Result<Val, VmErr> {
    with_vm(f).ok_or(VmErr::Runtime(err))?
}

pub(super) unsafe fn write_out(s: &str) -> usize {
    let b = s.as_bytes();
    let n = b.len().min(SZ);
    with_runtime(|rt| rt.out[..n].copy_from_slice(&b[..n]));
    n
}

/* `dispatch_*` prologue: resolve `recv_h` and run `f` against the live VM. Fails on stale handle or call outside `run()`. */
pub(super) fn with_recv<F>(invalid_recv_msg: &'static str, recv_h: u32, f: F) -> Result<Val, VmErr>
where F: FnOnce(&mut VM<'static>, Val) -> Result<Val, VmErr>
{ let recv = get_val(recv_h).ok_or(VmErr::Runtime(invalid_recv_msg))?; with_vm(|vm| f(vm, recv)).ok_or(VmErr::Runtime("edge_op called outside run()"))? }
