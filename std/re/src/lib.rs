/*
Edge Python `re` package. Exposes `match`/`search`/`fullmatch`/`findall`/`sub`/`groups`/`span` over the `wasm-pdk` ABI. A small backtracking engine, Unicode aware via std char predicates so it ships no Unicode tables.
*/

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

extern crate alloc;

wasm_pdk::module_fixed_pool!();

pub mod ast;
pub mod engine;
pub mod matcher;
pub mod parser;

/* Exports compile only for wasm32 so the engine stays native testable. */
#[cfg(target_arch = "wasm32")]
// Class exports follow the `__class_<Name>_<method>` ABI convention, not snake case.
#[allow(non_snake_case)]
mod wasm_api {
    use alloc::string::String;
    use alloc::vec::Vec;
    use wasm_pdk::*;
    use crate::engine::{self, Found, Mode, ReError};

    /* Routes engine errors to the matching host exception kind. */
    fn to_error(e: ReError) -> Error {
        match e {
            ReError::Syntax(m) => Error::Value(m), // bad pattern is a ValueError
            ReError::TooComplex(m) => Error::Runtime(m), // degradation is a RuntimeError
        }
    }

    fn rx<T>(r: core::result::Result<T, ReError>) -> Result<T> {
        r.map_err(to_error)
    }

    /* Compiled-pattern cache; every call path compiles a given pattern once. Capped so unbounded pattern churn stays bounded. */
    static PATTERNS: PluginCell<Vec<(String, engine::Regex)>> = PluginCell::new();

    fn with_compiled<T>(pattern: &str, f: impl FnOnce(&engine::Regex) -> Result<T>) -> Result<T> {
        let cache = PATTERNS.get_or_init(Vec::new);
        let idx = match cache.iter().position(|(p, _)| p == pattern) {
            Some(i) => i,
            None => {
                let re = rx(engine::Regex::compile(pattern))?;
                if cache.len() >= 64 { cache.remove(0); }
                cache.push((String::from(pattern), re));
                cache.len() - 1
            }
        };
        f(&cache[idx].1)
    }

    /* Shared op bodies; module functions and Pattern methods delegate here. */
    fn do_find(pattern: &str, string: &str, mode: Mode) -> Result<Option<String>> {
        with_compiled(pattern, |re| Ok(rx(engine::find_rx(re, string, mode))?.map(|f| f.text)))
    }

    fn do_findall(pattern: &str, string: &str) -> Result<Vec<Value>> {
        with_compiled(pattern, |re| {
            let (founds, ngroups) = rx(engine::find_all_rx(re, string))?;
            if ngroups <= 1 {
                return Ok(founds.iter().map(|f| Value::Bytes(pick(f, ngroups).into_bytes())).collect());
            }
            Ok(founds.iter().map(|f| Value::List(
                f.groups.iter().map(|g| Value::Bytes(g.clone().unwrap_or_default().into_bytes())).collect(),
            )).collect())
        })
    }

    fn do_groups(pattern: &str, string: &str) -> Result<Option<Vec<Value>>> {
        with_compiled(pattern, |re| {
            let Some(f) = rx(engine::find_rx(re, string, Mode::Search))? else { return Ok(None); };
            Ok(Some(f.groups.iter().map(|g| match g {
                Some(s) => Value::Bytes(s.clone().into_bytes()),
                None => Value::None,
            }).collect()))
        })
    }

    fn do_span(pattern: &str, string: &str) -> Result<Option<Vec<Value>>> {
        with_compiled(pattern, |re| {
            let Some(f) = rx(engine::find_rx(re, string, Mode::Search))? else { return Ok(None); };
            Ok(Some(alloc::vec![Value::Int(f.start as i128), Value::Int(f.end as i128)]))
        })
    }

    fn do_sub(pattern: &str, repl: &str, string: &str) -> Result<String> {
        with_compiled(pattern, |re| rx(engine::sub_rx(re, repl, string)))
    }

    /* search: leftmost match anywhere, returns group 0 or None. */
    #[plugin_fn]
    fn search(pattern: String, string: String) -> Result<Option<String>> {
        do_find(&pattern, &string, Mode::Search)
    }

    /* fullmatch: the pattern must consume the whole string. */
    #[plugin_fn]
    fn fullmatch(pattern: String, string: String) -> Result<Option<String>> {
        do_find(&pattern, &string, Mode::Full)
    }

