/*
The engine seam. `web` drives a headless browser, `native` drives the in-process VM.
*/

use anyhow::Result;

pub mod native;
pub mod web;

pub use web::{base_dir, run, Session};

pub struct Outcome {
    pub err: Option<String>,
    pub exit_code: Option<i32>,
}

/// The engine seam: repl and test drive the browser session or the native VM through this.
pub trait Backend {
    fn eval(&mut self, src: &str, base: Option<&str>, on_line: &mut dyn FnMut(&str)) -> Result<Outcome>;
    fn reset(&mut self) -> Result<()>;
}

pub fn emit_chunk(chunk: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(chunk.as_bytes());
    let _ = out.flush();
}
