use std::path::{Path, PathBuf};

pub enum JsonVal {
    Str(String),
    Map(Vec<(String, String)>),
}

// Runs registered tests, exit 3 flags an empty file.
pub const TEST_DRIVER: &str = "import test\nif not test._tests:\n    raise SystemExit(3)\ntest.run()";

pub const SCAFFOLD_MAIN_PY: &str = "print(\"hello from edge python\")\n";

// Bump with runtime element.ts and whenever the native plugin ABI changes.
pub const RUNTIME_CONTRACT: &str = "0.1.0";

pub const STD_PACKAGES: [&str; 5] = ["json", "re", "math", "struct", "test"];

pub const SYSTEM_PACKAGES: [&str; 4] = ["dom", "network", "storage", "time"];

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
    crate::util::jesc::escape(&mut out, s);
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
                    if self.peek() == Some(b'u') {
                        self.i += 1;
                        out.push(self.unicode_escape()?);
                        continue;
                    }
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        _ => return Err("unsupported escape".into()),
                    }
                    self.i += 1;
                }
                Some(_) => {
                    let start = self.i;
                    while !matches!(self.peek(), None | Some(b'"') | Some(b'\\')) { self.i += 1; }
                    // Spans between ASCII structurals in a &str are always valid UTF-8.
                    out.push_str(core::str::from_utf8(&self.b[start..self.i]).unwrap_or(""));
                }
            }
        }
    }

    /* Four hex digits after `\u`, surrogate pairs combine, lone surrogates read as U+FFFD. */
    fn unicode_escape(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) {
            if self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                let save = self.i;
                self.i += 2;
                let lo = self.hex4()?;
                if (0xDC00..0xE000).contains(&lo) {
                    return Ok(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)).unwrap_or('\u{FFFD}'));
                }
                self.i = save;
            }
            return Ok('\u{FFFD}');
        }
        Ok(char::from_u32(hi).unwrap_or('\u{FFFD}'))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = self.peek().and_then(|c| (c as char).to_digit(16))
                .ok_or_else(|| "expected 4 hex digits after \\u".to_string())?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }
}
