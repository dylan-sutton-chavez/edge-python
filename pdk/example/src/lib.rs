/*
Reference `wasm-pdk` module showcasing #[plugin_class], #[plugin_const], and variadic Args. Build with `cargo build --release --target wasm32-unknown-unknown -p slugify-mod`.
*/

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::{String, ToString}, vec::Vec};
use wasm_pdk::*;

#[global_allocator]
static A: lol_alloc::LeakingPageAllocator = lol_alloc::LeakingPageAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { core::arch::wasm32::unreachable() }

/// Module constant materialised once at import. Showcases #[plugin_const].
#[plugin_const]
fn version() -> i64 { 1 }

/// Variadic join over `parts`, separated by `sep`. Showcases the trailing `Args` param.
#[plugin_fn]
fn join_all(sep: String, parts: Args) -> Result<String> {
    let mut out = String::new();
    for (i, h) in parts.0.iter().enumerate() {
        if i > 0 { out.push_str(&sep); }
        out.push_str(&String::from_handle(h.raw())?);
    }
    Ok(out)
}

/// Accumulates slug parts across calls; exercises mutable state and Option/Result returns.
#[plugin_class]
pub struct Slugger {
    parts: Vec<String>,
}

#[plugin_methods]
impl Slugger {
    #[plugin_ctor]
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    pub fn add(&mut self, s: String) {
        let normalized: String = s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        for part in normalized.split('-').filter(|p| !p.is_empty()) {
            self.parts.push(part.to_string());
        }
    }

    pub fn build(&self) -> String {
        self.parts.join("-")
    }

    pub fn shout(&self) -> String {
        let mut out = self.parts.join("-").to_uppercase();
        out.push('!');
        out
    }

    pub fn repeat(&self, n: i64) -> Result<String> {
        if n < 0 {
            return Err(Error::Value("repeat count must be non-negative".to_string()));
        }
        Ok(self.parts.join("-").repeat(n as usize))
    }

    pub fn total_len(&self) -> i64 {
        self.parts.iter().map(|p| p.len() as i64).sum()
    }

    pub fn pop(&mut self) -> Option<String> {
        self.parts.pop()
    }
}

impl Default for Slugger {
    fn default() -> Self {
        Self::new()
    }
}
