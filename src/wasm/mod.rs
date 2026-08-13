use crate::vm::VM;
use crate::packages::Manifest;
use alloc::{boxed::Box, string::String, vec::Vec};

// Wires parser/VM to the host via the handle ABI, the wire contract lives in `crate::abi`, extend there, never here.
mod exports;
mod resolver;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    pub(super) fn host_print(ptr: *const u8, len: usize);

    /* CallExtern dispatch for register_native_module. Host owns argv, guest writes return into out. `call_id` correlates a deferred result back to its coro via `set_host_result_by_id`. */
    pub(super) fn host_call_native(id: u32, call_id: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32;

    /* Host-cached bytes for `spec`. Non-null `hash_ptr` is a 32-byte expected sha-256. */
    pub(super) fn host_fetch_bytes(spec_ptr: *const u8, spec_len: u32, hash_ptr: *const u8, out_len: *mut u32) -> *mut u8;

    /* Wall-clock in nanoseconds. WASM hosts wire to `Date.now() * 1_000_000`, native hosts to `Instant::now().as_nanos()`. Without this hook the VM falls back to `virtual_clock_ns` which advances deterministically for tests. */
    pub(super) fn host_now_ns() -> u64;
}

pub(super) fn stream_print(s: &str) {
    unsafe { host_print(s.as_ptr(), s.len()); }
}

/* `set_time_hook` wants a `fn() -> u64`. The host import itself is `unsafe extern "C"` so we wrap it in a safe pointer here, the same pattern as `stream_print`. */
pub(super) fn now_ns_host() -> u64 {
    unsafe { host_now_ns() }
}

/* dlmalloc, binned O(1) alloc/free, so cost stays flat as live Rust blocks grow. The old free-list allocator degraded linearly per op on large live heaps. */
#[global_allocator]
static A: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

/* Best-effort panic-to-stash so the host gets a typed message instead of an opaque trap. Re-entry during the format alloc falls through to unreachable(), same trap as before. */
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let msg = alloc::format!("internal panic: {}", info.message());
    crate::bridge::stash_raw_error(crate::abi::ErrorKind::Runtime as u32, msg);
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
    /* Earliest wake-up deadline (ns) from the last yield, zero for `PendingFrame` / `PendingEvent`. */
    pub last_yield_deadline_ns: u64,
}

/* Mutable WASM-host state behind `with_runtime`, handles, stash and live-VM pointer live in `crate::bridge`. */
pub(super) struct WasmRuntime {
    pub src: [u8; SZ],
    pub out: [u8; SZ],
    pub inp: [u8; SZ],
    pub inp_len: usize,
    pub registry: Vec<(String, ModuleEntry)>,
    pub manifests: Vec<(String, Manifest)>,
    /* Entry dir rooting the source's quoted imports. */
    pub entry_dir: String,
    /* Last `save_state` blob, read via `snapshot_ptr`. */
    pub snapshot: Vec<u8>,
    /* Owned across `run_start` / `run_resume`, mutually exclusive with the bridge's `current_vm`. */
    pub paused_run: Option<Box<PausedRun>>,
    /* REPL, the interpreter kept alive between `repl_eval` inputs. */
    pub repl_vm: Option<Box<VM<'static>>>,
    pub repl_mode: bool,
    /* Back-edges between preempt yields, 0 disables. */
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
            entry_dir: String::new(),
            snapshot: Vec::new(),
            paused_run: None,
            repl_vm: None,
            repl_mode: false,
            preempt_every: 0,
        }
    }
}

static mut RUNTIME: WasmRuntime = WasmRuntime::new();

// SAFETY single-threaded WASM, re-entrant callers route through `with_vm` to drop the borrow first.
pub(super) fn with_runtime<R>(f: impl FnOnce(&mut WasmRuntime) -> R) -> R {
    unsafe { f(&mut *core::ptr::addr_of_mut!(RUNTIME)) }
}

pub(super) unsafe fn write_out(s: &str) -> usize {
    let b = s.as_bytes();
    let n = b.len().min(SZ);
    with_runtime(|rt| rt.out[..n].copy_from_slice(&b[..n]));
    n
}
