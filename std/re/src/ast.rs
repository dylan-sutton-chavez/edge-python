/* Regex AST and flags, matched directly by the backtracking engine. */

use alloc::{boxed::Box, string::String, vec::Vec};

/* Inline flag state threaded into the matcher. */
#[derive(Clone, Copy, Default)]
pub struct Flags {
    pub ignorecase: bool, // (?i)
    pub dotall: bool, // (?s), dot also matches newline
    pub multiline: bool, // (?m), anchors match at line boundaries
}

/* One entry inside a bracket set. Predefined classes stay symbolic. */
#[derive(Clone)]
pub enum ClassItem {
    Ch(char),
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

#[derive(Clone)]
pub enum Node {
    Empty,
    Char(char),
    AnyChar, // the dot
    Class { items: Vec<ClassItem>, negated: bool },
    Start, // caret anchor
    End, // dollar anchor
    WordBoundary, // backslash b
    NotWordBoundary, // backslash B
    Group { index: usize, name: Option<String>, node: Box<Node> },
    NonCap(Box<Node>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: usize, max: Option<usize>, greedy: bool },
    Backref(usize),
    Look { node: Box<Node>, behind: bool, negative: bool },
}

/* Compiled pattern, the tree plus capture metadata and flags. */
pub struct Program {
    pub root: Node,
    pub group_count: usize,
    pub names: Vec<(String, usize)>, // maps a group name to its index
    pub flags: Flags,
}

#[derive(Debug)]
pub struct ParseError {
    pub msg: &'static str,
    pub pos: usize, // codepoint offset into the pattern
}
