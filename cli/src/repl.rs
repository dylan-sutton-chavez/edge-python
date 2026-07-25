/*
Interactive REPL: a persistent engine session driven by rustyline; multi-line blocks via the `:` heuristic.
*/

use anyhow::Result;
use rustyline::{error::ReadlineError, DefaultEditor};
use std::path::Path;

use crate::engine::Session;
use crate::pkg::Manifest;

const PROMPT: &str = ">>> ";
const CONT: &str = "... ";

pub fn run(manifest_path: &Path) -> Result<()> {
    let manifest = Manifest::load(manifest_path)?;
    let mut session = Session::open(&manifest)?;
    println!("Edge Python {}  ·  .exit, Ctrl+C or Ctrl+D to quit", env!("CARGO_PKG_VERSION"));

    let mut rl = DefaultEditor::new()?;
    loop {
        let first = match rl.readline(PROMPT) {
            Ok(s) => s,
            Err(ReadlineError::Interrupted) => break, // Ctrl+C exits
            Err(ReadlineError::Eof) => break, // Ctrl+D exits
            Err(e) => { eprintln!("repl error: {e}"); break; }
        };
        let _ = rl.add_history_entry(first.as_str());
        let block = match read_block(&mut rl, first)? {
            BlockResult::Done(b) => b,
            BlockResult::Exit => break,
        };

        let trimmed = block.trim();
        if trimmed.is_empty() { continue; }
        match trimmed {
            ".exit" => break,
            ".reset" => {
                // Wipe runtime modules in place; the browser keeps running.
                session.reset()?;
                continue;
            }
            _ => {}
        }

        let outcome = session.eval(&block, None, crate::engine::emit_chunk)?;
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

enum BlockResult {
    Done(String),
    Exit
}

/// Collect a multi-line block when the first line ends with `:`; an empty line closes it.
fn read_block(rl: &mut DefaultEditor, first: String) -> Result<BlockResult> {
    if !first.trim_end().ends_with(':') {
        return Ok(BlockResult::Done(first));
    }
    let mut block = first;
    loop {
        match rl.readline(CONT) {
            Ok(line) if line.is_empty() => return Ok(BlockResult::Done(block)),
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                block.push('\n');
                block.push_str(&line);
            }
            Err(ReadlineError::Interrupted) => return Ok(BlockResult::Exit), // Ctrl+C exits
            Err(ReadlineError::Eof) => return Ok(BlockResult::Done(block)), // Ctrl+D submits the partial block
            Err(e) => return Err(e.into()),
        }
    }
}
