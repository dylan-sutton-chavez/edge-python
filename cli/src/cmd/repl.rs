use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::Editor;
use std::path::Path;

use crate::host::driver::Session;

const PROMPT: &str = ">>> ";

type Repl = Editor<(), DefaultHistory>;

/// A persistent interpreter driven by rustyline, one line per eval.
pub fn run(packages: Option<&Path>) -> Result<()> {
    let mut session = Session::open(packages)?;
    println!("Edge Python {}  ·  .reset to start fresh  ·  .exit, Ctrl+C or Ctrl+D to quit", env!("CARGO_PKG_VERSION"));

    let mut rl: Repl = Editor::new()?;
    loop {
        let line = match rl.readline(PROMPT) {
            Ok(s) => s,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("repl error: {e}");
                break;
            }
        };
        let _ = rl.add_history_entry(line.as_str());

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            ".exit" => break,
            ".reset" => {
                // Wipe the interpreter in place, the engine keeps running.
                session.reset()?;
                rl.clear_screen()?;
                continue;
            }
            _ => {}
        }

        let outcome = session.eval(&line, None, None)?;
        // `raise SystemExit` quits the session with its code, matching the one-shot runner.
        if let Some(code) = outcome.exit_code {
            drop(session);
            std::process::exit(code);
        }
        if let Some(err) = outcome.err {
            crate::ui::traceback(&err);
        }
    }
    Ok(())
}
