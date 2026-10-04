use anyhow::{anyhow, bail, Result};
use compiler::modules::rules::shaped_like_version;
use serde_json::{Map, Value};
use std::io::Read;
use std::path::Path;

use crate::asks;
use crate::host::{cdn, get, site, system};
use crate::lock::{self, Entry, Lock};
use crate::manifest::Manifest;
use crate::ui;

// Bounds a runaway download while a url is hashed into the lock.
const MAX_FETCH_BYTES: u64 = 64 << 20;

/* What the registry says a name resolves to, its newest release unless one is named. A refresh says so, since only taking a package for the first time is worth counting. */
fn release(name: &str, version: Option<&str>, refresh: bool) -> Result<Entry> {
    let asked = [version.map(|v| format!("v={v}")), refresh.then(|| "lock=1".to_string())].into_iter().flatten().collect::<Vec<_>>().join("&");
    let query = match asked.is_empty() {
        true => String::new(),
        false => format!("?{asked}"),
    };
    let source = site(&format!("/api/resolve/package/{name}{query}"));

    let mut response = get(&source).map_err(|e| match (e, version) {
        (ureq::Error::StatusCode(404), Some(v)) => anyhow!("'{name}' has no version {v}"),
        (ureq::Error::StatusCode(404), None) => anyhow!("unknown package '{name}'; give a url with {name}=<url>"),
        (other, _) => anyhow!("asking the registry about '{name}': {other}"),
    })?;

    let text = response.body_mut().read_to_string().map_err(|e| anyhow!("reading {source}: {e}"))?;
    let answer: serde_json::Value = serde_json::from_str(&text).map_err(|e| anyhow!("parsing {source}: {e}"))?;
    let field = |key: &str| answer.get(key).and_then(|v| v.as_str()).map(str::to_string);

    match (field("version"), field("url"), field("digest")) {
        (Some(version), Some(url), Some(digest)) => Ok(Entry { version: Some(version), url, digest: format!("sha256-{digest}") }),
        _ => bail!("the registry sent no release for '{name}'"),
    }
}

pub fn add(path: &Path, pkgs: &[String]) -> Result<()> {
    if pkgs.is_empty() {
        bail!("nothing to add: pass one or more package names");
    }
    // Validate every spec first so a single unknown name aborts before any write or print.
    let mut releases = Lock::default();
    let resolved: Vec<(&str, String)> = pkgs
        .iter()
        .map(|spec| {
            let (name, version, url) = parse_spec(spec);
            if url.as_deref().is_some_and(javascript) {
                bail!("module '{name}' is JavaScript, ship a .py or a .wasm");
            }
            let target = match url {
                Some(url) => url,
                // A version is all the manifest keeps, `edge lock` is what turns it into an address.
                None => {
                    let entry = release(name, version, false)?;
                    let version = entry.version.clone().unwrap_or_default();
                    releases.insert(name, entry);
                    version
                }
            };
            Ok::<_, anyhow::Error>((name, target))
        })
        .collect::<Result<_>>()?;

    let added: Map<String, Value> = resolved.iter().map(|(name, target)| (name.to_string(), Value::String(target.clone()))).collect();
    let packages = asks::packages(path, &added, &releases)?;
    let mut m = Manifest::load(path)?;
    let versions = resolved.iter().any(|(_, target)| shaped_like_version(target));
    for (name, target) in resolved {
        ui::added(name, &target);
        m.imports.insert(name.to_string(), target);
    }
    let mut asking = false;
    for package in packages.iter().filter(|p| !p.section.is_null()) {
        // Against an empty grant every ask comes back unmet, which lists them all.
        let asked = system::unmet(&[], &package.section);
        if !asked.is_empty() {
            ui::asks(&package.who(), &asked);
            asking = true;
        }
    }
    m.save(path)?;
    ui::note(match (asking, versions) {
        (true, _) => "updated edge.json, grant what they ask for under permissions, then run edge lock",
        (false, true) => "updated edge.json, run edge lock to resolve it",
        (false, false) => "updated edge.json",
    });
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

/* Resolves every version and url the manifest declares and writes the lock a run reads, so nothing else has to ask the registry where a name points. */
pub fn lock(path: &Path) -> Result<()> {
    let manifest = Manifest::load(path)?;
    let mut lock = Lock::default();

    for (name, target) in &manifest.imports {
        if javascript(target) {
            bail!("module '{name}' is JavaScript, ship a .py or a .wasm");
        }
        let entry = match shaped_like_version(target) {
            true => release(name, Some(target), true)?,
            // A path carries its own bytes and a pinned url its own digest.
            false if !target.contains("://") || target.contains("#sha256-") => continue,
            false => Entry { version: None, url: target.clone(), digest: lock::digest_of(&download(target)?) },
        };
        ui::added(name, &entry.url);
        lock.insert(name, entry);
    }

    // Nothing is written until the root grants what every package in the tree asks for.
    asks::check(path, &lock)?;
    let written = lock::save(&lock, path)?;
    ui::note(&format!("wrote {}", written.display()));
    Ok(())
}

/* Whether a target is a JavaScript module, which no host loads besides its own. */
fn javascript(target: &str) -> bool {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    matches!(path.rsplit('.').next(), Some("js" | "mjs"))
}

/* The bytes at `url`, hashed into the lock so a later run can tell whether they ever changed. */
fn download(url: &str) -> Result<Vec<u8>> {
    let source = cdn(url);
    let mut response = get(&source).map_err(|e| anyhow!("fetching {source}: {e}"))?;
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(MAX_FETCH_BYTES).read_to_end(&mut bytes).map_err(|e| anyhow!("reading {source}: {e}"))?;
    Ok(bytes)
}

/// Parse `name`, `name@version` or `name=url`.
fn parse_spec(spec: &str) -> (&str, Option<&str>, Option<String>) {
    if let Some((name, url)) = spec.split_once('=') {
        return (name, None, Some(url.to_string()));
    }
    match spec.split_once('@') {
        Some((name, version)) => (name, Some(version), None),
        None => (spec, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_names_a_package_a_version_or_a_url() {
        assert_eq!(parse_spec("json"), ("json", None, None));
        assert_eq!(parse_spec("json@0.1.0"), ("json", Some("0.1.0"), None));
        assert_eq!(parse_spec("foo=https://x/foo.wasm"), ("foo", None, Some("https://x/foo.wasm".to_string())));
    }

    #[test]
    fn a_javascript_target_is_told_apart_from_the_modules_a_host_loads() {
        for target in ["https://x/charts.js", "https://x/charts.mjs", "https://x/time/index.js?v=2"] {
            assert!(javascript(target), "{target}");
        }
        for target in ["https://x/json.wasm", "https://x/pkg/json/0.1.0/app.edge", "https://x/helper.py"] {
            assert!(!javascript(target), "{target}");
        }
    }
}
