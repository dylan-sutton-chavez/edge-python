/*
Edge Python `math` package. Where scalar and integer surface over `libm`, plus a packed-f64 batch fast path.
*/

#![no_std]
#![cfg_attr(target_arch = "wasm32", no_main)]

extern crate alloc;

wasm_pdk::module_fixed_pool!();

pub mod batch;
pub mod float;
pub mod int;
