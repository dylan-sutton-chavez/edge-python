use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::s;

/* Parsed `packages.json`. `imports` maps bare names to specs; `extends` inherits another manifest's imports when a name isn't local. */
// Vec not a map: parsed once, looked up linearly; avoids a hashbrown monomorphization.
#[derive(Clone)]
pub struct Manifest {
    pub imports: Vec<(String, String)>,
    pub extends: Option<String>,
}

/* Parse `{ "imports": {...}, "host": {...}, "extends": "..." }`. All optional; unknown keys skipped for forward compat; numbers, arrays, bools rejected. */
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let src = core::str::from_utf8(bytes).map_err(|_| s!("packages.json is not valid UTF-8"))?;
    let mut p = Reader { src: src.as_bytes(), pos: 0 };
    let mut m = Manifest { imports: Vec::new(), extends: None };

    p.skip_ws();
    p.expect(b'{', "packages.json must be a JSON object")?;
    p.skip_ws();
    if p.peek() == Some(b'}') { return Ok(m); }

    loop {
        p.skip_ws();
        let key = p.read_string()?;
        p.skip_ws();
        p.expect(b':', "expected ':' after key in packages.json")?;
        p.skip_ws();
        match key.as_str() {
            "imports" => p.read_imports_into(&mut m.imports)?,
            "host" => {
                let mut pairs = Vec::new();
                p.read_imports_into(&mut pairs)?;
                // Host names fold in as `mt:` specs; urls are runtime-side.
                m.imports.extend(pairs.into_iter().map(|(name, _)| {
                    let spec = s!("mt:", str &name);
                    (name, spec)
                }));
            }
            "extends" => m.extends = Some(p.read_string()?),
            _ => p.skip_value()?,
        }
        p.skip_ws();
        match p.peek() {
            Some(b',') => { p.pos += 1; continue; }
            Some(b'}') => return Ok(m),
            _ => return Err(s!("expected ',' or '}' in packages.json")),
        }
    }
}

/* Yield the directory of `start` and every parent, in order. Each ends in '/' or is "" (topmost). */
pub fn walk_up_dirs(start: &str) -> impl Iterator<Item = String> + '_ {
    let _ = start;
    let mut current = Some(start.to_string());
    core::iter::from_fn(move || {
        let dir = current.take()?;
        current = parent_dir(&dir);
        Some(dir)
    })
}

/* Directory of `spec`, up to and including the last '/'. "lib/foo.py" -> "lib/", "foo.py" -> "". */
pub fn dir_of(spec: &str) -> &str {
    match spec.rfind('/') {
        Some(i) => &spec[..=i],
        None => "",
    }
}

/* Resolve `target` against `dir`. Absolute forms pass through; `../` pops parents; `./` strips only when base is non-empty. */
pub fn join_relative(dir: &str, target: &str) -> String {
    if target.contains("://") || target.starts_with('/') || target.starts_with("mt:") {
        return target.to_string();
    }
    let mut base = dir.to_string();
    let mut t = target;
    while let Some(rest) = t.strip_prefix("../") {
        base = parent_dir(&base).unwrap_or_default();
        t = rest;
    }
    if t == ".." { return parent_dir(&base).unwrap_or_default(); }
    if t == "." || t.is_empty() { return base; }
    if !base.is_empty() {
        while let Some(rest) = t.strip_prefix("./") { t = rest; }
        if !base.ends_with('/') { base.push('/'); }
    }
    base.push_str(t);
    base
}

fn parent_dir(dir: &str) -> Option<String> {
    if dir.is_empty() { return None; }
    let trimmed = dir.trim_end_matches('/');
    // URL guard: never strip the host. After "scheme://" there must still be a '/' to walk into.
    if let Some(scheme_end) = trimmed.find("://") {
        let after = &trimmed[scheme_end + 3..];
        if !after.contains('/') { return None; }
    }
    match trimmed.rsplit_once('/') {
        Some(("", _)) => Some(String::new()),
        Some((head, _)) => Some(s!(str head, "/")),
        None => Some(String::new()),
    }
}

struct Reader<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> { self.src.get(self.pos).copied() }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') { self.pos += 1; } else { break; }
        }
    }

    fn expect(&mut self, c: u8, msg: &'static str) -> Result<(), String> {
        if self.peek() == Some(c) { self.pos += 1; Ok(()) } else { Err(s!(str msg)) }
    }

    fn read_string(&mut self) -> Result<String, String> {
        self.expect(b'"', "expected '\"' starting a string")?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(s!("unterminated string in packages.json")),
                Some(b'"') => { self.pos += 1; return Ok(out); }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| s!("dangling '\\' in packages.json"))?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        _ => return Err(s!("unsupported escape '\\", char esc as char, "' in packages.json")),
                    }
                    self.pos += 1;
                }
                Some(c) => { out.push(c as char); self.pos += 1; }
            }
        }
    }

    fn read_imports_into(&mut self, out: &mut Vec<(String, String)>) -> Result<(), String> {
        self.expect(b'{', "'imports' must be an object")?;
        self.skip_ws();
        if self.peek() == Some(b'}') { self.pos += 1; return Ok(()); }
        loop {
            self.skip_ws();
            let k = self.read_string()?;
            self.skip_ws();
            self.expect(b':', "expected ':' after import name")?;
            self.skip_ws();
            let v = self.read_string()?;
            out.push((k, v));
            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; continue; }
                Some(b'}') => { self.pos += 1; return Ok(()); }
                _ => return Err(s!("expected ',' or '}' in 'imports'")),
            }
        }
    }

    /* Skip a string or string-keyed object, forgives future keys. Numeric, array, bool values surface as errors so typos don't pass silently. */
    fn skip_value(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(b'"') => { let _ = self.read_string()?; Ok(()) }
            Some(b'{') => {
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') { self.pos += 1; return Ok(()); }
                loop {
                    self.skip_ws();
                    let _ = self.read_string()?;
                    self.skip_ws();
                    self.expect(b':', "expected ':' in nested object")?;
                    self.skip_ws();
                    self.skip_value()?;
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => { self.pos += 1; continue; }
                        Some(b'}') => { self.pos += 1; return Ok(()); }
                        _ => return Err(s!("expected ',' or '}' in nested object")),
                    }
                }
            }
            _ => Err(s!("unsupported value in packages.json (only strings / string-objects)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_relative_routes() {
        assert_eq!(join_relative("lib/test/", "./helper.py"), "lib/test/helper.py");
        assert_eq!(join_relative("lib/test/", "../parse.py"), "lib/parse.py");
        assert_eq!(join_relative("lib/test/", "../../main.py"), "main.py");
        assert_eq!(join_relative("lib/", "../../escape.py"), "escape.py"); // clamped at root
        assert_eq!(join_relative("lib/test/", "/std/x.py"), "/std/x.py");
        assert_eq!(join_relative("", "https://x/y.py"), "https://x/y.py");
    }

    #[test]
    fn dir_walk_routes() {
        assert_eq!(dir_of("lib/test/a.py"), "lib/test/");
        assert_eq!(dir_of("a.py"), "");
        assert_eq!(parent_dir("lib/test/"), Some("lib/".into()));
        assert_eq!(parent_dir("lib/"), Some("".into()));
        assert_eq!(parent_dir(""), None);
    }
}
