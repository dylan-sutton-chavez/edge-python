use anyhow::{anyhow, bail, Result};
use std::path::Path;

use crate::manifest::{registry, Kind, Manifest};
use crate::ui;

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
            Kind::System => {
                m.system.insert(name.to_string(), url);
                ui::added(name, "system");
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
        if !m.imports.contains_key(*name) && !m.system.contains_key(*name) {
            bail!("'{name}' is not in {}", path.display());
        }
    }
    for name in names {
        let _ = m.imports.remove(name).is_some() | m.system.remove(name).is_some();
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

/// A `.wasm` or `.py` url is a worker-side std package, anything else is a system package.
fn kind_from_url(url: &str) -> Kind {
    if url.ends_with(".wasm") || url.ends_with(".py") {
        Kind::Std
    } else {
        Kind::System
    }
}

