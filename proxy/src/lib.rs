mod gate;
mod guard;
mod jmp;
mod neutralize;

pub use gate::block_message;
pub use guard::{register_range, run_plugin, take_block};
pub use neutralize::neutralize;

// Seccomp sandbox for untrusted eval threads, an allowlist that fails any other syscall.
#[cfg(target_os = "linux")]
mod sandbox;

#[cfg(target_os = "linux")]
pub use sandbox::lock_thread;
