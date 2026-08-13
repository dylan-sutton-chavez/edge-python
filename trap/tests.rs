use core::ffi::c_void;

type PluginFn = unsafe extern "C" fn(*const u32, u32, *mut u32) -> i32;

// Machine code for `return 42`, no syscall.
#[cfg(target_arch = "x86_64")]
const BENIGN: &[u8] = &[0xb8, 0x2a, 0x00, 0x00, 0x00, 0xc3]; // mov eax, 42 then ret
#[cfg(target_arch = "aarch64")]
const BENIGN: &[u8] = &[0x40, 0x05, 0x80, 0x52, 0xc0, 0x03, 0x5f, 0xd6]; // mov w0, #42 then ret

// Machine code that reaches a raw syscall then returns.
#[cfg(target_arch = "x86_64")]
const SYSCALL: &[u8] = &[0x0f, 0x05, 0xc3]; // syscall then ret
#[cfg(target_arch = "aarch64")]
const SYSCALL: &[u8] = &[0x01, 0x00, 0x00, 0xd4, 0xc0, 0x03, 0x5f, 0xd6]; // svc #0 then ret

// Copies `code` into fresh RW pages and hands back the base for neutralization.
fn write_code(code: &[u8]) -> *mut u8 {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let len = code.len().div_ceil(page) * page;
    let p = unsafe {
        libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0)
    };
    assert_ne!(p, libc::MAP_FAILED, "mmap failed: {}", std::io::Error::last_os_error());
    unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, code.len()) };
    p as *mut u8
}

#[test]
fn a_benign_plugin_returns_its_value() {
    let base = write_code(BENIGN);
    let n = unsafe { trap::neutralize(base, BENIGN.len()) };
    assert_eq!(n, 0, "a benign function has no syscall to neutralize");
    trap::register_range(base as usize, base as usize + BENIGN.len());
    let f: PluginFn = unsafe { core::mem::transmute(base) };
    let mut out = 0u32;
    let r = unsafe { trap::run_plugin(f, core::ptr::null(), 0, &mut out) };
    assert!(!trap::take_block(), "a benign function must not trip the guard");
    assert_eq!(r, 42, "the function's own return value must pass through");
}

#[test]
fn a_reachable_syscall_is_trapped_and_reported() {
    let base = write_code(SYSCALL);
    let n = unsafe { trap::neutralize(base, SYSCALL.len()) };
    assert_eq!(n, 1, "the one syscall instruction must be neutralized");
    trap::register_range(base as usize, base as usize + SYSCALL.len());
    let f: PluginFn = unsafe { core::mem::transmute(base) };
    let mut out = 0u32;
    let _ = unsafe { trap::run_plugin(f, core::ptr::null(), 0, &mut out) };
    assert!(trap::take_block(), "the reachable syscall must be trapped");
    assert!(trap::block_message().contains("move this to a system package"));
}

// Builds a cdylib, opens it, and cycles one loaded text page through the neutralize protections.
#[test]
fn loaded_library_text_can_be_reprotected() {
    let dir = std::env::temp_dir().join("edge_trap_reprotect");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("t.rs");
    std::fs::write(&src, "#[no_mangle]\npub extern \"C\" fn answer() -> i32 { 42 }\n").unwrap();
    let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
    let so = dir.join(format!("libt.{ext}"));
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let ok = std::process::Command::new(rustc)
        .args(["--edition", "2021", "--crate-type", "cdylib", "-O", "-o"])
        .arg(&so)
        .arg(&src)
        .status()
        .expect("spawn rustc");
    assert!(ok.success(), "rustc failed to build the fixture");

    let cpath = std::ffi::CString::new(so.to_string_lossy().as_bytes()).unwrap();
    let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW) };
    assert!(!handle.is_null(), "dlopen failed");
    let sym = unsafe { libc::dlsym(handle, c"answer".as_ptr()) };
    assert!(!sym.is_null(), "dlsym answer failed");

    // Toggle the page the symbol sits on through the same states neutralize uses.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let start = (sym as usize) & !(page - 1);
    let rw = unsafe { libc::mprotect(start as *mut c_void, page, libc::PROT_READ | libc::PROT_WRITE) };
    assert_eq!(rw, 0, "mprotect rw on loaded text failed: {}", std::io::Error::last_os_error());
    let rx = unsafe { libc::mprotect(start as *mut c_void, page, libc::PROT_READ | libc::PROT_EXEC) };
    assert_eq!(rx, 0, "mprotect rx on loaded text failed: {}", std::io::Error::last_os_error());

    let answer: unsafe extern "C" fn() -> i32 = unsafe { core::mem::transmute(sym) };
    assert_eq!(unsafe { answer() }, 42, "the reprotected function must still run");
}
