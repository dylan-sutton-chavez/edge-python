use std::collections::BTreeMap;

use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch};

/* The syscalls a pure-compute interpreter needs to load a program, run it, and print, everything else fails. */
fn allowed() -> Vec<i64> {
    let nrs = [
        libc::SYS_mmap, libc::SYS_munmap, libc::SYS_mremap, libc::SYS_mprotect, libc::SYS_brk, libc::SYS_madvise,
        libc::SYS_read, libc::SYS_write, libc::SYS_readv, libc::SYS_writev, libc::SYS_pread64, libc::SYS_lseek, libc::SYS_close,
        libc::SYS_openat, libc::SYS_newfstatat, libc::SYS_statx, libc::SYS_fstat, libc::SYS_getdents64, libc::SYS_fcntl, libc::SYS_ioctl,
        libc::SYS_futex, libc::SYS_sched_yield, libc::SYS_sched_getaffinity, libc::SYS_get_robust_list, libc::SYS_set_robust_list,
        libc::SYS_clock_gettime, libc::SYS_clock_getres, libc::SYS_clock_nanosleep, libc::SYS_nanosleep, libc::SYS_gettimeofday,
        libc::SYS_getrandom,
        libc::SYS_rt_sigprocmask, libc::SYS_rt_sigaction, libc::SYS_rt_sigreturn, libc::SYS_sigaltstack,
        libc::SYS_getpid, libc::SYS_gettid, libc::SYS_getuid, libc::SYS_geteuid, libc::SYS_getgid, libc::SYS_getegid,
        libc::SYS_prlimit64, libc::SYS_exit, libc::SYS_exit_group,
        libc::SYS_ppoll, libc::SYS_epoll_pwait, libc::SYS_epoll_ctl, libc::SYS_epoll_create1,
    ];
    nrs.to_vec()
}

fn arch() -> TargetArch {
    #[cfg(target_arch = "aarch64")]
    { TargetArch::aarch64 }
    #[cfg(target_arch = "x86_64")]
    { TargetArch::x86_64 }
}

/* Locks the calling thread to the interpreter allowlist, any other syscall fails with EPERM so untrusted code cannot reach the kernel directly or through libc. Set EDGE_SANDBOX_TRACE=1 to log a blocked syscall instead of failing it, which tunes the allowlist. */
pub fn lock_thread() -> Result<(), String> {
    let mismatch = match std::env::var_os("EDGE_SANDBOX_TRACE") {
        Some(v) if v == "1" => SeccompAction::Log,
        _ => SeccompAction::Errno(libc::EPERM as u32),
    };
    let rules: BTreeMap<i64, Vec<SeccompRule>> = allowed().into_iter().map(|nr| (nr, Vec::new())).collect();
    let filter = SeccompFilter::new(rules, mismatch, SeccompAction::Allow, arch()).map_err(|e| e.to_string())?;
    let prog: BpfProgram = filter.try_into().map_err(|e| format!("{e}"))?;
    apply_filter(&prog).map_err(|e| e.to_string())?;
    Ok(())
}
