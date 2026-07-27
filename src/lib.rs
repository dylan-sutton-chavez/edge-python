#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

pub mod abi;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/* Internal compiler helpers (not Edge Python stdlib); separate from pipeline code. */
pub mod util {
    pub mod fx;
    pub mod fstr;
    pub mod sha256;
}

pub mod lexer;
pub mod vm;
pub mod parser;
pub mod packages;
