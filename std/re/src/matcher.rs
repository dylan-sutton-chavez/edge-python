/* Backtracking matcher over codepoints, so offsets are Unicode aware. */

use alloc::vec::Vec;
use core::cell::Cell;
use super::ast::*;

/* Capture slots, index 0 is the whole match. */
pub type Caps = Vec<Option<(usize, usize)>>;

/* Bundled repetition spec, keeps the recursive helpers small. */
struct Rep<'a> {
    node: &'a Node,
    min: usize,
    max: Option<usize>,
    greedy: bool,
}

pub struct Matcher<'a> {
    input: &'a [char],
    flags: Flags,
    steps: Cell<u64>, // backtracking work counter for the current attempt
    budget: u64, // abort once the counter passes this
}

impl<'a> Matcher<'a> {
    pub fn new(input: &'a [char], flags: Flags) -> Self {
        // Linear allowance, legitimate matches stay under it, blowups race past it.
        let budget = 100_000 + 2_000 * input.len() as u64;
        Self { input, flags, steps: Cell::new(0), budget }
    }

    /* True when the last attempt exhausted its step budget. */
    pub fn exceeded(&self) -> bool { self.steps.get() > self.budget }

    /* Leftmost search from the beginning. */
    pub fn search(&self, root: &Node, ngroups: usize) -> Option<Caps> {
        self.search_from(root, ngroups, 0)
    }

    /* Leftmost search starting no earlier than `from`. */
    pub fn search_from(&self, root: &Node, ngroups: usize, from: usize) -> Option<Caps> {
        self.steps.set(0);
        for start in from..=self.input.len() {
            if self.exceeded() { break; } // stop scanning once the budget is gone
            let mut caps: Caps = empty_caps(ngroups);
            let mut found: Option<usize> = None;
            self.m(root, start, &mut caps, &mut |end, _| { found = Some(end); true });
            if let Some(end) = found {
                caps[0] = Some((start, end));
                return Some(caps);
            }
        }
        None
    }

    /* Anchored match at position zero, optionally requiring full consumption. */
    pub fn match_at(&self, root: &Node, ngroups: usize, full: bool) -> Option<Caps> {
        self.steps.set(0);
        let mut caps = empty_caps(ngroups);
        let mut found: Option<usize> = None;
        let len = self.input.len();
        self.m(root, 0, &mut caps, &mut |end, _| {
            if full && end != len { return false; }
            found = Some(end);
            true
        });
        found.map(|end| { caps[0] = Some((0, end)); caps })
    }

    /* Core dispatch, k is the continuation called with the position after node. */
    fn m(&self, node: &Node, pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n > self.budget { return false; } // budget gone, unwind every branch
        match node {
            Node::Empty => k(pos, caps),
            Node::Char(_) | Node::AnyChar | Node::Class { .. } => {
                pos < self.input.len() && self.single_match(node, pos) && k(pos + 1, caps)
            }
            Node::Start => self.at_start(pos) && k(pos, caps),
            Node::End => self.at_end(pos) && k(pos, caps),
            Node::WordBoundary => self.boundary(pos) && k(pos, caps),
            Node::NotWordBoundary => !self.boundary(pos) && k(pos, caps),
            Node::Concat(v) => self.m_seq(v, pos, caps, k),
            Node::Alt(v) => {
                for branch in v {
                    if self.m(branch, pos, caps, k) { return true; }
                }
                false
            }
            Node::NonCap(inner) => self.m(inner, pos, caps, k),
            Node::Group { index, node: inner, .. } => {
                let index = *index;
                let start = pos;
                self.m(inner, pos, caps, &mut |end, caps| {
                    let prev = caps[index];
                    caps[index] = Some((start, end));
                    if k(end, caps) { true } else { caps[index] = prev; false }
                })
            }
            Node::Repeat { node: inner, min, max, greedy } => {
                let rep = Rep { node: inner, min: *min, max: *max, greedy: *greedy };
                self.repeat(&rep, pos, 0, caps, k)
            }
            Node::Backref(n) => self.backref(*n, pos, caps, k),
            Node::Look { node: inner, behind, negative } => {
                self.look(inner, *behind, *negative, pos, caps, k)
            }
        }
    }

    /* Sequence walker, threads the continuation across nodes. */
    fn m_seq(&self, nodes: &[Node], pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        match nodes.split_first() {
            None => k(pos, caps),
            Some((first, rest)) => {
                self.m(first, pos, caps, &mut |p, c| self.m_seq(rest, p, c, k))
            }
        }
    }

    /* Repetition. Single codepoint atoms run iteratively to bound recursion. */
    fn repeat(&self, rep: &Rep, pos: usize, count: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        if is_single(rep.node) {
            return self.repeat_single(rep, pos, caps, k);
        }
        let can_more = rep.max.is_none_or(|m| count < m);
        if rep.greedy {
            if can_more {
                let stepped = self.m(rep.node, pos, caps, &mut |p, c| {
                    if p == pos { return false; } // stop zero width expansion
                    self.repeat(rep, p, count + 1, c, k)
                });
                if stepped { return true; }
            }
            count >= rep.min && k(pos, caps)
        } else {
            if count >= rep.min && k(pos, caps) { return true; }
            if can_more {
                return self.m(rep.node, pos, caps, &mut |p, c| {
                    if p == pos { return false; }
                    self.repeat(rep, p, count + 1, c, k)
                });
            }
            false
        }
    }

