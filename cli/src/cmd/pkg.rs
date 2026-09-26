use anyhow::{anyhow, bail, Result};
use std::path::Path;

use crate::host::{get, site};
use crate::manifest::Manifest;
use crate::ui;

/* The newest version of a published package, pinned to the digest the registry reports, so a build fails if those bytes ever change. */
fn published(name: &str) -> Result<String> {
    let source = site(&format!("/api/packages/{name}"));

    let mut response = get(&source).map_err(|e| match e {
        ureq::Error::StatusCode(404) => anyhow!("unknown package '{name}'; give a url with {name}=<url>"),
        other => anyhow!("asking the registry about '{name}': {other}"),
    })?;

    let text = response.body_mut().read_to_string().map_err(|e| anyhow!("reading {source}: {e}"))?;
    let answer: serde_json::Value = serde_json::from_str(&text).map_err(|e| anyhow!("parsing {source}: {e}"))?;

    let url = answer.get("url").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("the registry sent no url for '{name}'"))?;
    let digest = answer.get("digest").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("the registry sent no digest for '{name}'"))?;

    Ok(format!("{url}#sha256-{digest}"))
}

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
                None => published(name)?,
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
