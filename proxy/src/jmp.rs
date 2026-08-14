// A hand-rolled setjmp buffer, sized for the larger aarch64 frame with headroom.
pub type JmpBuf = [u64; 24];

pub const fn empty() -> JmpBuf {
    [0; 24]
}

// Saves the current call frame into `buf` and returns zero, a later `long_jump` returns nonzero.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub unsafe extern "C" fn set_jump(buf: *mut u64) -> i32 {
    core::arch::naked_asm!(
        "mov [rdi], rbx",
        "mov [rdi + 8], rbp",
        "mov [rdi + 16], r12",
        "mov [rdi + 24], r13",
        "mov [rdi + 32], r14",
        "mov [rdi + 40], r15",
        "lea rax, [rsp + 8]", // caller rsp, past our return address
        "mov [rdi + 48], rax",
        "mov rax, [rsp]",
        "mov [rdi + 56], rax",
        "xor eax, eax",
        "ret",
    )
}

// Restores the frame saved in `buf` and resumes as if `set_jump` returned `val`, forced nonzero.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub unsafe extern "C" fn long_jump(buf: *const u64, val: i32) -> ! {
    core::arch::naked_asm!(
        "mov rbx, [rdi]",
        "mov rbp, [rdi + 8]",
        "mov r12, [rdi + 16]",
        "mov r13, [rdi + 24]",
        "mov r14, [rdi + 32]",
        "mov r15, [rdi + 40]",
        "mov rsp, [rdi + 48]",
        "mov rcx, [rdi + 56]",
        "mov eax, esi",
        "test eax, eax",
        "jnz 2f",
        "mov eax, 1",
        "2:",
        "jmp rcx",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn set_jump(buf: *mut u64) -> i32 {
    core::arch::naked_asm!(
        "stp x19, x20, [x0, #0]",
        "stp x21, x22, [x0, #16]",
        "stp x23, x24, [x0, #32]",
        "stp x25, x26, [x0, #48]",
        "stp x27, x28, [x0, #64]",
        "stp x29, x30, [x0, #80]",
        "mov x1, sp",
        "str x1, [x0, #96]",
        "stp d8, d9, [x0, #104]",
        "stp d10, d11, [x0, #120]",
        "stp d12, d13, [x0, #136]",
        "stp d14, d15, [x0, #152]",
        "mov w0, #0",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn long_jump(buf: *const u64, val: i32) -> ! {
    core::arch::naked_asm!(
        "ldp x19, x20, [x0, #0]",
        "ldp x21, x22, [x0, #16]",
        "ldp x23, x24, [x0, #32]",
        "ldp x25, x26, [x0, #48]",
        "ldp x27, x28, [x0, #64]",
        "ldp x29, x30, [x0, #80]",
        "ldr x2, [x0, #96]",
        "mov sp, x2",
        "ldp d8, d9, [x0, #104]",
        "ldp d10, d11, [x0, #120]",
        "ldp d12, d13, [x0, #136]",
        "ldp d14, d15, [x0, #152]",
        "cmp w1, #0",
        "csinc w0, w1, wzr, ne", // force a nonzero result when val is zero
        "ret",
    )
}
