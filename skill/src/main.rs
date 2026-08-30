mod cell;
mod markdown;
mod runner;

use cell::{Cell, Engine, Kind, Verdict};
use std::time::Duration;

const NATIVE_TIMEOUT: Duration = Duration::from_secs(30);
const ACTOR_TIMEOUT: Duration = Duration::from_secs(30);
const WEB_TIMEOUT: Duration = Duration::from_secs(150);

struct Opts {
    file: String,
    engine: Engine,
    edge: String,
}

fn parse_args() -> Result<Opts, String> {
    let mut file = None;
    let mut engine = Engine::Both;
    let mut edge = "edge".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine" => {
                let value = args.next().ok_or("--engine needs a value")?;
                engine = match value.as_str() {
                    "native" => Engine::Native,
                    "web" => Engine::Web,
                    "both" => Engine::Both,
                    other => return Err(format!("unknown engine '{other}'")),
                };
            }
            "--edge" => {
                edge = args.next().ok_or("--edge needs a value")?;
            }
            "-h" | "--help" => {
                return Err(
                    "usage: skill <file.md> [--engine native|web|both] [--edge <path>]"
                        .to_string(),
                );
            }
            other if other.starts_with('-') => return Err(format!("unknown flag '{other}'")),
            other => {
                if file.is_some() {
                    return Err("only one file may be given".to_string());
                }
                file = Some(other.to_string());
            }
        }
    }
    let file = file.ok_or("usage: skill <file.md> [--engine native|web|both] [--edge <path>]")?;
    Ok(Opts { file, engine, edge })
}

fn run_cell(opts: &Opts, c: &Cell) -> Vec<Result<(), String>> {
    let mut results = Vec::new();
    let native = matches!(opts.engine, Engine::Native | Engine::Both)
        && matches!(c.engine, Engine::Native | Engine::Both);
    let web = matches!(opts.engine, Engine::Web | Engine::Both)
        && matches!(c.engine, Engine::Web | Engine::Both)
        && c.kind == Kind::Python;

    if native {
        let outcome = match c.kind {
            Kind::Python => runner::run_native(&opts.edge, &c.body, NATIVE_TIMEOUT),
            Kind::Actor | Kind::Untrusted => runner::run_actor(&opts.edge, &c.body, ACTOR_TIMEOUT),
            Kind::PythonSkip => unreachable!("skip cells never become cells"),
        };
        results.push(outcome.and_then(|o| {
            runner::check(&c.expect, &o, c.verdict == Verdict::Error)
        }));
    }
    if web {
        let outcome = runner::run_web(&opts.edge, &c.body, WEB_TIMEOUT);
        results.push(match (c.engine, c.verdict) {
            (Engine::Both, Verdict::Output) => {
                // Dual-engine cells prove web support by succeeding, output is only compared on native.
                outcome.and_then(|o| {
                    if o.ok {
                        Ok(())
                    } else {
                        Err(format!("web run failed\n  stderr: {}", o.stderr.trim()))
                    }
                })
            }
            _ => outcome.and_then(|o| runner::check(&c.expect, &o, c.verdict == Verdict::Error)),
        });
    }
    results
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(if e.starts_with("usage:") { 0 } else { 2 });
        }
    };
    let src = match std::fs::read_to_string(&opts.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {e}", opts.file);
            std::process::exit(2);
        }
    };
    let cells = markdown::scan(&src)
        .and_then(|blocks| cell::collect(&blocks))
        .unwrap_or_else(|e| {
            eprintln!("{}: {e}", opts.file);
            std::process::exit(2);
        });

    let mut passed = 0;
    let mut failed = 0;
    for c in &cells {
        for result in run_cell(&opts, c) {
            match result {
                Ok(()) => passed += 1,
                Err(e) => {
                    failed += 1;
                    eprintln!("FAIL {}:{}\n{e}", opts.file, c.line);
                }
            }
        }
    }
    println!("{passed} passed, {failed} failed");
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
