use core::ffi::c_void;

// libSystem flushes the instruction cache on Apple, libc does not expose it.
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
}

// The compiler runtime provides the cache flush on aarch64 Linux.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
unsafe extern "C" {
    fn __clear_cache(start: *mut c_void, end: *mut c_void);
}

/// Rewrites each syscall instruction in the range to a same-length trap and returns the count.
///
/// # Safety
/// `base` must point at `len` bytes of mapped plugin code that is safe to make writable.
pub unsafe fn neutralize(base: *mut u8, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let start = (base as usize) & !(page - 1);
    let end = (base as usize + len).div_ceil(page) * page;
    let span = end - start;
    let rc = unsafe { libc::mprotect(start as *mut c_void, span, libc::PROT_READ | libc::PROT_WRITE) };
    assert_eq!(rc, 0, "cannot make plugin code writable to neutralize it");

    let code = unsafe { core::slice::from_raw_parts_mut(base, len) };
    let count = patch(code);

    let rc = unsafe { libc::mprotect(start as *mut c_void, span, libc::PROT_READ | libc::PROT_EXEC) };
    assert_eq!(rc, 0, "cannot restore plugin code to read-execute");
    flush(base, len);
    count
}

// On x86-64 `syscall`, `sysenter`, and `int 0x80` each become the two-byte `ud2`, scanned unaligned.
#[cfg(target_arch = "x86_64")]
fn patch(code: &mut [u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i + 1 < code.len() {
        let hit = matches!((code[i], code[i + 1]), (0x0f, 0x05) | (0x0f, 0x34) | (0xcd, 0x80));
        if hit {
            code[i] = 0x0f;
            code[i + 1] = 0x0b;
            count += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    count
}

// On aarch64 instructions are fixed four-byte words, so every `svc` becomes `brk #0`.
#[cfg(target_arch = "aarch64")]
fn patch(code: &mut [u8]) -> usize {
    let mut count = 0;
    for word in code.chunks_exact_mut(4) {
        let w = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        // mask off the imm16 and match the svc opcode
        if w & 0xffe0_001f == 0xd400_0001 {
            word.copy_from_slice(&0xd420_0000u32.to_le_bytes());
            count += 1;
        }
    }
    count
}

fn flush(base: *mut u8, len: usize) {
    #[cfg(target_os = "macos")]
    unsafe {
        sys_icache_invalidate(base as *mut c_void, len);
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    unsafe {
        __clear_cache(base as *mut c_void, base.add(len) as *mut c_void);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let _ = (base, len); // x86 keeps instruction and data caches coherent
    }
}
