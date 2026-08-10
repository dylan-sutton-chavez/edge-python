use std::process::Command;

// First working candidate wins, a local cli build beats a stale installed edge.
fn edge_binary() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("SKILL_EDGE") {
        candidates.push(p);
    }
    candidates.push(format!("{manifest}/../cli/target/release/edge"));
    candidates.push(format!("{manifest}/../cli/target/debug/edge"));
    candidates.push("edge".to_string());
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[test]
fn skill_md_native() {
    let Some(edge) = edge_binary() else {
        eprintln!("skipping, no edge binary found (set SKILL_EDGE or build cli/)");
        return;
    };
    let doc = concat!(env!("CARGO_MANIFEST_DIR"), "/SKILL.md");
    let out = Command::new(env!("CARGO_BIN_EXE_skill"))
        .arg(doc)
        .args(["--engine", "native", "--edge", &edge])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
