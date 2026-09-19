#[path = "src/host/config.rs"]
mod config;

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const STD: [&str; 4] = ["json", "re", "math", "struct"];

fn main() {
    println!("cargo:rerun-if-env-changed=EDGE_COMPILER_WASM");
    println!("cargo:rerun-if-env-changed=EDGE_STD_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let target = std::env::var("TARGET").expect("TARGET");
    registry(&manifest.join(".."), &out.join("registry.rs"));
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

/* The name to url index `edge add` reads, one entry per package directory in std and js/builtins. */
fn registry(repo: &Path, out: &Path) {
    let (std, builtins) = (repo.join("std"), repo.join("js/builtins"));
    println!("cargo:rerun-if-changed={}", std.display());
    println!("cargo:rerun-if-changed={}", builtins.display());
    let mut entries = Vec::new();
    for (name, dir) in packages(&std) {
        if dir.join("Cargo.toml").exists() {
            entries.push((name, "std", "wasm".to_string()));
        } else if dir.join("src/entry.py").exists() {
            entries.push((name, "std", "py".to_string()));
        }
    }
    for (name, dir) in packages(&builtins) {
        // A Python facade is the entry when the library has one, its JavaScript sits behind it.
        for file in ["entry.py", "index.js"] {
            if dir.join("src").join(file).exists() {
                entries.push((name, "js/builtins", file.to_string()));
                break;
            }
        }
    }
    entries.sort();
    let mut table = String::from("const REGISTRY: &[(&str, &str)] = &[\n");
    for (name, tree, file) in entries {
        let url = match tree {
            "std" => format!("https://cdn.edgepython.com/std/{name}.{file}"),
            _ => format!("https://cdn.edgepython.com/js/builtins/{name}/{file}"),
        };
        table.push_str(&format!("    ({name:?}, {url:?}),\n"));
    }
    table.push_str("];\n");
    std::fs::write(out, table).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}

fn packages(tree: &Path) -> Vec<(String, PathBuf)> {
    let entries = std::fs::read_dir(tree).unwrap_or_else(|e| panic!("reading {}: {e}", tree.display()));
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .collect()
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
    // A rerun over an unrelated file keeps the artifact while the input and the engine match.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let stamp = output.with_extension("key");
    if output.exists() && std::fs::read_to_string(&stamp).is_ok_and(|k| k == key) {
        return;
    }
    let cwasm = engine.precompile_module(&bytes).unwrap_or_else(|e| panic!("precompiling {}: {e}", input.display()));
    std::fs::write(output, cwasm).unwrap_or_else(|e| panic!("writing {}: {e}", output.display()));
    std::fs::write(&stamp, key).unwrap_or_else(|e| panic!("writing {}: {e}", stamp.display()));
}
