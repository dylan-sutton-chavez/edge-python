/*
Packages: the packages.json model, the official package registry, and the add/remove commands.
*/

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::ui;

/* The manifest: `imports` for worker-side .wasm/.py modules, `host` for main-thread JS libraries. */
#[derive(Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub imports: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host: BTreeMap<String, String>,
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
    fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))
    }
}

/* Official package registry. Mirrors the runtime's built-in default manifest. */
pub enum Kind {
    Std,
    Host,
}

use compiler::devkit::{HOST_PACKAGES as HOST, STD_PACKAGES as STD};

/// Resolve a bare name against the official registry; user manifest overrides go through `resolve`.
pub fn registry(name: &str) -> Option<(Kind, String)> {
    if STD.contains(&name) {
        Some((Kind::Std, std_url(name)))
    } else if HOST.contains(&name) {
        Some((Kind::Host, format!("https://cdn.edgepython.com/host/{name}/index.js")))
    } else {
        None
    }
}

/// CDN url for a std package. Most ship as `.wasm`; `test` is pure Edge Python, served as `.py`. Mirrors runtime/src/defaults.js.
fn std_url(name: &str) -> String {
    let ext = if name == "test" { "py" } else { "wasm" };
    format!("https://cdn.edgepython.com/std/{name}.{ext}")
}

/// Resolve `name` for the runtime: user manifest entry first, registry fallback.
pub fn resolve(name: &str, manifest: &Manifest) -> Option<(Kind, String)> {
    if let Some(url) = manifest.imports.get(name) {
        return Some((Kind::Std, url.clone()));
    }
    if let Some(url) = manifest.host.get(name) {
        return Some((Kind::Host, url.clone()));
    }
    registry(name)
}

pub fn add(path: &Path, pkgs: &[String]) -> Result<()> {
    if pkgs.is_empty() {
        bail!("nothing to add: pass one or more package names");
    }
    // Validate every spec first so a single unknown name aborts before any write or print.
    let resolved: Vec<(&str, Kind, String)> = pkgs
        .iter()
        .map(|spec| {
            let (name, url_override) = parse_spec(spec);
            let (kind, url) = match url_override {
                Some(u) => (kind_from_url(&u), u),
                None => registry(name)
                    .ok_or_else(|| anyhow!("unknown package '{name}'; give a url with {name}=<url>"))?,
            };
            Ok::<_, anyhow::Error>((name, kind, url))
        })
        .collect::<Result<_>>()?;

    let mut m = Manifest::load(path)?;
    for (name, kind, url) in resolved {
        match kind {
            Kind::Std => {
                m.imports.insert(name.to_string(), url);
                ui::added(name, "std");
            }
            Kind::Host => {
                m.host.insert(name.to_string(), url);
                ui::added(name, "host");
            }
        }
    }
    m.save(path)?;
    ui::note("updated packages.json");
    Ok(())
}

pub fn remove(path: &Path, pkgs: &[String]) -> Result<()> {
    if pkgs.is_empty() {
        bail!("nothing to remove: pass one or more package names");
    }
    let mut m = Manifest::load(path)?;
    let names: Vec<&str> = pkgs.iter().map(|s| parse_spec(s).0).collect();
    // Validate every name exists first so a single bad one aborts before any write or print.
    for name in &names {
        if !m.imports.contains_key(*name) && !m.host.contains_key(*name) {
            bail!("'{name}' is not in {}", path.display());
        }
    }
    for name in names {
        let _ = m.imports.remove(name).is_some() | m.host.remove(name).is_some();
        ui::removed(name);
    }
    m.save(path)?;
    ui::note("updated packages.json");
    Ok(())
}

/// Parse `name` or `name=url`.
fn parse_spec(spec: &str) -> (&str, Option<String>) {
    if let Some((name, url)) = spec.split_once('=') {
        return (name, Some(url.to_string()));
    }
    (spec, None)
}

/// A `.wasm` url is a worker-side std package; anything else is a host library.
fn kind_from_url(url: &str) -> Kind {
    if url.ends_with(".wasm") {
        Kind::Std
    } else {
        Kind::Host
    }
}
