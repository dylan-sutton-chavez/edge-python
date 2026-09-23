include!(concat!(env!("OUT_DIR"), "/js_host.rs"));

pub const COMPILER_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compiler.wasm"));

#[cfg(test)]
mod tests {
    use super::*;

    /* A dist is only portable while the engine in it is the module a browser can instantiate, so an accidental precompile would pass every path check and fail in the page. */
    #[test]
    fn the_embedded_engine_is_a_wasm_module() {
        assert_eq!(&COMPILER_WASM[..4], b"\0asm");
    }

    // The keys are urls a page imports, so a missing prefix or a windows separator breaks a page without breaking a build.
    #[test]
    fn the_embedded_host_is_keyed_by_its_cdn_path() {
        assert!(!JS_HOST.is_empty());
        assert!(JS_HOST.iter().all(|(key, _)| key.starts_with("src/") && key.ends_with(".js") && !key.contains('\\')));
        assert!(JS_HOST.iter().any(|(key, _)| *key == "src/element.js"));
    }
}
