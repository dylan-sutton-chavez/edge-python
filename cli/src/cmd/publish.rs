use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;

use crate::host::site;

/* Sends a packed `.edge` to the registry. The artifact is the whole request, so the registry reads the name, the license and what the bundle carries out of the bytes that will run, and nothing here declares it a second time. */
pub fn run(artifact: &Path) -> Result<()> {
    let token = std::env::var("EDGE_TOKEN").map_err(|_| anyhow!("set EDGE_TOKEN to a token from edgepython.com/settings#tokens"))?;

    let bytes = fs::read(artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let name = artifact.file_name().unwrap_or(artifact.as_os_str()).to_string_lossy().into_owned();

    let sp = crate::ui::spinner(&format!("publishing {name}"));

    match send(&token, &bytes) {
        Ok(published) => {
            sp.done(&format!("published {} {}", published.name, published.version));
            crate::ui::note(&format!("add it with  edge add {}", published.name));
            crate::ui::note(&published.url);
            Ok(())
        }
        Err(e) => {
            sp.fail(&format!("failed to publish {name}"));
            Err(e)
        }
    }
}

/* What the registry made of the artifact, which is where the name and version come from now that it reads them itself. */
struct Published {
    name: String,
    version: String,
    url: String
}

/* The artifact as it sits on disk is the whole body. */
fn send(token: &str, artifact: &[u8]) -> Result<Published> {
    let mut response = ureq::post(site("/api/publish").as_str())
        .header("authorization", &format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .send(artifact)
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => anyhow!("the registry refused it with {code}"),
            other => anyhow!("reaching the registry: {other}")
        })?;

    let text = response.body_mut().read_to_string().context("reading the registry's answer")?;
    let answer: serde_json::Value = serde_json::from_str(&text).context("parsing the registry's answer")?;

    let field = |key: &str| answer.get(key).and_then(|v| v.as_str()).map(str::to_string);

    match (field("name"), field("version"), field("url")) {
        (Some(name), Some(version), Some(url)) => Ok(Published { name, version, url }),
        _ => bail!("{}", answer.get("error").and_then(|e| e.as_str()).unwrap_or("the registry sent no url"))
    }
}
