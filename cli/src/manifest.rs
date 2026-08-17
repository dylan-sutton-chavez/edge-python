use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/* The manifest holds `imports` for worker-side .wasm/.py modules and `system` for main-thread JS libraries. */
#[derive(Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub imports: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system: BTreeMap<String, String>,
}

impl Manifest {
    /// Load the manifest, or an empty one when the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write the manifest back as pretty JSON with a trailing newline.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))
    }
}

pub enum Kind {
    Std,
    System,
}

use compiler::devkit::{STD_PACKAGES as STD, SYSTEM_PACKAGES as SYSTEM};

/// Official package registry. Mirrors the runtime's built-in default manifest, user overrides go through `resolve`.
pub fn registry(name: &str) -> Option<(Kind, String)> {
    if STD.contains(&name) {
        Some((Kind::Std, std_url(name)))
    } else if SYSTEM.contains(&name) {
        Some((Kind::System, format!("https://cdn.edgepython.com/web/builtins/{name}/index.js")))
    } else {
        None
    }
}

/// CDN url for a std package. Most ship as `.wasm`, `test` is pure Edge Python served as `.py`. Mirrors web/src/defaults.ts.
fn std_url(name: &str) -> String {
    let ext = if name == "test" { "py" } else { "wasm" };
    format!("https://cdn.edgepython.com/std/{name}.{ext}")
}

/// Resolve `name` for the runtime, user manifest entry first then the registry fallback.
pub fn resolve(name: &str, manifest: &Manifest) -> Option<(Kind, String)> {
    if let Some(url) = manifest.imports.get(name) {
        return Some((Kind::Std, url.clone()));
    }
    if let Some(url) = manifest.system.get(name) {
        return Some((Kind::System, url.clone()));
    }
    registry(name)
}
