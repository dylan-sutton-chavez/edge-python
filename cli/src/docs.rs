use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Component, Path};

// Every page lands under this prefix inside the bundle, so no import can ever resolve to one.
pub const PREFIX: &str = "@docs/";

/* Every page under the declared docs dir, keyed for the bundle, empty when the manifest declares none. */
pub fn collect(project: &Path, declared: Option<&str>) -> Result<Vec<(String, Vec<u8>)>> {
    let Some(declared) = declared else { return Ok(Vec::new()) };
    let root = project.join(relative(declared)?);
    if !root.is_dir() {
        bail!("edge.json declares docs at '{declared}' and no directory is there");
    }
    let mut found = pages(&root, "")?;
    if found.is_empty() {
        bail!("no .mdx pages under '{declared}', write one or drop the docs key");
    }
    found.sort();
    let mut out = Vec::with_capacity(found.len());
    for page in found {
        let bytes = fs::read(root.join(&page)).with_context(|| format!("reading {declared}/{page}"))?;
        let text = core::str::from_utf8(&bytes).map_err(|_| anyhow!("'{page}' is not valid UTF-8"))?;
        check(&page, text)?;
        out.push((format!("{PREFIX}{page}"), bytes));
    }
    Ok(out)
}

/* The declared path as a plain relative one, the same guard a bundle path gets on read. */
fn relative(declared: &str) -> Result<String> {
    let trimmed = declared.trim_start_matches("./").trim_end_matches('/');
    let plain = !trimmed.is_empty()
        && !declared.contains("://")
        && !declared.starts_with('/')
        && Path::new(trimmed).components().all(|c| matches!(c, Component::Normal(_)));
    if !plain {
        bail!("docs '{declared}' must be a relative path inside the project");
    }
    Ok(trimmed.to_string())
}

/* Page paths relative to the docs root, hidden entries skipped the way the project walk skips them. */
fn pages(dir: &Path, prefix: &str) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            found.extend(pages(&path, &format!("{prefix}{name}/"))?);
        } else if name.ends_with(".mdx") {
            found.push(format!("{prefix}{name}"));
        }
    }
    Ok(found)
}

/* The convention the site renderer relies on, so a package cannot ship docs the site cannot lay out. */
fn check(page: &str, text: &str) -> Result<()> {
    let segments: Vec<&str> = page.split('/').collect();
    if segments.len() > 2 {
        bail!("'{page}' nests deeper than one folder, a section and its pages is all the renderer orders");
    }
    if !segments.iter().copied().all(ordered) {
        bail!("'{page}' needs a numeric prefix on every segment, like '01-reference/02-cli.mdx'");
    }
    let mut open: Option<&str> = None;
    let mut closed: Option<&str> = None;
    let mut headings = 0usize;
    for line in front(page, text)?.lines() {
        if let Some(rest) = line.trim().strip_prefix("```") {
            match open.take() {
                Some(lang) => closed = Some(lang),
                None => {
                    let lang = rest.trim();
                    if lang == "output" && closed != Some("edge-python") {
                        bail!("'{page}' has an output block that follows no edge-python block");
                    }
                    open = Some(lang);
                    closed = None;
                }
            }
            continue;
        }
        if open.is_none() {
            if line.starts_with("# ") {
                headings += 1;
            }
            // Blank lines keep two fences adjacent, prose between them does not.
            if !line.trim().is_empty() {
                closed = None;
            }
        }
    }
    if open.is_some() {
        bail!("'{page}' leaves a code fence unterminated");
    }
    if headings != 1 {
        bail!("'{page}' has {headings} top-level headings, the renderer needs exactly one");
    }
    Ok(())
}

/* The page past its frontmatter, which names the page and describes it, and closes. */
fn front<'a>(page: &str, text: &'a str) -> Result<&'a str> {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        bail!("'{page}' opens with no frontmatter, a page needs a title and a description");
    };
    let mut at = 0usize;
    let mut named = false;
    let mut described = false;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed == "---" {
            if !named || !described {
                bail!("'{page}' needs both a title and a description in its frontmatter");
            }
            return Ok(&rest[at + line.len()..]);
        }
        if !trimmed.is_empty() {
            let Some((key, value)) = trimmed.split_once(':') else {
                bail!("'{page}' has the frontmatter line '{trimmed}', which is no key and value");
            };
            if value.trim().is_empty() {
                bail!("'{page}' leaves the frontmatter '{}' empty", key.trim());
            }
            match key.trim() {
                "title" => named = true,
                "description" => described = true,
                _ => {}
            }
        }
        at += line.len();
    }
    bail!("'{page}' leaves its frontmatter unterminated")
}

/* A segment the renderer can order, digits then a separator, `01-cli.mdx` or `02-reference`. */
fn ordered(segment: &str) -> bool {
    let rest = segment.trim_start_matches(|c: char| c.is_ascii_digit());
    rest.len() < segment.len() && (rest.starts_with('-') || rest.starts_with('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        files: BTreeMap<String, String>,
        #[serde(default)]
        error: Option<String>,
    }

    /* The corpus the site renderer shares, so neither side can change a rule on its own. */
    #[test]
    fn the_shared_corpus_agrees() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../tests/cases/docs.json")).expect("parsing docs.json");
        assert!(!cases.is_empty());
        for case in cases {
            let dir = tempfile::tempdir().unwrap();
            for (path, body) in &case.files {
                let at = dir.path().join("docs").join(path);
                fs::create_dir_all(at.parent().unwrap()).unwrap();
                fs::write(at, body).unwrap();
            }
            let pages = case.files.keys().filter(|p| p.ends_with(".mdx")).count();
            match (&case.error, collect(dir.path(), Some("./docs"))) {
                (None, Ok(got)) => assert_eq!(got.len(), pages, "{}", case.name),
                (None, Err(e)) => panic!("{} should pass, got {e:#}", case.name),
                (Some(want), Err(e)) => {
                    let got = format!("{e:#}");
                    assert!(got.contains(want), "{} wanted '{want}', got '{got}'", case.name);
                }
                (Some(want), Ok(_)) => panic!("{} should fail with '{want}'", case.name),
            }
        }
    }

    const PAGE: &str = "---\ntitle: Cli\ndescription: Every command.\n---\n\n# Cli\n";

    #[test]
    fn a_page_lands_under_the_reserved_prefix() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("docs/01-reference")).unwrap();
        fs::write(dir.path().join("docs/01-reference/01-cli.mdx"), PAGE).unwrap();
        let got = collect(dir.path(), Some("docs")).unwrap();
        assert_eq!(got[0].0, "@docs/01-reference/01-cli.mdx");
    }

    #[test]
    fn no_docs_key_collects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(collect(dir.path(), None).unwrap().is_empty());
    }

    #[test]
    fn a_path_outside_the_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../docs", "/etc", "https://example.com/docs", "./"] {
            let err = format!("{:#}", collect(dir.path(), Some(bad)).expect_err(bad));
            assert!(err.contains("must be a relative path"), "{bad} got '{err}'");
        }
    }
}
