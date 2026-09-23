use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::docs::PREFIX;
use crate::host::site;
use crate::manifest::{declares, Manifest};
use crate::pack::Bundle;

// The modules only a browser can serve, so a project reaching for one runs nowhere else.
const BROWSER_ONLY: [&str; 2] = ["dom", "storage"];

/* Sends a packed `.edge` to the registry. The artifact travels whole and opaque, the metadata and the doc pages travel beside it, so the registry never has to learn the bundle format. */
pub fn run(artifact: &Path) -> Result<()> {
    let token = std::env::var("EDGE_TOKEN").map_err(|_| anyhow!("set EDGE_TOKEN to a token from edgepython.com/settings#tokens"))?;

    let bytes = fs::read(artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let bundle = Bundle::decode(&bytes).map_err(|e| anyhow!("{} is not an edge package: {e}", artifact.display()))?;

    let files = bundle.into_files();

    let declared = files
        .get("edge.json")
        .ok_or_else(|| anyhow!("{} carries no edge.json, so it has nothing to publish under", artifact.display()))?;
    let manifest: Manifest = serde_json::from_slice(declared).context("parsing the packed edge.json")?;

    let meta = Meta {
        description: manifest.description.clone(),
        repository: manifest.repository.clone(),
        notice: notice(&files),
        hosts: hosts(&manifest)
    };

    let name = manifest.name.ok_or_else(|| anyhow!("edge.json needs a name before it can be published"))?;
    let version = manifest.version.ok_or_else(|| anyhow!("edge.json needs a version before it can be published"))?;

    // The pages the bundle carries, keyed by the path the site orders them with.
    let docs: serde_json::Map<String, serde_json::Value> = files
        .iter()
        .filter_map(|(path, body)| path.strip_prefix(PREFIX).map(|page| (page, body)))
        .map(|(page, body)| (page.to_string(), serde_json::Value::String(String::from_utf8_lossy(body).into_owned())))
        .collect();

    let sp = crate::ui::spinner(&format!("publishing {name} {version}"));

    match send(&token, &name, &version, &meta, &bytes, &docs) {
        Ok(url) => {
            sp.done(&format!("published {name} {version}"));
            crate::ui::note(&format!("add it with  edge add {name}"));
            crate::ui::note(&url);
            Ok(())
        }
        Err(e) => {
            sp.fail(&format!("failed to publish {name} {version}"));
            Err(e)
        }
    }
}

/* What the artifact answers about itself, which the registry shows and never has to derive. */
struct Meta {
    description: Option<String>,
    repository: Option<String>,
    notice: Option<String>,
    hosts: String
}

/* The LICENSE the bundle carries, whatever its extension, so the registry names the license without guessing at a name. */
fn notice(files: &HashMap<String, Vec<u8>>) -> Option<String> {
    let (_, body) = files
        .iter()
        .find(|(path, _)| !path.contains('/') && path.split('.').next().is_some_and(|s| s.eq_ignore_ascii_case("LICENSE")))?;

    Some(String::from_utf8_lossy(body).into_owned())
}

/* Where a program can run, narrowed by the modules a browser alone can serve. */
fn hosts(manifest: &Manifest) -> String {
    match BROWSER_ONLY.iter().any(|name| declares(manifest, name)) {
        true => "web".to_string(),
        false => "cli,web,actor".to_string()
    }
}

/* One multipart request, the artifact as it sits on disk. */
fn send(
    token: &str,
    name: &str,
    version: &str,
    meta: &Meta,
    artifact: &[u8],
    docs: &serde_json::Map<String, serde_json::Value>
) -> Result<String> {
    let manifest = serde_json::json!({
        "name": name,
        "version": version,
        "description": meta.description,
        "repository": meta.repository,
        "notice": meta.notice,
        "hosts": meta.hosts
    });
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let boundary = format!("edge{nanos:x}");

    let mut body = Vec::new();
    part(&mut body, &boundary, "manifest", None, manifest.to_string().as_bytes());
    part(&mut body, &boundary, "docs", None, serde_json::Value::Object(docs.clone()).to_string().as_bytes());
    part(&mut body, &boundary, "artifact", Some("app.edge"), artifact);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let mut response = ureq::post(site("/api/publish").as_str())
        .header("authorization", &format!("Bearer {token}"))
        .header("content-type", &format!("multipart/form-data; boundary={boundary}"))
        .send(&body[..])
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => anyhow!("the registry refused it with {code}"),
            other => anyhow!("reaching the registry: {other}")
        })?;

    let text = response.body_mut().read_to_string().context("reading the registry's answer")?;
    let answer: serde_json::Value = serde_json::from_str(&text).context("parsing the registry's answer")?;

    match answer.get("url").and_then(|url| url.as_str()) {
        Some(url) => Ok(url.to_string()),
        None => bail!("{}", answer.get("error").and_then(|e| e.as_str()).unwrap_or("the registry sent no url"))
    }
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &str, filename: Option<&str>, value: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());

    match filename {
        Some(file) => body.extend_from_slice(
            format!("content-disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\ncontent-type: application/octet-stream\r\n\r\n").as_bytes()
        ),
        None => body.extend_from_slice(format!("content-disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes())
    }

    body.extend_from_slice(value);
    body.extend_from_slice(b"\r\n");
}
