use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/* Points every test child at this checkout, so web builds read the local tree, never the CDN. */
pub fn local_tree() -> &'static [(&'static str, PathBuf)] {
    static HOOKS: OnceLock<Vec<(&'static str, PathBuf)>> = OnceLock::new();
    HOOKS.get_or_init(|| {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let or = |var: &str, local: PathBuf| std::env::var_os(var).map_or(local, PathBuf::from);
        vec![
            ("EDGE_JS_DIR", or("EDGE_JS_DIR", repo.join("js"))),
            ("EDGE_COMPILER_WASM", or("EDGE_COMPILER_WASM", repo.join("target/wasm32-unknown-unknown/release/compiler.wasm"))),
            ("EDGE_STD_DIR", or("EDGE_STD_DIR", std_dir(&repo))),
        ]
    })
}

// The flat name.wasm layout the CDN serves, copied from each std package build that exists.
fn std_dir(repo: &Path) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("edge-test-std-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for pkg in ["json", "re", "math", "struct"] {
        let release = repo.join(format!("std/{pkg}/target/wasm32-unknown-unknown/release"));
        for built in [release.join(format!("{pkg}.wasm")), release.join(format!("edge_{pkg}.wasm"))] {
            if std::fs::copy(&built, dir.join(format!("{pkg}.wasm"))).is_ok() {
                break;
            }
        }
    }
    let _ = std::fs::copy(repo.join("std/test/src/entry.py"), dir.join("test.py"));
    dir
}
