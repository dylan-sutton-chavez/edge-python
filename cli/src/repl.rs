/*
Interactive REPL: a persistent engine session driven by rustyline; one line, one eval.
*/

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::Editor;
use std::path::Path;

use crate::engine::Session;
use crate::pkg::Manifest;

const PROMPT: &str = ">>> ";

type Repl = Editor<(), DefaultHistory>;

pub fn run(manifest_path: &Path) -> Result<()> {
    let manifest = Manifest::load(manifest_path)?;
    let mut session = Session::open(&manifest)?;
    println!("Edge Python {}  ·  .reset to start fresh  ·  .exit, Ctrl+C or Ctrl+D to quit", env!("CARGO_PKG_VERSION"));

    let mut rl: Repl = Editor::new()?;
    loop {
        let line = match rl.readline(PROMPT) {
            Ok(s) => s,
            Err(ReadlineError::Interrupted) => break, // Ctrl+C exits
            Err(ReadlineError::Eof) => break, // Ctrl+D exits
            Err(e) => { eprintln!("repl error: {e}"); break; }
        };
        let _ = rl.add_history_entry(line.as_str());

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        match trimmed {
            ".exit" => break,
            ".reset" => {
                // Wipe runtime modules in place; the browser keeps running.
                session.reset()?;
                rl.clear_screen()?;
                continue;
            }
            _ => {}
        }

        let outcome = session.eval(&line, None, crate::engine::emit_chunk)?;
        // `raise SystemExit` quits the session with its code, matching the one-shot runner.
        if let Some(code) = outcome.exit_code {
            std::process::exit(code);
        }
        if let Some(err) = outcome.err {
            crate::ui::traceback(&err);
        }
    }
    Ok(())
}
