/*
Edge Python `re` package. Exposes `match`/`search`/`fullmatch`/`findall`/`sub`/`groups`/`span` over the `wasm-pdk` ABI. A small backtracking engine, Unicode aware via std char predicates so it ships no Unicode tables.
*/

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]
#![allow(special_module_name)]

extern crate alloc;

wasm_pdk::module_fixed_pool!();

pub mod main;

/* Exports compile only for wasm32 so the engine stays native testable. */
#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use alloc::string::String;
    use alloc::vec::Vec;
    use wasm_pdk::*;
    use crate::main::{self, Found, Mode, ReError};

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

    /* search: leftmost match anywhere, returns group 0 or None. */
    #[plugin_fn]
    fn search(pattern: String, string: String) -> Result<Option<String>> {
        Ok(rx(main::find(&pattern, &string, Mode::Search))?.map(|f| f.text))
    }

    /* fullmatch: the pattern must consume the whole string. */
    #[plugin_fn]
    fn fullmatch(pattern: String, string: String) -> Result<Option<String>> {
        Ok(rx(main::find(&pattern, &string, Mode::Full))?.map(|f| f.text))
    }

    /* findall: list of matches, group shaped like CPython for zero or one group. One boundary crossing per call via the LIST transit. */
    #[plugin_fn]
    fn findall(pattern: String, string: String) -> Result<Vec<Value>> {
        let (founds, ngroups) = rx(main::find_all(&pattern, &string))?;
        if ngroups <= 1 {
            return Ok(founds.iter().map(|f| Value::Bytes(pick(f, ngroups).into_bytes())).collect());
        }
        Ok(founds.iter().map(|f| Value::List(
            f.groups.iter().map(|g| Value::Bytes(g.clone().unwrap_or_default().into_bytes())).collect(),
        )).collect())
    }

    /* groups: capture groups of the first match, or None. */
    #[plugin_fn]
    fn groups(pattern: String, string: String) -> Result<Option<Vec<Value>>> {
        let Some(f) = rx(main::find(&pattern, &string, Mode::Search))? else { return Ok(None); };
        Ok(Some(f.groups.iter().map(|g| match g {
            Some(s) => Value::Bytes(s.clone().into_bytes()),
            None => Value::None,
        }).collect()))
    }

    /* span: codepoint start and end of the first match as a two element list. */
    #[plugin_fn]
    fn span(pattern: String, string: String) -> Result<Option<Vec<Value>>> {
        let Some(f) = rx(main::find(&pattern, &string, Mode::Search))? else { return Ok(None); };
        Ok(Some(alloc::vec![Value::Int(f.start as i128), Value::Int(f.end as i128)]))
    }

    /* sub: replace every match, expanding backreferences in the template. */
    #[plugin_fn]
    fn sub(pattern: String, repl: String, string: String) -> Result<String> {
        rx(main::sub(&pattern, &repl, &string))
    }

    fn pick(f: &Found, ngroups: usize) -> String {
        if ngroups == 1 { f.groups[0].clone().unwrap_or_default() } else { f.text.clone() }
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
        let value = match main::find(&pattern, &string, Mode::Match) {
            Ok(v) => v.map(|f| f.text),
            Err(e) => { __internals::stash_error(to_error(e)); return 1; }
        };
        match IntoValue::into_handle(value) {
            Ok(h) => { unsafe { *out = h.into_raw(); } 0 }
            Err(e) => { __internals::stash_error(e); 1 }
        }
    }
}
