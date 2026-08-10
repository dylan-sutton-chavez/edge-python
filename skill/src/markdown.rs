pub struct Block {
    pub lang: String,
    pub meta: String,
    pub body: String,
    pub line: usize,
}

pub fn scan(src: &str) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut lines = src.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        let head = line.trim_end();
        if !head.starts_with("```") || head.starts_with("````") {
            continue;
        }
        let head = head[3..].trim();
        let mut parts = head.splitn(2, char::is_whitespace);
        let lang = parts.next().unwrap_or("").to_string();
        let meta = parts.next().unwrap_or("").trim().to_string();
        let mut body = String::new();
        let mut closed = false;
        for (_, l) in lines.by_ref() {
            if l.trim_end() == "```" {
                closed = true;
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        if !closed {
            return Err(format!("line {}: unclosed fence", i + 1));
        }
        blocks.push(Block { lang, meta, body, line: i + 1 });
    }
    Ok(blocks)
}