    /* Iterative repeat for atoms that consume exactly one codepoint. */
    fn repeat_single(&self, rep: &Rep, pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        let mut n = 0;
        let mut p = pos;
        while rep.max.is_none_or(|m| n < m) && p < self.input.len() && self.single_match(rep.node, p) {
            p += 1;
            n += 1;
        }
        if n < rep.min { return false; }
        if rep.greedy {
            let mut i = n;
            loop {
                if k(pos + i, caps) { return true; }
                if i == rep.min { return false; }
                i -= 1;
            }
        } else {
            let mut i = rep.min;
            loop {
                if k(pos + i, caps) { return true; }
                if i == n { return false; }
                i += 1;
            }
        }
    }

    fn backref(&self, n: usize, pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        match caps.get(n).copied().flatten() {
            None => k(pos, caps), // unmatched group behaves like empty
            Some((s, e)) => {
                let len = e - s;
                if pos + len > self.input.len() { return false; }
                for i in 0..len {
                    if !self.ci_eq(self.input[pos + i], self.input[s + i]) { return false; }
                }
                k(pos + len, caps)
            }
        }
    }

    fn look(&self, inner: &Node, behind: bool, negative: bool, pos: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        let matched = if behind {
            match fixed_len(inner) {
                Some(w) if pos >= w => {
                    let mut hit = false;
                    self.m(inner, pos - w, caps, &mut |end, _| {
                        if end == pos { hit = true; true } else { false }
                    });
                    hit
                }
                _ => false,
            }
        } else {
            let mut hit = false;
            self.m(inner, pos, caps, &mut |_, _| { hit = true; true });
            hit
        };
        if matched != negative { k(pos, caps) } else { false }
    }

    /* True when node consumes input[pos] as a single codepoint. */
    fn single_match(&self, node: &Node, pos: usize) -> bool {
        let c = self.input[pos];
        match node {
            Node::Char(want) => self.ci_eq(c, *want),
            Node::AnyChar => self.flags.dotall || c != '\n',
            Node::Class { items, negated } => {
                let hit = self.class_hit(items, c);
                hit != *negated
            }
            _ => false,
        }
    }

    /* Class membership, widening by case when ignorecase is set. */
    fn class_hit(&self, items: &[ClassItem], c: char) -> bool {
        if class_contains(items, c) { return true; }
        if self.flags.ignorecase {
            for alt in fold_variants(c) {
                if alt != c && class_contains(items, alt) { return true; }
            }
        }
        false
    }

    fn ci_eq(&self, a: char, b: char) -> bool {
        if a == b { return true; }
        if self.flags.ignorecase { a.to_lowercase().eq(b.to_lowercase()) } else { false }
    }

    fn at_start(&self, pos: usize) -> bool {
        pos == 0 || (self.flags.multiline && pos > 0 && self.input[pos - 1] == '\n')
    }

    fn at_end(&self, pos: usize) -> bool {
        let len = self.input.len();
        if pos == len { return true; }
        if pos == len - 1 && self.input[pos] == '\n' { return true; } // before a trailing newline
        self.flags.multiline && self.input[pos] == '\n'
    }

    fn boundary(&self, pos: usize) -> bool {
        let before = pos > 0 && is_word(self.input[pos - 1]);
        let after = pos < self.input.len() && is_word(self.input[pos]);
        before != after
    }
}

fn empty_caps(ngroups: usize) -> Caps {
    let mut v = Vec::with_capacity(ngroups + 1);
    for _ in 0..=ngroups { v.push(None); }
    v
}

fn is_single(node: &Node) -> bool {
    matches!(node, Node::Char(_) | Node::AnyChar | Node::Class { .. })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/* Predefined class predicates lean on std, so Unicode needs no tables. */
fn item_match(item: &ClassItem, c: char) -> bool {
    match item {
        ClassItem::Ch(x) => c == *x,
        ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
        ClassItem::Digit => c.is_numeric(),
        ClassItem::NotDigit => !c.is_numeric(),
        ClassItem::Word => is_word(c),
        ClassItem::NotWord => !is_word(c),
        ClassItem::Space => c.is_whitespace(),
        ClassItem::NotSpace => !c.is_whitespace(),
    }
}

fn class_contains(items: &[ClassItem], c: char) -> bool {
    items.iter().any(|it| item_match(it, c))
}

/* Case variants to test for ignorecase class membership. */
fn fold_variants(c: char) -> [char; 2] {
    let lo = c.to_lowercase().next().unwrap_or(c);
    let up = c.to_uppercase().next().unwrap_or(c);
    [lo, up]
}

/* Fixed codepoint width of a node, None when it varies. */
pub fn fixed_len(node: &Node) -> Option<usize> {
    match node {
        Node::Empty | Node::Start | Node::End | Node::WordBoundary | Node::NotWordBoundary => Some(0),
        Node::Look { .. } => Some(0),
        Node::Char(_) | Node::AnyChar | Node::Class { .. } => Some(1),
        Node::Concat(v) => {
            let mut total = 0;
            for n in v { total += fixed_len(n)?; }
            Some(total)
        }
        Node::Alt(v) => {
            let mut it = v.iter();
            let first = fixed_len(it.next()?)?;
            for n in it { if fixed_len(n)? != first { return None; } }
            Some(first)
        }
        Node::Group { node, .. } | Node::NonCap(node) => fixed_len(node),
        Node::Repeat { node, min, max, .. } => {
            let m = (*max)?;
            if m != *min { return None; }
            Some(fixed_len(node)? * m)
        }
        Node::Backref(_) => None,
    }
}
