/*
Downloads `<links>.wasm` from GitHub release into OUT_DIR; path exposed via `DEP_<UPPERCASE(links)>_WASM`.
URL: `<repository>/releases/download/v<version>/<links>.wasm`. No-op on wasm32. Requires `curl`.
Gated on default-on `prebuilt` feature; producers opt out with `--no-default-features`.
*/

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }

    if env::var_os("CARGO_FEATURE_PREBUILT").is_none() {
        return;
    }

    let links = env::var("CARGO_MANIFEST_LINKS").expect("set `links` in [package] of Cargo.toml");
    let asset = format!("{links}.wasm");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let dst = out_dir.join(&asset);

    if !dst.exists() {
        let repo = env::var("CARGO_PKG_REPOSITORY")
            .expect("set `repository` in [package] of Cargo.toml")
            .trim_end_matches('/')
            .to_string();
        let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION not set");
        let url = format!("{repo}/releases/download/v{version}/{asset}"); // Include `/v{version}` to follow the GitHub tags format.

        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&dst)
            .arg(&url)
            .status()
            .expect("`curl` must be on PATH to fetch the pre-built wasm");

        if !status.success() {
            let _ = std::fs::remove_file(&dst);
            panic!(
                "failed to download {asset} from {url}\n\
                 hint: ensure the release tag v{version} exists with `{asset}` \
                 as an asset, or pre-place the binary at {}",
                dst.display()
            );
        }
    }

    println!("cargo::metadata=wasm={}", dst.display());
}
