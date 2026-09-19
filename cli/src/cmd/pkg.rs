use anyhow::{anyhow, bail, Result};
use std::path::Path;

use crate::manifest::{registry, Manifest};
use crate::ui;

pub fn add(path: &Path, pkgs: &[String]) -> Result<()> {
    if pkgs.is_empty() {
        bail!("nothing to add: pass one or more package names");
    }
    // Validate every spec first so a single unknown name aborts before any write or print.
    let resolved: Vec<(&str, String)> = pkgs
        .iter()
        .map(|spec| {
            let (name, url_override) = parse_spec(spec);
            let url = match url_override {
                Some(u) => u,
                None => registry(name)
                    .ok_or_else(|| anyhow!("unknown package '{name}'; give a url with {name}=<url>"))?
                    .to_string(),
            };
            Ok::<_, anyhow::Error>((name, url))
        })
        .collect::<Result<_>>()?;

    let mut m = Manifest::load(path)?;
    for (name, url) in resolved {
        ui::added(name, &url);
        m.imports.insert(name.to_string(), url);
    }
    m.save(path)?;
    ui::note("updated edge.json");
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
        if !m.imports.contains_key(*name) {
            bail!("'{name}' is not in {}", path.display());
        }
    }
    for name in names {
        m.imports.remove(name);
        ui::removed(name);
    }
    m.save(path)?;
    ui::note("updated edge.json");
    Ok(())
}

/// Parse `name` or `name=url`.
fn parse_spec(spec: &str) -> (&str, Option<String>) {
    if let Some((name, url)) = spec.split_once('=') {
        return (name, Some(url.to_string()));
    }
    (spec, None)
}
