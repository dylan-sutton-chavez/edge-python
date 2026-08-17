mod cmd;
mod engine;
mod manifest;
/// Minimalist terminal output, plain text only, no colors.
mod ui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use manifest::Manifest;

// Hand-written so the three top-level help forms print identically.
const HELP: &str = "\
The Edge Python developer CLI

Usage  edge <command> [options]

Commands
  run <file|.edge>   Run a script, a .edge, stdin or -c <code>
  build              Pack a standalone .edge  (--bundle, --web)
  swarm <file>       Run a swarm from swarm.yml
  serve              Dev server with live reload
  repl               Interactive shell
  test [path]        Run *_test.py files
  init <name>        Scaffold a new project
  add <pkgs>         Add packages to packages.json
  remove <pkgs>      Remove packages from packages.json
  uninstall          Remove the edge binary and PATH entry

Native run flags   --events <f>  --save-state <f>  --restore-state <f>  --preempt <n>
Global             --packages <file>   manifest, default packages.json
                   --web               browser runtime instead of native

edge <command> -h for details \u{00b7} -v for version \u{00b7} edgepython.com";

#[derive(Parser)]
#[command(name = "edge", disable_help_subcommand = true, color = clap::ColorChoice::Never)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Use a specific manifest instead of ./packages.json.
    #[arg(long, global = true)]
    packages: Option<PathBuf>,

    /// Drive the browser runtime instead of the in-process native engine.
    #[arg(long, global = true)]
    web: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a script.
    Run {
        /// Script, packed .edge or .package, or stdin when omitted.
        file: Option<PathBuf>,
        /// Run this code inline instead of a file or stdin.
        #[arg(short = 'c', conflicts_with = "file")]
        code: Option<String>,
        /// Feed each line of this file (or FIFO) into one receive() call. Native only.
        #[arg(long)]
        events: Option<PathBuf>,
        /// Snapshot to this file when the script suspends on an unservable wait. Native only.
        #[arg(long)]
        save_state: Option<PathBuf>,
        /// Boot from a snapshot instead of a script and keep running. Native only.
        #[arg(long)]
        restore_state: Option<PathBuf>,
        /// Yield every n loop back-edges and resume. Native only.
        #[arg(long)]
        preempt: Option<usize>,
    },
    /// Interactive shell. Ctrl+C, Ctrl+D, or .exit to quit.
    Repl,
    /// Dev server with live reload.
    Serve {
        /// Bind address, use 0.0.0.0 to expose on your LAN.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on.
        #[arg(long, default_value_t = 5173)]
        port: u16,
        /// Open the app in a browser once the server is up.
        #[arg(long)]
        open: bool,
    },
    /// Run *_test.py files.
    Test {
        /// Directory or file to run, the tree is searched when omitted.
        path: Option<PathBuf>,
    },
    /// Scaffold a new project.
    Init {
        /// Project directory to create, the current one when omitted.
        name: Option<String>,
        /// Skip the browser index.html, scaffold only main.py and packages.json.
        #[arg(long)]
        bare: bool,
    },
    /// Add packages to packages.json.
    Add {
        /// Package names to add.
        pkgs: Vec<String>,
    },
    /// Remove packages from packages.json.
    Remove {
        /// Package names to remove.
        pkgs: Vec<String>,
    },
    /// Pack the app, a standalone binary by default, --bundle for a swarm, --web for the browser.
    Build {
        /// Output path, defaults to app.edge, app.package, or dist/ per mode.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Vendor the browser runtime into dist/ instead of a native artifact.
        #[arg(long)]
        web: bool,
        /// Emit a lightweight .package for a swarm that already ships the runtime.
        #[arg(long)]
        bundle: bool,
    },
    /// Remove the edge binary, its PATH entry, and optionally the bundled browser cache.
    Uninstall,
    /// Run a swarm of nodes from a swarm.yml manifest.
    Swarm {
        /// Path to the swarm.yml manifest.
        file: PathBuf,
    },
}

