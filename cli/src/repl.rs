/*
Interactive REPL: a persistent engine session driven by rustyline; one line, one eval.
*/

use anyhow::Result;
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::engine::Session;
use crate::pkg::Manifest;

const PROMPT: &str = ">>> ";
const COMMANDS: &[&str] = &[".exit", ".reset"];

// Completion names; compiler sync enforced by unit test.
const LANGUAGE: &[&str] = &[
    "abs", "all", "and", "any", "as", "assert", "async", "await", "bin", "bool", "break", "bytes",
    "bytes_fromhex", "callable", "cancel", "case", "chr", "class", "classmethod", "continue", "def",
    "del", "delattr", "dict", "divmod", "elif", "else", "enumerate", "except", "False", "filter",
    "finally", "float", "for", "format", "frame", "from", "frozenset", "gather", "getattr", "global",
    "globals", "hasattr", "hash", "hex", "id", "if", "import", "import_module", "in", "input",
    "int", "int_from_bytes", "int_to_bytes", "is", "isinstance", "issubclass", "iter", "lambda",
    "len", "list", "locals", "map", "match", "max", "min", "next", "None", "nonlocal", "not",
    "oct", "or", "ord", "pass", "pow", "print", "property", "raise", "range", "receive", "repr",
    "return", "reversed", "round", "run", "set", "setattr", "sleep", "slice", "sorted",
    "staticmethod", "str", "sum", "super", "True", "try", "tuple", "type", "vars", "while", "with",
    "with_timeout", "yield", "zip",
];

type Repl = Editor<ReplHelper, DefaultHistory>;

pub fn run(manifest_path: &Path) -> Result<()> {
    let manifest = Manifest::load(manifest_path)?;
    let mut session = Session::open(&manifest)?;
    println!("Edge Python {}  ·  Tab to autocomplete  ·  .reset to start fresh  ·  .exit, Ctrl+C or Ctrl+D to quit", env!("CARGO_PKG_VERSION"));

    // Shared with the completer; refreshed after each eval.
    let names = Arc::new(Mutex::new(Vec::new()));
    let mut rl: Repl = Editor::new()?;
    rl.set_helper(Some(ReplHelper { names: names.clone() }));
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
                names.lock().unwrap().clear();
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
        if let Ok(globals) = session.globals() {
            *names.lock().unwrap() = globals.into_keys().collect();
        }
    }
    Ok(())
}

/* Tab completion over dot-commands, language names, session globals. */
struct ReplHelper {
    names: Arc<Mutex<Vec<String>>>,
}

impl Completer for ReplHelper {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<String>)> {
        let head = &line[..pos];
        let start = head
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_alphanumeric() && *c != '_' && *c != '.')
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = &head[start..];
        // A bare Tab stays inert instead of listing every name.
        if word.is_empty() {
            return Ok((pos, Vec::new()));
        }
        let mut out: Vec<String> = Vec::new();
        if start == 0 {
            out.extend(COMMANDS.iter().filter(|c| c.starts_with(word)).map(|c| c.to_string()));
        }
        out.extend(LANGUAGE.iter().filter(|n| n.starts_with(word)).map(|n| n.to_string()));
        out.extend(self.names.lock().unwrap().iter().filter(|n| n.starts_with(word)).cloned());
        out.sort();
        out.dedup();
        Ok((start, out))
    }
}

impl Hinter for ReplHelper { type Hint = String; }
impl Highlighter for ReplHelper {}
impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alphanumeric names inside `quote`-opened string literals.
    fn quoted(src: &str, quote: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = src;
        while let Some(i) = rest.find(quote) {
            rest = &rest[i + quote.len()..];
            let Some(end) = rest.find('"') else { break };
            let name = &rest[..end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                names.push(name.to_string());
            }
            rest = &rest[end + 1..];
        }
        names
    }

    #[test]
    fn language_list_tracks_compiler_tables() {
        let vm = std::fs::read_to_string("../src/modules/vm/types/mod.rs").unwrap();
        let builtins = vm.split("builtins! {").nth(1).unwrap().split("\n}").next().unwrap();
        let lexer = std::fs::read_to_string("../src/modules/lexer/tables.rs").unwrap();

        let mut missing: Vec<String> = quoted(builtins, "\"")
            .into_iter()
            .chain(quoted(&lexer, "b\""))
            .filter(|n| !LANGUAGE.contains(&n.as_str()))
            .collect();
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "add to LANGUAGE: {missing:?}");
    }
}
