use alloc::{format, string::String, vec::Vec};
use crate::ast::{Node, Program};
use crate::matcher::{fixed_len, Caps, Matcher};

/* Which anchoring a single match uses. */
pub enum Mode {
    Search,
    Match,
    Full,
}

/* Engine error, the variant decides the host exception kind. */
#[derive(Debug)]
pub enum ReError {
    Syntax(String), // malformed pattern, surfaces as ValueError
    TooComplex(String), // backtracking blew its budget, surfaces as RuntimeError
}

/* A resolved match, offsets are codepoint indices. */
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub groups: Vec<Option<String>>,
}

pub struct Regex {
    prog: Program,
}

impl Regex {
    pub fn compile(pattern: &str) -> Result<Regex, ReError> {
        let prog = crate::parser::parse(pattern).map_err(|e| ReError::Syntax(format!("{} at position {}", e.msg, e.pos)))?;
        validate(&prog.root)?;
        Ok(Regex { prog })
    }

    pub fn group_count(&self) -> usize { self.prog.group_count }
}

/* Single match in the requested mode, over an already-compiled regex. */
pub fn find_rx(re: &Regex, text: &str, mode: Mode) -> Result<Option<Found>, ReError> {
    let chars: Vec<char> = text.chars().collect();
    let m = Matcher::new(&chars, re.prog.flags);
    let caps = match mode {
        Mode::Search => m.search(&re.prog.root, re.prog.group_count),
        Mode::Match => m.match_at(&re.prog.root, re.prog.group_count, false),
        Mode::Full => m.match_at(&re.prog.root, re.prog.group_count, true),
    };
    match caps {
        Some(c) => Ok(Some(build(&chars, &c, re.prog.group_count))),
        None if m.exceeded() => Err(too_complex()),
        None => Ok(None),
    }
}

/* All non overlapping matches over a compiled regex, plus the group count for output shaping. */
pub fn find_all_rx(re: &Regex, text: &str) -> Result<(Vec<Found>, usize), ReError> {
    let chars: Vec<char> = text.chars().collect();
    let m = Matcher::new(&chars, re.prog.flags);
    let mut out = Vec::new();
    let mut start = 0;
    while start <= chars.len() {
        match m.search_from(&re.prog.root, re.prog.group_count, start) {
            Some(c) => {
                let (s, e) = c[0].unwrap();
                out.push(build(&chars, &c, re.prog.group_count));
                start = if e > s { e } else { e + 1 }; // step past an empty match
            }
            None => {
                if m.exceeded() { return Err(too_complex()); }
                break;
            }
        }
    }
    Ok((out, re.prog.group_count))
}

/* Replace every match over a compiled regex, expanding backreferences in the template. */
pub fn sub_rx(re: &Regex, repl: &str, text: &str) -> Result<String, ReError> {
    let chars: Vec<char> = text.chars().collect();
    let repl_chars: Vec<char> = repl.chars().collect();
    let m = Matcher::new(&chars, re.prog.flags);
    let mut out = String::new();
    let mut last = 0;
    let mut start = 0;
    while start <= chars.len() {
        let Some(c) = m.search_from(&re.prog.root, re.prog.group_count, start) else {
            if m.exceeded() { return Err(too_complex()); }
            break;
        };
        let (s, e) = c[0].unwrap();
        for ch in &chars[last..s] { out.push(*ch); }
        expand(&repl_chars, &chars, &c, &re.prog.names, &mut out)?;
        if e > s {
            last = e;
            start = e;
        } else {
            if e < chars.len() { out.push(chars[e]); }
            last = e + 1;
            start = e + 1;
        }
    }
    for ch in &chars[last.min(chars.len())..] { out.push(*ch); }
    Ok(out)
}

fn build(chars: &[char], caps: &Caps, ngroups: usize) -> Found {
    let (s, e) = caps[0].unwrap();
    let text: String = chars[s..e].iter().collect();
    let mut groups = Vec::with_capacity(ngroups);
    for i in 1..=ngroups {
        groups.push(caps.get(i).copied().flatten().map(|(a, b)| chars[a..b].iter().collect()));
    }
    Found { start: s, end: e, text, groups }
}

/* Expand a replacement template against the captured groups. */
fn expand(repl: &[char], chars: &[char], caps: &Caps, names: &[(String, usize)], out: &mut String) -> Result<(), ReError> {
    let mut i = 0;
    while i < repl.len() {
        let ch = repl[i];
        if ch != '\\' { out.push(ch); i += 1; continue; }
        i += 1;
        let Some(&n) = repl.get(i) else { return Err(ReError::Syntax(String::from("bad replacement, trailing backslash"))); };
        match n {
            '\\' => { out.push('\\'); i += 1; }
            'n' => { out.push('\n'); i += 1; }
            't' => { out.push('\t'); i += 1; }
            'r' => { out.push('\r'); i += 1; }
            '0'..='9' => {
                let mut num = 0usize;
                while i < repl.len() && repl[i].is_ascii_digit() {
                    num = num * 10 + repl[i].to_digit(10).unwrap() as usize;
                    i += 1;
                }
                push_group(num, chars, caps, out);
            }
            'g' => {
                i += 1;
                if repl.get(i) != Some(&'<') { return Err(ReError::Syntax(String::from("missing < in group reference"))); }
                i += 1;
                let mut name = String::new();
                while i < repl.len() && repl[i] != '>' { name.push(repl[i]); i += 1; }
                if repl.get(i) != Some(&'>') { return Err(ReError::Syntax(String::from("missing > in group reference"))); }
                i += 1;
                let idx = resolve_name(&name, names)?;
                push_group(idx, chars, caps, out);
            }
            other => { out.push('\\'); out.push(other); i += 1; }
        }
    }
    Ok(())
}

fn resolve_name(name: &str, names: &[(String, usize)]) -> Result<usize, ReError> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        return name.parse::<usize>().map_err(|_| ReError::Syntax(String::from("bad group number")));
    }
    names.iter().find(|(nm, _)| nm == name).map(|(_, ix)| *ix).ok_or(ReError::Syntax(String::from("unknown group name")))
}

fn push_group(idx: usize, chars: &[char], caps: &Caps, out: &mut String) {
    if let Some((s, e)) = caps.get(idx).copied().flatten() {
        for ch in &chars[s..e] { out.push(*ch); }
    }
}

/* Signals that backtracking degraded, so the author can rewrite the pattern. */
fn too_complex() -> ReError {
    ReError::TooComplex(String::from("catastrophic backtracking: O(n^2) time or worse on this input, simplify nested quantifiers"))
}

/* Reject lookbehind whose width is not fixed, which the engine cannot run. */
fn validate(node: &Node) -> Result<(), ReError> {
    match node {
        Node::Look { node, behind: true, .. } => {
            if fixed_len(node).is_none() { return Err(ReError::Syntax(String::from("lookbehind requires fixed width"))); }
            validate(node)
        }
        Node::Look { node, .. } => validate(node),
        Node::Concat(v) | Node::Alt(v) => { for n in v { validate(n)?; } Ok(()) }
        Node::Group { node, .. } | Node::NonCap(node) | Node::Repeat { node, .. } => validate(node),
        _ => Ok(()),
    }
}
