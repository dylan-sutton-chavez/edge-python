use crate::markdown::Block;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Both,
    Native,
    Web,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Output,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Python,
    PythonSkip,
    Swarm,
    Untrusted,
}

pub struct Cell {
    pub kind: Kind,
    pub engine: Engine,
    pub verdict: Verdict,
    pub body: String,
    pub expect: String,
    pub line: usize,
}

fn parse_text_meta(meta: &str, line: usize) -> Result<(Engine, Verdict), String> {
    let mut engine = Engine::Both;
    let mut verdict = Verdict::Output;
    for tok in meta.split_whitespace() {
        match tok {
            "Output" => engine = Engine::Both,
            "Native" => engine = Engine::Native,
            "Web" => engine = Engine::Web,
            "Error" => verdict = Verdict::Error,
            other => return Err(format!("line {line}: unknown text meta '{other}'")),
        }
    }
    Ok((engine, verdict))
}

pub fn collect(blocks: &[Block]) -> Result<Vec<Cell>, String> {
    let mut cells = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let b = &blocks[i];
        let kind = match (b.lang.as_str(), b.meta.as_str()) {
            ("python", "") => Kind::Python,
            ("python", "skip") => Kind::PythonSkip,
            ("yml", "swarm") => Kind::Swarm,
            ("yml", "untrusted") => Kind::Untrusted,
            ("python" | "yml", meta) => {
                return Err(format!("line {}: unknown {} meta '{meta}'", b.line, b.lang));
            }
            _ => {
                i += 1;
                continue;
            }
        };

        if kind == Kind::PythonSkip {
            if !b.body.contains("# skip") {
                return Err(format!(
                    "line {}: skip cell needs a '# skip' comment at the nondeterministic construct",
                    b.line
                ));
            }
            if blocks.get(i + 1).is_some_and(|n| n.lang == "text") {
                return Err(format!(
                    "line {}: skip cell never pairs with a text block",
                    b.line
                ));
            }
            i += 1;
            continue;
        }

        let next = blocks.get(i + 1);
        let Some(text) = next.filter(|n| n.lang == "text") else {
            if kind == Kind::Python {
                // A bare python block is illustrative, only a text pair makes it runnable.
                i += 1;
                continue;
            }
            return Err(format!(
                "line {}: runnable cell is not followed by a text block",
                b.line
            ));
        };
        let (engine, verdict) = parse_text_meta(&text.meta, text.line)?;

        if matches!(kind, Kind::Swarm | Kind::Untrusted) && engine == Engine::Web {
            return Err(format!(
                "line {}: swarm cells are native only and cannot pair with Web",
                text.line
            ));
        }
        if kind == Kind::Swarm && b.body.contains("eval: true") {
            return Err(format!(
                "line {}: yml swarm group uses eval, label it yml untrusted",
                b.line
            ));
        }
        if kind == Kind::Untrusted && !b.body.contains("eval: true") {
            return Err(format!(
                "line {}: yml untrusted needs every group with eval: true",
                b.line
            ));
        }

        cells.push(Cell {
            kind,
            engine,
            verdict,
            body: b.body.clone(),
            expect: text.body.clone(),
            line: b.line,
        });
        i += 2;
    }
    Ok(cells)
}
