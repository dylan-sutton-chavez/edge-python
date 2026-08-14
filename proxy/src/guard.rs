use crate::jmp::{self, JmpBuf};
use core::cell::Cell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

// run_plugin returns this when a trap aborted the call, any nonzero value works.
const SENTINEL: i32 = i32::MIN;

// The neutralized code ranges, a fixed atomic table the signal handler can read safely.
const MAX_RANGES: usize = 64;
static RANGES: [(AtomicUsize, AtomicUsize); MAX_RANGES] =
    [const { (AtomicUsize::new(0), AtomicUsize::new(0)) }; MAX_RANGES];
static RESERVED: AtomicUsize = AtomicUsize::new(0);
static PUBLISHED: AtomicUsize = AtomicUsize::new(0);

// Per-thread guard state, the resume frame, whether a guarded call is live, and if one was aborted.
thread_local! {
    static CUR: Cell<*mut u64> = const { Cell::new(core::ptr::null_mut()) };
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static BLOCKED: Cell<bool> = const { Cell::new(false) };
}

// Records a neutralized plugin range so the handler can tell a plugin trap from a host fault.
pub fn register_range(lo: usize, hi: usize) {
    let idx = RESERVED.fetch_add(1, Ordering::Relaxed);
    if idx >= MAX_RANGES {
        return;
    }
    RANGES[idx].0.store(lo, Ordering::Relaxed);
    RANGES[idx].1.store(hi, Ordering::Relaxed);
    // Publish in order so the handler never reads a half-written slot.
    while PUBLISHED
        .compare_exchange_weak(idx, idx + 1, Ordering::Release, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn in_plugin(addr: usize) -> bool {
    let n = PUBLISHED.load(Ordering::Acquire);
    for slot in RANGES.iter().take(n) {
        let lo = slot.0.load(Ordering::Relaxed);
        let hi = slot.1.load(Ordering::Relaxed);
        if addr >= lo && addr < hi {
            return true;
        }
    }
    false
}

// The trap a neutralized syscall raises lands here, plugin traps jump back and host faults recur.
extern "C" fn on_trap(sig: libc::c_int, info: *mut libc::siginfo_t, _ctx: *mut c_void) {
    let addr = unsafe { (*info).si_addr() } as usize;
    if ARMED.with(Cell::get) && in_plugin(addr) {
        BLOCKED.with(|b| b.set(true));
        let buf = CUR.with(Cell::get);
        unsafe { jmp::long_jump(buf, 1) };
    }
    unsafe { libc::signal(sig, libc::SIG_DFL) };
}

fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let mut sa: libc::sigaction = core::mem::zeroed();
        sa.sa_sigaction = on_trap as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGILL, &sa, core::ptr::null_mut());
        libc::sigaction(libc::SIGTRAP, &sa, core::ptr::null_mut());
    });
}

// Unblock the trapping signal by hand, the jump out of the handler skipped the kernel restore.
fn unblock() {
    unsafe {
        let mut set: libc::sigset_t = core::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGILL);
        libc::sigaddset(&mut set, libc::SIGTRAP);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, core::ptr::null_mut());
    }
}

/// Runs one plugin entry under the trap guard, returning the sentinel if a syscall was trapped.
///
/// # Safety
/// `f` must be a valid plugin entry whose code range was registered and neutralized.
pub unsafe fn run_plugin(
    f: unsafe extern "C" fn(*const u32, u32, *mut u32) -> i32,
    argv: *const u32,
    argc: u32,
    out: *mut u32,
) -> i32 {
    install();
    let mut buf: JmpBuf = jmp::empty();
    let prev_buf = CUR.with(|c| c.replace(buf.as_mut_ptr()));
    let prev_armed = ARMED.with(|a| a.replace(true));
    let ret = if unsafe { jmp::set_jump(buf.as_mut_ptr()) } == 0 {
        unsafe { f(argv, argc, out) }
    } else {
        // landed here from on_trap, the plugin tried a syscall
        unblock();
        SENTINEL
    };
    ARMED.with(|a| a.set(prev_armed));
    CUR.with(|c| c.set(prev_buf));
    ret
}

// Whether the last guarded call on this thread was aborted by the trap guard, clearing it.
pub fn take_block() -> bool {
    BLOCKED.with(Cell::take)
}
