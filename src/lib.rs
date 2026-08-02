#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

pub mod abi;

/* Host bridge shared by the WASM ABI and the native engine. */
#[cfg(any(target_arch = "wasm32", feature = "native"))]
pub mod bridge;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/* Dev-tooling constants and helpers shared by the edge CLI and the native engine. */
#[cfg(not(target_arch = "wasm32"))]
pub mod devkit;

/* The native engine, resolver, plugin loader, host modules and drive loop shared by every native front end. */
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;

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
