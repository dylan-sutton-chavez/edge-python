#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(special_module_name)]

extern crate alloc;

pub mod abi;

#[cfg(target_arch = "wasm32")]
pub mod main;

/* Internal compiler helpers (not Edge Python stdlib); separated from `modules/` runtime code. */
pub mod util {
    pub mod fx;
    pub mod fstr;
    pub mod sha256;
}

pub mod modules {
    pub mod lexer;
    pub mod vm;
    pub mod parser;
    pub mod packages;
}
