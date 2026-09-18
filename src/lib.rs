#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod abi;

/* Host bridge behind the six plugin imports, wasm only. */
#[cfg(target_arch = "wasm32")]
pub mod bridge;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/* Dev-tooling constants and helpers shared with the edge CLI. */
#[cfg(feature = "std")]
pub mod devkit;

/* Internal compiler helpers (not Edge Python stdlib), separate from pipeline code. */
pub mod util {
    pub mod hash;
    pub mod fstr;
    pub mod jesc;
    pub mod sha256;
}

/* NaN-boxed values and the mark-and-sweep heap, the layer both the frontend and the VM build on. */
pub mod value;

pub mod lexer;
pub mod parser;
/* Post-SSA passes, run between parse and boot, touches no VM state. */
pub mod optimizer;
pub mod vm;
pub mod packages;
