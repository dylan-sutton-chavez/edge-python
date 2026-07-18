/*
Edge Python `struct` package. Format-string driven packing of values into fixed-width binary `bytes` and back.
*/

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]
#![allow(special_module_name)]

extern crate alloc;

wasm_pdk::module_fixed_pool!();

pub mod main;