    /* findall: list of matches, group shaped like CPython for zero or one group. One boundary crossing per call via the LIST transit. */
    #[plugin_fn]
    fn findall(pattern: String, string: String) -> Result<Vec<Value>> {
        do_findall(&pattern, &string)
    }

    /* groups: capture groups of the first match, or None. */
    #[plugin_fn]
    fn groups(pattern: String, string: String) -> Result<Option<Vec<Value>>> {
        do_groups(&pattern, &string)
    }

    /* span: codepoint start and end of the first match as a two element list. */
    #[plugin_fn]
    fn span(pattern: String, string: String) -> Result<Option<Vec<Value>>> {
        do_span(&pattern, &string)
    }

    /* sub: replace every match, expanding backreferences in the template. */
    #[plugin_fn]
    fn sub(pattern: String, repl: String, string: String) -> Result<String> {
        do_sub(&pattern, &repl, &string)
    }

    fn pick(f: &Found, ngroups: usize) -> String {
        if ngroups == 1 { f.groups[0].clone().unwrap_or_default() } else { f.text.clone() }
    }

    /* `re.compile(p)`: a native class named `compile`, so instantiation is the constructor call. Instances carry the source pattern; the compiled program lives in the cache. */
    #[plugin_fn]
    fn __class_compile___init__(self_h: Handle, pattern: String) -> Result<()> {
        // Bad patterns raise here, at compile time, like CPython.
        with_compiled(&pattern, |_| Ok(()))?;
        let ph = IntoValue::into_handle(pattern)?;
        self_h.set_attr("pattern", &ph)
    }

    fn self_pattern(self_h: &Handle) -> Result<String> {
        let h = self_h.get_attr("pattern")?;
        String::from_handle(h.raw())
    }

    #[plugin_fn]
    fn __class_compile_match(self_h: Handle, string: String) -> Result<Option<String>> {
        do_find(&self_pattern(&self_h)?, &string, Mode::Match)
    }

    #[plugin_fn]
    fn __class_compile_search(self_h: Handle, string: String) -> Result<Option<String>> {
        do_find(&self_pattern(&self_h)?, &string, Mode::Search)
    }

    #[plugin_fn]
    fn __class_compile_fullmatch(self_h: Handle, string: String) -> Result<Option<String>> {
        do_find(&self_pattern(&self_h)?, &string, Mode::Full)
    }

    #[plugin_fn]
    fn __class_compile_findall(self_h: Handle, string: String) -> Result<Vec<Value>> {
        do_findall(&self_pattern(&self_h)?, &string)
    }

    #[plugin_fn]
    fn __class_compile_groups(self_h: Handle, string: String) -> Result<Option<Vec<Value>>> {
        do_groups(&self_pattern(&self_h)?, &string)
    }

    #[plugin_fn]
    fn __class_compile_span(self_h: Handle, string: String) -> Result<Option<Vec<Value>>> {
        do_span(&self_pattern(&self_h)?, &string)
    }

    #[plugin_fn]
    fn __class_compile_sub(self_h: Handle, repl: String, string: String) -> Result<String> {
        do_sub(&self_pattern(&self_h)?, &repl, &string)
    }

    /* match: anchored at the start. Hand written export since `match` is a keyword. */
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub extern "C" fn r#match(argv: *const u32, argc: u32, out: *mut u32) -> i32 {
        if argc != 3 {
            __internals::stash_error(Error::Type(alloc::format!("match expects 2 positional args, got {}", argc - 1)));
            return 1;
        }
        let pattern = match String::from_handle(unsafe { *argv.add(0) }) {
            Ok(v) => v,
            Err(e) => { __internals::stash_error(e); return 1; }
        };
        let string = match String::from_handle(unsafe { *argv.add(1) }) {
            Ok(v) => v,
            Err(e) => { __internals::stash_error(e); return 1; }
        };
        let value = match do_find(&pattern, &string, Mode::Match) {
            Ok(v) => v,
            Err(e) => { __internals::stash_error(e); return 1; }
        };
        match IntoValue::into_handle(value) {
            Ok(h) => { unsafe { *out = h.into_raw(); } 0 }
            Err(e) => { __internals::stash_error(e); 1 }
        }
    }
}
