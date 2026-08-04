/*
`edge uninstall`: prompts here, then drops the bundled uninstall.sh to a temp file and runs it via bash.
*/

use anyhow::{anyhow, bail, Result};
use std::io::Write;
use std::process::Command;

// Bundled at compile time so the binary is self-contained; no network needed to clean up.
const UNINSTALL_SH: &str = include_str!("../../setup/uninstall.sh");

pub fn run() -> Result<()> {
    println!("This removes the edge binary and the PATH entry from your shell rc files.");
    println!("System browsers (apt/brew Chromium, Chrome) are never touched.");
    print!("Also remove the bundled chrome-headless-shell cache at ~/.cache/edge? [y/N] ");
    std::io::stdout().flush().ok();

    let mut ans = String::new();
    std::io::stdin().read_line(&mut ans).map_err(|e| anyhow!("reading answer: {e}"))?;
    let remove_browser = matches!(ans.trim(), "y" | "Y" | "yes" | "Yes" | "YES");

    // Spawning bash with the script as a file lets read prompts (none, in this path) work normally.
    let temp = std::env::temp_dir().join("edge-uninstall.sh");
    std::fs::write(&temp, UNINSTALL_SH).map_err(|e| anyhow!("staging uninstall script: {e}"))?;

    let mut cmd = Command::new("bash");
    cmd.arg(&temp);
    // Tell the script which prompt path the user already answered.
    cmd.env("EDGE_UNINSTALL_REMOVE_BROWSER", if remove_browser { "1" } else { "0" });
    // Point at the install dir derived from where this binary lives, so non-default installs still clean up.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            cmd.env("EDGE_INSTALL_DIR", dir);
        }

    let status = cmd.status().map_err(|e| anyhow!("running bash: {e}"))?;
    let _ = std::fs::remove_file(&temp);
    if !status.success() {
        bail!("uninstall script exited with code {:?}", status.code());
    }
    Ok(())
}
