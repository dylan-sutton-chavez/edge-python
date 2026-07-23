/*
edge: the Edge Python developer CLI. Run, serve, test, and scaffold Edge Python apps.
*/

mod ui;
mod init;
mod pkg;
mod serve;
mod engine;
mod repl;
mod build;
mod uninstall;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use pkg::Manifest;

#[derive(Parser)]
#[command(name = "edge", version, about = "The Edge Python developer command line interface.", after_help = "Press Ctrl+C at any time to exit cleanly.", after_long_help = "Docs: https://edgepython.com/", color = clap::ColorChoice::Never)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Use a specific manifest instead of ./packages.json.
    #[arg(long, global = true)]
    packages: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a script.
    Run { file: Option<PathBuf> },
    /// Interactive shell. Ctrl+C, Ctrl+D, or .exit to quit.
    Repl,
    /// Dev server with live reload.
    Serve {
        #[arg(long, default_value_t = 5173)]
        port: u16,
        #[arg(long)]
        open: bool,
    },
    /// Run *_test.py files.
    Test { path: Option<PathBuf> },
    /// Scaffold a new project.
    Init {
        name: Option<String>,
        #[arg(long)]
        bare: bool,
    },
    /// Add packages to packages.json.
    Add { pkgs: Vec<String> },
    /// Remove packages from packages.json.
    Remove { pkgs: Vec<String> },
    /// Bundle the app into dist/.
    Build {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Remove the edge binary, its PATH entry, and optionally the bundled browser cache.
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    ctrlc::set_handler(|| std::process::exit(130)).ok();

    let manifest_path = cli.packages.unwrap_or_else(|| PathBuf::from("packages.json"));

    let result = match cli.cmd {
        Cmd::Init { name, bare } => init::run(name.as_deref(), bare),
        Cmd::Add { pkgs } => pkg::add(&manifest_path, &pkgs),
        Cmd::Remove { pkgs } => pkg::remove(&manifest_path, &pkgs),
        Cmd::Serve { port, open } => serve::run(PathBuf::from("."), port, open),
        Cmd::Run { file } => run_script(&manifest_path, file.as_deref()),
        Cmd::Repl => repl::run(&manifest_path),
        Cmd::Build { out } => build::run(&manifest_path, out.unwrap_or_else(|| PathBuf::from("dist"))),
        Cmd::Uninstall => uninstall::run(),
        // Last one standing: needs a `test` module + discovery + reporter on top of the engine.
        Cmd::Test { .. } => bail!("not wired yet: edge test"),
    };

    if let Err(e) = result {
        ui::error(&e);
        std::process::exit(1);
    }
    Ok(())
}

/// Read a script from `file` (or stdin when absent) and run it; a script that raises exits non-zero.
fn run_script(manifest_path: &Path, file: Option<&Path>) -> Result<()> {
    let src = match file {
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        None => {
            // A bare `edge run` from a terminal would block on stdin forever; force an explicit pipe or path.
            if std::io::stdin().is_terminal() {
                bail!("no script given; pass a file path or pipe Python to stdin");
            }
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).context("reading stdin")?;
            s
        }
    };
    let manifest = Manifest::load(manifest_path)?;
    let code = engine::run(&src, &manifest)?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
