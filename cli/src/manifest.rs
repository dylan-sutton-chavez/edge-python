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
    Imports,
    System,
}

use compiler::devkit::{STD_PACKAGES as STD, SYSTEM_PACKAGES as SYSTEM};

/// Official package registry, the urls `edge add` writes for a bare name.
pub fn registry(name: &str) -> Option<(Kind, String)> {
    if STD.contains(&name) {
        Some((Kind::Imports, std_url(name)))
    } else if name == "dom" {
        // The facade is a .py module, its sibling manifest on the CDN supplies `_dom` by walk-up.
        Some((Kind::Imports, "https://cdn.edgepython.com/js/builtins/dom/entry.py".to_string()))
    } else if SYSTEM.contains(&name) {
        Some((Kind::System, format!("https://cdn.edgepython.com/js/builtins/{name}/index.js")))
    } else {
        None
    }
}

/// CDN url for a std package. Most ship as `.wasm`, `test` is pure Edge Python served as `.py`.
fn std_url(name: &str) -> String {
    let ext = if name == "test" { "py" } else { "wasm" };
    format!("https://cdn.edgepython.com/std/{name}.{ext}")
}
