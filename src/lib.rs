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
    pub mod hash;
    pub mod fstr;
    pub mod sha256;
    /* RFC 6455 codec, shared by the native websocket client and the test mock. */
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub mod ws;
}

/* NaN-boxed values and the mark-and-sweep heap; the layer both the frontend and the VM build on. */
pub mod value;

pub mod lexer;
pub mod parser;
/* Post-SSA passes, run between parse and boot; touches no VM state. */
pub mod optimizer;
pub mod vm;
pub mod packages;
