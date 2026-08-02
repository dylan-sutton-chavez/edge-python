use std::path::{Path, PathBuf};

pub enum JsonVal {
    Str(String),
    Map(Vec<(String, String)>),
}

// Runs registered tests, exit 3 flags an empty file.
pub const TEST_DRIVER: &str = "import test\nif not test._tests:\n    raise SystemExit(3)\ntest.run()";

pub const SCAFFOLD_MAIN_PY: &str = "print(\"hello from edge python\")\n";

pub const STD_PACKAGES: [&str; 5] = ["json", "re", "math", "struct", "test"];

pub const HOST_PACKAGES: [&str; 4] = ["dom", "network", "storage", "time"];

/* Collects *_test.py under `dir` recursively, skipping dist and hidden entries, sorted. */
pub fn discover_tests(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name == "dist" { continue; }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if name.ends_with("_test.py") {
            out.push(path);
        }
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

/* Tolerant reader for one json object whose values are strings or flat string maps. */
pub fn parse_object(text: &str) -> Result<Vec<(String, JsonVal)>, String> {
    let mut p = Cursor { b: text.as_bytes(), i: 0 };
    p.ws();
    p.eat(b'{', "expected '{'")?;
    let mut out = Vec::new();
    p.ws();
    if p.peek() == Some(b'}') { return Ok(out); }
    loop {
        p.ws();
        let key = p.string()?;
        p.ws();
        p.eat(b':', "expected ':' after key")?;
        p.ws();
        let val = match p.peek() {
            Some(b'"') => JsonVal::Str(p.string()?),
            Some(b'{') => {
                p.i += 1;
                let mut m = Vec::new();
                p.ws();
                if p.peek() == Some(b'}') { p.i += 1; } else {
                    loop {
                        p.ws();
                        let k = p.string()?;
                        p.ws();
                        p.eat(b':', "expected ':' in nested object")?;
                        p.ws();
                        let v = p.string()?;
                        m.push((k, v));
                        p.ws();
                        match p.peek() {
                            Some(b',') => { p.i += 1; }
                            Some(b'}') => { p.i += 1; break; }
                            _ => return Err("expected ',' or '}' in nested object".into()),
                        }
                    }
                }
                JsonVal::Map(m)
            }
            _ => return Err("only strings and string maps are supported".into()),
        };
        out.push((key, val));
        p.ws();
        match p.peek() {
            Some(b',') => { p.i += 1; }
            Some(b'}') => return Ok(out),
            _ => return Err("expected ',' or '}'".into()),
        }
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> { self.b.get(self.i).copied() }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) { self.i += 1; }
    }

    fn eat(&mut self, c: u8, msg: &str) -> Result<(), String> {
        if self.peek() == Some(c) { self.i += 1; Ok(()) } else { Err(msg.to_string()) }
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"', "expected '\"'")?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some(b'"') => { self.i += 1; return Ok(out); }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        _ => return Err("unsupported escape".into()),
                    }
                    self.i += 1;
                }
                Some(c) => { out.push(c as char); self.i += 1; }
            }
        }
    }
}
