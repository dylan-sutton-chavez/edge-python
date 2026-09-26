use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// A short description reads as one line in a listing, past this it is a readme.
const MAX_DESCRIPTION: usize = 60;

/* The manifest as `edge add` edits it, the registry fields, `imports`, and every other key kept as written. */
#[derive(Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub imports: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extends: Option<String>,
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
        manifest.check(path)?;
        Ok(manifest)
    }

    /* The registry fields, refused here so a build never writes what a registry would turn away. */
    fn check(&self, path: &Path) -> Result<()> {
        let at = path.display();
        if let Some(name) = &self.name
            && !named(name)
        {
            bail!("edge.json at '{at}': name '{name}' must be lowercase letters, digits and single hyphens, starting with a letter");
        }
        if let Some(version) = &self.version
            && !versioned(version)
        {
            bail!("edge.json at '{at}': version '{version}' must be major.minor.patch, digits only");
        }
        if let Some(description) = &self.description {
            let len = description.chars().count();
            if description.trim().is_empty() {
                bail!("edge.json at '{at}': description is empty");
            }
            if description.contains('\n') {
                bail!("edge.json at '{at}': description must be one line");
            }
            if len > MAX_DESCRIPTION {
                bail!("edge.json at '{at}': description is {len} characters, the cap is {MAX_DESCRIPTION}");
            }
        }
        if let Some(repository) = &self.repository
            && !linked(repository)
        {
            bail!("edge.json at '{at}': repository '{repository}' must be an https url a listing can link");
        }
        Ok(())
    }

    /// Write the manifest back as pretty JSON with a trailing newline.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))
    }
}

/* A name that reads the same in a url, an import and a listing. */
fn named(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/* An address a listing can open, so an ssh remote or a bare host is refused. */
fn linked(url: &str) -> bool {
    let Some(host) = url.strip_prefix("https://") else { return false };
    !host.is_empty() && url.len() <= 256 && !url.contains(char::is_whitespace)
}

/* Three numeric parts, no prerelease tags, so an ordering never depends on how a tag sorts. */
fn versioned(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(body: &str) -> Result<Manifest> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.json");
        std::fs::write(&path, body).unwrap();
        Manifest::load(&path)
    }

    #[test]
    fn the_registry_fields_round_trip() {
        let m = load(r#"{ "name": "slugify", "version": "0.1.0", "description": "Turn text into a slug.", "repository": "https://github.com/x/slugify", "docs": "./docs" }"#).unwrap();
        assert_eq!(m.name.as_deref(), Some("slugify"));
        assert_eq!(m.version.as_deref(), Some("0.1.0"));
        assert_eq!(m.repository.as_deref(), Some("https://github.com/x/slugify"));
        assert_eq!(m.docs.as_deref(), Some("./docs"));
    }

    #[test]
    fn a_manifest_without_the_registry_fields_still_loads() {
        let m = load(r#"{ "imports": { "json": "https://x/json.wasm" } }"#).unwrap();
        assert!(m.name.is_none() && m.version.is_none() && m.docs.is_none());
    }

    #[test]
    fn the_fields_a_registry_would_turn_away_are_refused() {
        for (body, want) in [
            (r#"{ "name": "Slugify" }"#, "must be lowercase"),
            (r#"{ "name": "1slug" }"#, "must be lowercase"),
            (r#"{ "name": "slug--ify" }"#, "must be lowercase"),
            (r#"{ "name": "slugify-" }"#, "must be lowercase"),
            (r#"{ "version": "1.0" }"#, "must be major.minor.patch"),
            (r#"{ "version": "1.0.0-rc1" }"#, "must be major.minor.patch"),
            (r#"{ "description": "  " }"#, "description is empty"),
            (r#"{ "description": "Turn absolutely any text that you have into a tidy url slug fast." }"#, "the cap is 60"),
            (r#"{ "repository": "git@github.com:x/slugify.git" }"#, "must be an https url"),
            (r#"{ "repository": "http://github.com/x/slugify" }"#, "must be an https url"),
            (r#"{ "repository": "github.com/x/slugify" }"#, "must be an https url"),
            (r#"{ "repository": "https://" }"#, "must be an https url"),
        ] {
            let Err(e) = load(body) else { panic!("{body} should be refused") };
            let err = format!("{e:#}");
            assert!(err.contains(want), "{body} wanted '{want}', got '{err}'");
        }
    }

    #[test]
    fn unknown_keys_survive_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.json");
        std::fs::write(&path, r#"{ "extends": "..", "imports": {}, "future": "kept" }"#).unwrap();
        Manifest::load(&path).unwrap().save(&path).unwrap();
        let back = std::fs::read_to_string(&path).unwrap();
        assert!(back.contains("\"extends\": \"..\""), "{back}");
        assert!(back.contains("\"future\": \"kept\""), "{back}");
    }
}
