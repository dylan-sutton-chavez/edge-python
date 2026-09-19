use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// The name to url index `edge add` reads, generated from the std and js/builtins directories.
include!(concat!(env!("OUT_DIR"), "/registry.rs"));

/* The manifest as `edge add` edits it, `imports` plus every other key kept as written. */
#[derive(Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub imports: BTreeMap<String, String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

impl Manifest {
    /// Load the manifest, or an empty one when the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if manifest.rest.contains_key("system") {
            bail!("edge.json at '{}': move the system entries into imports", path.display());
        }
        Ok(manifest)
    }

    /// Write the manifest back as pretty JSON with a trailing newline.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))
    }
}

/// The url `edge add` writes for an official package name.
pub fn registry(name: &str) -> Option<&'static str> {
    REGISTRY.iter().find(|(n, _)| *n == name).map(|(_, url)| *url)
}
