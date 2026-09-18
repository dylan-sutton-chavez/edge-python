#[path = "src/host/config.rs"]
mod config;

use std::path::{Path, PathBuf};

const STD: [&str; 4] = ["json", "re", "math", "struct"];

fn main() {
    println!("cargo:rerun-if-env-changed=EDGE_COMPILER_WASM");
    println!("cargo:rerun-if-env-changed=EDGE_STD_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let target = std::env::var("TARGET").expect("TARGET");
    let mut cfg = config::base();
    cfg.target(&target).expect("wasmtime has no backend for the build target");
    cfg.cranelift_opt_level(wasmtime::OptLevel::Speed);
    let engine = wasmtime::Engine::new(&cfg).expect("wasmtime engine");
    let compiler = std::env::var("EDGE_COMPILER_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../target/wasm32-unknown-unknown/release/compiler.wasm"));
    precompile(&engine, &compiler, &out.join("compiler.cwasm"), "run cargo wasm first");
    for pkg in STD {
        precompile(&engine, &std_wasm(&manifest, pkg), &out.join(format!("{pkg}.cwasm")), &format!("build std/{pkg} first"));
    }
}

// The struct crate is named after a Rust keyword, so its artifact carries the edge prefix.
fn std_wasm(manifest: &Path, pkg: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("EDGE_STD_DIR") {
        return PathBuf::from(dir).join(format!("{pkg}.wasm"));
    }
    let release = manifest.join(format!("../std/{pkg}/target/wasm32-unknown-unknown/release"));
    let plain = release.join(format!("{pkg}.wasm"));
    if plain.exists() { plain } else { release.join(format!("edge_{pkg}.wasm")) }
}

fn precompile(engine: &wasmtime::Engine, input: &Path, output: &Path, hint: &str) {
    println!("cargo:rerun-if-changed={}", input.display());
    let bytes = std::fs::read(input).unwrap_or_else(|e| panic!("cannot read {}: {e}, {hint}", input.display()));
    let cwasm = engine.precompile_module(&bytes).unwrap_or_else(|e| panic!("precompiling {}: {e}", input.display()));
    std::fs::write(output, cwasm).unwrap_or_else(|e| panic!("writing {}: {e}", output.display()));
}
