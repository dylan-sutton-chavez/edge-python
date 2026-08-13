use std::path::{Component, Path, PathBuf};

// Leading bytes marking an edge package, checked before anything is trusted.
pub const MAGIC: &[u8] = b"EDGEPKG\x01";

// Caps a hostile bundle, plenty for a real project tree.
const MAX_FILES: usize = 4096;
const MAX_TOTAL: u64 = 64 << 20;

// One file inside the archive, path relative to the project root.
pub struct Entry {
    pub path: String,
    pub bytes: Vec<u8>,
}

// A resolved project serialized as a flat length-prefixed archive, the shape edge build and the swarm ingress share, entry is the relative path of main.py. No zip so no zip-slip or zip-bomb, every path is validated relative on read.
pub struct Bundle {
    pub entry: String,
    pub files: Vec<Entry>,
}

impl Bundle {
    /* Encodes magic, entry, then each file as `<len>\n<path>\n<bytes>`, all lengths ascii. */
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        put_str(&mut b, &self.entry);
        put_usz(&mut b, self.files.len());
        for f in &self.files {
            put_str(&mut b, &f.path);
            put_bytes(&mut b, &f.bytes);
        }
        b
    }

    /* Decodes and validates a bundle, every path must stay inside the tree. */
    pub fn decode(buf: &[u8]) -> Result<Bundle, String> {
        let mut r = Reader { buf, p: 0 };
        if !r.take(MAGIC.len())?.starts_with(MAGIC) {
            return Err("not an edge package".to_string());
        }
        let entry = r.str()?;
        safe_rel(&entry)?;
        let n = r.usz()?;
        if n > MAX_FILES {
            return Err(format!("bundle has {n} files, over the {MAX_FILES} cap"));
        }
        let mut files = Vec::with_capacity(n);
        let mut total = 0u64;
        for _ in 0..n {
            let path = r.str()?;
            safe_rel(&path)?;
            let bytes = r.bytes()?;
            total = total.saturating_add(bytes.len() as u64);
            if total > MAX_TOTAL {
                return Err(format!("bundle exceeds the {MAX_TOTAL} byte cap"));
            }
            files.push(Entry { path, bytes });
        }
        Ok(Bundle { entry, files })
    }

    /* Materializes the tree under `root`, creating parent dirs, returns the entry path on disk. */
    pub fn write_to(&self, root: &Path) -> std::io::Result<PathBuf> {
        for f in &self.files {
            let dest = root.join(&f.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &f.bytes)?;
        }
        Ok(root.join(&self.entry))
    }
}

/* Rejects any path that is absolute or climbs out with `..`, the whole anti zip-slip guard. */
fn safe_rel(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("bundle has an empty path".to_string());
    }
    for c in Path::new(path).components() {
        match c {
            Component::Normal(_) => {}
            _ => return Err(format!("bundle path '{path}' is not a plain relative path")),
        }
    }
    Ok(())
}

fn put_usz(b: &mut Vec<u8>, n: usize) {
    b.extend_from_slice(n.to_string().as_bytes());
    b.push(b'\n');
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    put_bytes(b, s.as_bytes());
}

fn put_bytes(b: &mut Vec<u8>, bytes: &[u8]) {
    put_usz(b, bytes.len());
    b.extend_from_slice(bytes);
}

struct Reader<'a> {
    buf: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.p.checked_add(n).ok_or("bundle length overflow")?;
        if end > self.buf.len() {
            return Err("bundle truncated".to_string());
        }
        let slice = &self.buf[self.p..end];
        self.p = end;
        Ok(slice)
    }

    // Reads an ascii length terminated by a newline.
    fn usz(&mut self) -> Result<usize, String> {
        let start = self.p;
        while self.p < self.buf.len() && self.buf[self.p] != b'\n' {
            self.p += 1;
        }
        if self.p >= self.buf.len() {
            return Err("bundle truncated reading a length".to_string());
        }
        let text = core::str::from_utf8(&self.buf[start..self.p]).map_err(|_| "bundle length not ascii")?;
        self.p += 1;
        text.parse().map_err(|_| format!("bundle length '{text}' is not a number"))
    }

    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.usz()?;
        Ok(self.take(n)?.to_vec())
    }

    fn str(&mut self) -> Result<String, String> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes).map_err(|_| "bundle string not utf-8".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Bundle {
        Bundle {
            entry: "main.py".to_string(),
            files: vec![
                Entry { path: "main.py".to_string(), bytes: b"import lib\n".to_vec() },
                Entry { path: "lib/util.py".to_string(), bytes: b"def f(): pass\n".to_vec() },
            ],
        }
    }

    #[test]
    fn roundtrips_a_tree() {
        let b = Bundle::decode(&sample().encode()).unwrap();
        assert_eq!(b.entry, "main.py");
        assert_eq!(b.files.len(), 2);
        assert_eq!(b.files[1].path, "lib/util.py");
        assert_eq!(b.files[1].bytes, b"def f(): pass\n");
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(Bundle::decode(b"NOTPKG\x00\x00junk").is_err());
    }

    #[test]
    fn rejects_traversal_paths() {
        for bad in ["../evil.py", "/etc/passwd", "a/../../b"] {
            let mut b = sample();
            b.files[0].path = bad.to_string();
            assert!(Bundle::decode(&b.encode()).is_err(), "path '{bad}' should be rejected");
        }
    }
}
