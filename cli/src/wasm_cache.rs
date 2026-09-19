use compiler::util::sha256::{hex_encode, sha256};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use wasmtime::{Engine, Module};

/* A third party plugin as native code, Cranelift compiles it once and later runs map the cache. */
pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Module, String> {
    match cache_dir() {
        Some(dir) => load_in(engine, bytes, &dir),
        None => compile(engine, bytes),
    }
}

fn load_in(engine: &Engine, bytes: &[u8], dir: &Path) -> Result<Module, String> {
    let path = dir.join(format!("{}.cwasm", key(engine, bytes)));
    // SAFETY the file is only ever written below from this engine's own serialization.
    if let Ok(module) = unsafe { Module::deserialize_file(engine, &path) } {
        return Ok(module);
    }
    // A missing, torn or foreign entry is a miss, the fresh compile replaces it.
    let module = compile(engine, bytes)?;
    if let Ok(native) = module.serialize() {
        let _ = store(dir, &path, &native);
    }
    Ok(module)
}

fn compile(engine: &Engine, bytes: &[u8]) -> Result<Module, String> {
    Module::new(engine, bytes).map_err(|e| format!("{e:#}"))
}

/* The content hash plus the engine's compatibility hash, a new wasmtime or config never hits. */
fn key(engine: &Engine, bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let mut input = hasher.finish().to_le_bytes().to_vec();
    input.extend_from_slice(bytes);
    hex_encode(&sha256(&input))
}

// Temp plus rename, a reader never maps a half written artifact.
fn store(dir: &Path, path: &Path, native: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, native)?;
    std::fs::rename(&tmp, path)
}

fn cache_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(base.join("edge").join("plugins"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Instance, Store};

    // (module (func (export "f") (result i32) i32.const 42)) assembled by hand.
    const ANSWER: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x05,
        0x01, 0x01, 0x66, 0x00, 0x00, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x2a, 0x0b,
    ];

    fn call_f(engine: &Engine, module: &Module) -> i32 {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, module, &[]).unwrap();
        instance.get_typed_func::<(), i32>(&mut store, "f").unwrap().call(&mut store, ()).unwrap()
    }

    #[test]
    fn the_first_load_compiles_and_the_second_maps_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::default();
        assert_eq!(call_f(&engine, &load_in(&engine, ANSWER, dir.path()).unwrap()), 42);
        let cached = dir.path().join(format!("{}.cwasm", key(&engine, ANSWER)));
        assert!(cached.exists());
        let stamp = std::fs::metadata(&cached).unwrap().modified().unwrap();
        assert_eq!(call_f(&engine, &load_in(&engine, ANSWER, dir.path()).unwrap()), 42);
        assert_eq!(std::fs::metadata(&cached).unwrap().modified().unwrap(), stamp, "a hit must not rewrite the entry");
    }

    #[test]
    fn a_corrupt_entry_is_a_miss_and_gets_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::default();
        let cached = dir.path().join(format!("{}.cwasm", key(&engine, ANSWER)));
        std::fs::write(&cached, b"not a wasmtime artifact").unwrap();
        assert_eq!(call_f(&engine, &load_in(&engine, ANSWER, dir.path()).unwrap()), 42);
        assert!(unsafe { Module::deserialize_file(&engine, &cached) }.is_ok());
    }

    #[test]
    fn the_key_follows_the_bytes() {
        let engine = Engine::default();
        let mut other = ANSWER.to_vec();
        other[32] = 0x07;
        assert_ne!(key(&engine, ANSWER), key(&engine, &other));
        assert_eq!(key(&engine, ANSWER), key(&engine, ANSWER));
    }

    #[test]
    fn invalid_bytes_fail_to_compile() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(&Engine::default(), b"\0asm\x01\0\0\0\x7f", dir.path()).unwrap_err();
        assert!(err.contains("WebAssembly"), "error was {err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
