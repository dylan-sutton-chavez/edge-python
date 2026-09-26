extern crate alloc;

#[path = "src/host/config.rs"]
mod config;

#[allow(dead_code)]
#[path = "../src/util/sha256.rs"]
mod sha256;

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

// The StarlingMonkey release the JavaScript runtime is built from, starlingmonkey-v0.3.0.
const STARLING_SHA256: &str = "b5707b9d97164e0c29e471844a9ccdd81c445a5d379a9299ae2ee7a9dab3aabe";

fn main() {
    println!("cargo:rerun-if-env-changed=EDGE_COMPILER_WASM");
    println!("cargo:rerun-if-env-changed=EDGE_STARLING_WASM");
    println!("cargo:rerun-if-env-changed=EDGE_JS_DIST");
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
    precompile(&engine, &compiler, &out.join("compiler.cwasm"), "run cargo wasm first", |e, b| e.precompile_module(b));
    // A browser runs the module itself, never the precompile, so a dist and a headless run carry a raw copy.
    std::fs::copy(&compiler, out.join("compiler.wasm")).unwrap_or_else(|e| panic!("copying {}: {e}", compiler.display()));
    let js_dist = std::env::var("EDGE_JS_DIST").map(PathBuf::from).unwrap_or_else(|_| manifest.join("../js/dist"));
    js_host(&js_dist, &out.join("js_host.rs"));
    let starling = std::env::var("EDGE_STARLING_WASM").map(PathBuf::from).unwrap_or_else(|_| manifest.join("../target/starling.wasm"));
    js_runtime(&starling, &target, &out);
}

/* Precompiles the pinned StarlingMonkey for the target, the binary keeps only the artifact's hash. */
fn js_runtime(input: &Path, target: &str, out: &Path) {
    let hint = "fetch the pinned starling.wasm first, see CONTRIBUTING.md";
    let bytes = std::fs::read(input).unwrap_or_else(|e| panic!("cannot read {}: {e}, {hint}", input.display()));
    let got = sha256::hex_encode(&sha256::sha256(&bytes));
    assert!(got == STARLING_SHA256, "{} is sha256 {got}, not the pinned StarlingMonkey {STARLING_SHA256}", input.display());
    let mut cfg = config::base();
    // Linux builds share one artifact, Cranelift emits the same code for gnu and musl.
    let triple = match target.split_once("-unknown-linux-") {
        Some((arch, _)) => format!("{arch}-unknown-linux-gnu"),
        None => target.to_string(),
    };
    cfg.target(&triple).expect("wasmtime has no backend for the build target");
    cfg.cranelift_opt_level(wasmtime::OptLevel::Speed);
    let engine = wasmtime::Engine::new(&cfg).expect("wasmtime engine");
    let artifact = out.join("starling.cwasm");
    precompile(&engine, input, &artifact, hint, |e, b| e.precompile_component(b));
    let cwasm = std::fs::read(&artifact).unwrap_or_else(|e| panic!("reading {}: {e}", artifact.display()));
    let sha = sha256::hex_encode(&sha256::sha256(&cwasm));
    let consts = format!("// The sha256 of the precompiled runtime this binary downloads.\npub const JS_RUNTIME_SHA256: &str = {sha:?};\n");
    std::fs::write(out.join("js_runtime.rs"), consts).unwrap_or_else(|e| panic!("writing js_runtime.rs: {e}"));
    // A copy beside the profile's outputs, where staging picks it up for the CDN.
    let Some(dir) = out.ancestors().nth(3).map(|profile| profile.join("js-runtime")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if entry.file_name() != std::ffi::OsString::from(format!("{sha}.cwasm")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let staged = dir.join(format!("{sha}.cwasm"));
    if !staged.exists() {
        std::fs::write(&staged, &cwasm).unwrap_or_else(|e| panic!("writing {}: {e}", staged.display()));
    }
}

/* The compiled JS host the binary carries, keyed by the path the CDN serves each file at, so a page's imports read the same from a dist, from a headless run, or from the CDN. */
fn js_host(dist: &Path, out: &Path) {
    println!("cargo:rerun-if-changed={}", dist.display());
    let hint = "run tsc in js first, see CONTRIBUTING.md";
    let root = std::fs::canonicalize(dist).unwrap_or_else(|e| panic!("cannot read {}: {e}, {hint}", dist.display()));

    let mut found = Vec::new();
    walk_js(&root, &root, &mut found);
    found.sort();
    assert!(!found.is_empty(), "no .js under {}, {hint}", root.display());

    let mut table = String::from("pub const JS_HOST: &[(&str, &[u8])] = &[\n");
    for (key, path) in found {
        table.push_str(&format!("    ({key:?}, include_bytes!({:?})),\n", path.display().to_string()));
    }
    table.push_str("];\n");
    std::fs::write(out, table).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}

// tsc writes to js/dist and the CDN serves that tree under js/src, so the prefix is added back here.
fn walk_js(root: &Path, dir: &Path, found: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_js(root, &path, found);
        } else if path.extension().is_some_and(|e| e == "js") {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            found.push((format!("src/{rel}"), path));
        }
    }
}

fn precompile(engine: &wasmtime::Engine, input: &Path, output: &Path, hint: &str, compile: fn(&wasmtime::Engine, &[u8]) -> wasmtime::Result<Vec<u8>>) {
    println!("cargo:rerun-if-changed={}", input.display());
    let bytes = std::fs::read(input).unwrap_or_else(|e| panic!("cannot read {}: {e}, {hint}", input.display()));
    // A rerun over an unrelated file keeps the artifact while the input and the engine match.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let stamp = output.with_extension("key");
    if output.exists() && std::fs::read_to_string(&stamp).is_ok_and(|k| k == key) {
        return;
    }
    let cwasm = compile(engine, &bytes).unwrap_or_else(|e| panic!("precompiling {}: {e}", input.display()));
    std::fs::write(output, cwasm).unwrap_or_else(|e| panic!("writing {}: {e}", output.display()));
    std::fs::write(&stamp, key).unwrap_or_else(|e| panic!("writing {}: {e}", stamp.display()));
}