fn main() -> Result<()> {
    ctrlc::set_handler(|| std::process::exit(130)).ok();

    // A standalone .edge carries its project, run that instead of parsing subcommands.
    if let Some(payload) = cmd::build::embedded_payload() {
        let result = run_embedded(&payload);
        if let Err(e) = result {
            ui::error(&e);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Only the top-level help is intercepted, `edge run -h` still falls through to clap.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!("{HELP}");
        return Ok(());
    }
    if matches!(args.first().map(String::as_str), Some("-v" | "--version")) {
        println!("edge {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let cli = Cli::parse();

    let manifest_path = cli.packages.clone().unwrap_or_else(|| PathBuf::from("packages.json"));

    let result = match cli.cmd {
        Cmd::Init { name, bare } => cmd::init::run(name.as_deref(), bare),
        Cmd::Add { pkgs } => cmd::pkg::add(&manifest_path, &pkgs),
        Cmd::Remove { pkgs } => cmd::pkg::remove(&manifest_path, &pkgs),
        Cmd::Serve { host, port, open } => cmd::serve::run(PathBuf::from("."), &host, port, open),
        Cmd::Run { file, code, events, save_state, restore_state, preempt } => {
            if cli.web {
                if events.is_some() || save_state.is_some() || restore_state.is_some() || preempt.is_some() {
                    Err(anyhow::anyhow!("--events, --save-state, --restore-state and --preempt are native-only; drop --web"))
                } else {
                    run_script(&manifest_path, file.as_deref(), code)
                }
            } else {
                let opts = compiler::native::RunOpts {
                    packages: cli.packages.as_deref().map(|p| p.to_string_lossy().replace('\\', "/")),
                    preempt: preempt.unwrap_or(0),
                    events: events.map(|p| p.to_string_lossy().into_owned()),
                    save_state: save_state.map(|p| p.to_string_lossy().into_owned()),
                    restore_state: restore_state.map(|p| p.to_string_lossy().into_owned()),
                };
                engine::native::run(file.as_deref(), code.as_deref(), &opts).map(|code| {
                    if code != 0 { std::process::exit(code) }
                })
            }
        }
        Cmd::Repl => cmd::repl::run(&manifest_path, cli.packages.as_deref(), cli.web),
        Cmd::Build { out, web, bundle } => {
            if web {
                cmd::build::run(&manifest_path, out.unwrap_or_else(|| PathBuf::from("dist")))
            } else if bundle {
                cmd::build::bundle(&manifest_path, out.unwrap_or_else(|| PathBuf::from("app.package")))
            } else {
                cmd::build::standalone(&manifest_path, out.unwrap_or_else(|| PathBuf::from("app.edge")))
            }
        }
        Cmd::Uninstall => cmd::uninstall::run(),
        Cmd::Swarm { file } => cmd::swarm::run(&file),
        Cmd::Test { path } => cmd::test::run(&manifest_path, cli.packages.as_deref(), cli.web, path.as_deref()),
    };

    if let Err(e) = result {
        ui::error(&e);
        std::process::exit(1);
    }
    Ok(())
}

/// The run flags a standalone .edge understands, mirroring `edge run`.
#[derive(Parser)]
#[command(name = "edge-app", disable_help_flag = true)]
struct Embedded {
    #[arg(long)]
    save_state: Option<PathBuf>,
    #[arg(long)]
    restore_state: Option<PathBuf>,
    #[arg(long)]
    preempt: Option<usize>,
    #[arg(long)]
    events: Option<PathBuf>,
}

/// Runs the project embedded in this standalone .edge, honoring the run flags.
fn run_embedded(payload: &[u8]) -> Result<()> {
    let flags = Embedded::parse();
    let opts = compiler::native::RunOpts {
        packages: None,
        preempt: flags.preempt.unwrap_or(0),
        events: flags.events.map(|p| p.to_string_lossy().into_owned()),
        save_state: flags.save_state.map(|p| p.to_string_lossy().into_owned()),
        restore_state: flags.restore_state.map(|p| p.to_string_lossy().into_owned()),
    };
    let code = engine::native::run_bundle(payload, &opts)?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Read a script from `code`, `file` or stdin (last resort) and run it, a script that raises exits non-zero.
fn run_script(manifest_path: &Path, file: Option<&Path>, code: Option<String>) -> Result<()> {
    let from_pipe = code.is_none() && file.is_none();
    let src = match (code, file) {
        (Some(c), _) => c,
        (None, Some(p)) => std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        (None, None) => {
            // A bare `edge run` from a terminal would block on stdin forever, force an explicit pipe or path.
            if std::io::stdin().is_terminal() {
                bail!("no script given; pass a file path or pipe Python to stdin");
            }
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).context("reading stdin")?;
            s
        }
    };
    // Unless the script itself came from stdin, piped stdin feeds `input()`.
    let mut input = String::new();
    let input = if !from_pipe && !std::io::stdin().is_terminal() && std::io::stdin().read_to_string(&mut input).is_ok() && !input.is_empty() {
        Some(input)
    } else {
        None
    };
    let manifest = Manifest::load(manifest_path)?;
    let base = file.and_then(engine::base_dir);
    let code = engine::run(&src, &manifest, base.as_deref(), input.as_deref())?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
