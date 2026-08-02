/*
`edge init`: scaffold a ready-to-run project (entry script, host page, manifest).
*/

use anyhow::{bail, Context, Result};
use compiler::devkit::SCAFFOLD_MAIN_PY;
use std::fs;
use std::path::Path;

const PACKAGES_JSON: &str = "{}\n";
const INDEX_HTML: &str = include_str!("templates/scaffold.html");

fn index_html(title: &str) -> String {
    INDEX_HTML.replace("__EDGE_TITLE__", title)
}

pub fn run(name: Option<&str>, bare: bool) -> Result<()> {
    let dir = name.unwrap_or(".");
    let root = Path::new(dir);

    if dir != "." {
        if root.exists() {
            bail!("'{dir}' already exists");
        }
        fs::create_dir_all(root).with_context(|| format!("creating {dir}"))?;
    }

    fs::write(root.join("main.py"), SCAFFOLD_MAIN_PY)?;
    fs::write(root.join("packages.json"), PACKAGES_JSON)?;

    let mut items = vec![];
    if !bare {
        let title = if dir == "." { "edge app" } else { dir };
        fs::write(root.join("index.html"), index_html(title))?;
        items.push("index.html");
    }
    items.push("main.py");
    items.push("packages.json");

    let next = if dir == "." {
        "edge serve".to_string()
    } else {
        format!("cd {dir} && edge serve")
    };
    crate::ui::scaffolded(dir, &items, &next);
    Ok(())
}
