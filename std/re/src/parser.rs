/* Recursive descent parser, pattern text to AST. */

use alloc::{boxed::Box, string::String, vec::Vec};
use super::ast::*;

/* Parse a pattern into a Program or fail with a positioned error. */
pub fn parse(pattern: &str) -> Result<Program, ParseError> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = Parser {
        p: &chars,
        pos: 0,
        group_count: 0,
        names: Vec::new(),
        flags: Flags::default(),
    };
    let root = p.alternation()?;
    if p.pos != p.p.len() {
        return Err(p.err("unbalanced parenthesis"));
    }
    Ok(Program { root, group_count: p.group_count, names: p.names, flags: p.flags })
}

struct Parser<'a> {
    p: &'a [char],
    pos: usize,
    group_count: usize,
    names: Vec<(String, usize)>,
    flags: Flags,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> { self.p.get(self.pos).copied() }
    fn at(&self, i: usize) -> Option<char> { self.p.get(self.pos + i).copied() }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() { self.pos += 1; }
        c
    }
    fn err(&self, msg: &'static str) -> ParseError { ParseError { msg, pos: self.pos } }

    fn expect(&mut self, c: char) -> Result<(), ParseError> {
        if self.peek() == Some(c) { self.bump(); Ok(()) }
        else { Err(self.err("missing closing parenthesis")) }
    }

    /* Lowest precedence, branches split on the pipe. */
    fn alternation(&mut self) -> Result<Node, ParseError> {
        let mut branches = Vec::new();
        branches.push(self.concat()?);
        while self.peek() == Some('|') {
            self.bump();
            branches.push(self.concat()?);
        }
        if branches.len() == 1 { Ok(branches.pop().unwrap()) }
        else { Ok(Node::Alt(branches)) }
    }

    /* A run of quantified atoms until pipe, close paren, or end. */
    fn concat(&mut self) -> Result<Node, ParseError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => items.push(self.quantified()?),
            }
        }
        match items.len() {
            0 => Ok(Node::Empty),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(Node::Concat(items)),
        }
    }

    /* An atom plus optional repetition operator. */
    fn quantified(&mut self) -> Result<Node, ParseError> {
        let atom = self.atom()?;
        let (min, max) = match self.peek() {
            Some('*') => { self.bump(); (0, None) }
            Some('+') => { self.bump(); (1, None) }
            Some('?') => { self.bump(); (0, Some(1)) }
            Some('{') => match self.try_bound()? {
                Some(b) => b,
                None => return Ok(atom), // a bare brace is a literal
            },
            _ => return Ok(atom),
        };
        let greedy = if self.peek() == Some('?') { self.bump(); false } else { true };
        if matches!(self.peek(), Some('*') | Some('+')) {
            return Err(self.err("multiple repeat"));
        }
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    /* Parse a counted bound, restoring position when it is not one. */
    fn try_bound(&mut self) -> Result<Option<(usize, Option<usize>)>, ParseError> {
        let save = self.pos;
        self.bump(); // opening brace
        let lo = self.read_int();
        match self.peek() {
            Some('}') => match lo {
                Some(n) => { self.bump(); Ok(Some((n, Some(n)))) }
                None => { self.pos = save; Ok(None) }
            },
            Some(',') => {
                self.bump();
                let hi = self.read_int();
                if self.peek() == Some('}') {
                    self.bump();
                    Ok(Some((lo.unwrap_or(0), hi)))
                } else {
                    self.pos = save;
                    Ok(None)
                }
            }
            _ => { self.pos = save; Ok(None) }
        }
    }

    fn read_int(&mut self) -> Option<usize> {
        let start = self.pos;
        let mut n: usize = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d as usize);
            self.bump();
        }
        if self.pos == start { None } else { Some(n) }
    }

    fn atom(&mut self) -> Result<Node, ParseError> {
        match self.peek() {
            Some('(') => self.group(),
            Some('[') => self.class(),
            Some('.') => { self.bump(); Ok(Node::AnyChar) }
            Some('^') => { self.bump(); Ok(Node::Start) }
            Some('$') => { self.bump(); Ok(Node::End) }
            Some('\\') => self.escape(),
            Some('*') | Some('+') | Some('?') => Err(self.err("nothing to repeat")),
            Some(c) => { self.bump(); Ok(Node::Char(c)) }
            None => Ok(Node::Empty),
        }
    }

    fn group(&mut self) -> Result<Node, ParseError> {
        self.bump(); // open paren
        if self.peek() != Some('?') {
            self.group_count += 1;
            let index = self.group_count;
            let node = self.alternation()?;
            self.expect(')')?;
            return Ok(Node::Group { index, name: None, node: Box::new(node) });
        }
        self.bump(); // question mark
        match self.peek() {
            Some(':') => { self.bump(); let n = self.alternation()?; self.expect(')')?; Ok(Node::NonCap(Box::new(n))) }
            Some('=') => { self.bump(); self.look(false, false) }
            Some('!') => { self.bump(); self.look(false, true) }
            Some('<') => {
                self.bump();
                match self.peek() {
                    Some('=') => { self.bump(); self.look(true, false) }
                    Some('!') => { self.bump(); self.look(true, true) }
                    _ => { let name = self.read_name('>')?; self.named_group(name) }
                }
            }
            Some('P') => {
                self.bump();
                match self.peek() {
                    Some('<') => { self.bump(); let name = self.read_name('>')?; self.named_group(name) }
                    Some('=') => { self.bump(); let name = self.read_name(')')?; self.named_backref(&name) }
                    _ => Err(self.err("unknown extension")),
                }
            }
            Some('#') => {
                while let Some(c) = self.peek() { if c == ')' { break; } self.bump(); }
                self.expect(')')?;
                Ok(Node::Empty)
            }
            Some(c) if is_flag_char(c) => { self.read_flags()?; self.expect(')')?; Ok(Node::Empty) }
            _ => Err(self.err("unknown extension")),
        }
    }

    fn look(&mut self, behind: bool, negative: bool) -> Result<Node, ParseError> {
        let node = self.alternation()?;
        self.expect(')')?;
        Ok(Node::Look { node: Box::new(node), behind, negative })
    }

    /* Assign the index before the body so order matches paren order. */
    fn named_group(&mut self, name: String) -> Result<Node, ParseError> {
        self.group_count += 1;
        let index = self.group_count;
        self.names.push((name.clone(), index));
        let node = self.alternation()?;
        self.expect(')')?;
        Ok(Node::Group { index, name: Some(name), node: Box::new(node) })
    }

    fn named_backref(&mut self, name: &str) -> Result<Node, ParseError> {
        let idx = self.names.iter().find(|(n, _)| n == name).map(|(_, i)| *i);
        match idx {
            Some(i) => Ok(Node::Backref(i)),
            None => Err(self.err("unknown group name")),
        }
    }

    fn read_name(&mut self, term: char) -> Result<String, ParseError> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c == term { break; }
            s.push(c);
            self.bump();
        }
        if self.peek() != Some(term) { return Err(self.err("missing group name terminator")); }
        self.bump();
        if s.is_empty() { return Err(self.err("missing group name")); }
        Ok(s)
    }

    fn read_flags(&mut self) -> Result<(), ParseError> {
        while let Some(c) = self.peek() {
            match c {
                'i' => { self.flags.ignorecase = true; self.bump(); }
                's' => { self.flags.dotall = true; self.bump(); }
                'm' => { self.flags.multiline = true; self.bump(); }
                'a' | 'L' | 'u' | 'x' => { self.bump(); } // accepted but inert in this version
                ')' => break,
                _ => return Err(self.err("unknown flag")),
            }
        }
        Ok(())
    }

    fn class(&mut self) -> Result<Node, ParseError> {
        self.bump(); // open bracket
        let negated = if self.peek() == Some('^') { self.bump(); true } else { false };
        let mut items = Vec::new();
        if self.peek() == Some(']') { items.push(ClassItem::Ch(']')); self.bump(); }
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated character set")),
                Some(']') => { self.bump(); break; }
                Some('\\') => { self.bump(); items.push(self.class_escape()?); }
                Some(c) => {
                    self.bump();
                    let is_range = self.peek() == Some('-')
                        && self.at(1).is_some()
                        && self.at(1) != Some(']');
                    if is_range {
                        self.bump(); // dash
                        let end = if self.peek() == Some('\\') {
                            self.bump();
                            match self.class_escape()? {
                                ClassItem::Ch(e) => e,
                                _ => return Err(self.err("bad character range")),
                            }
                        } else {
                            self.bump().unwrap()
                        };
                        if (end as u32) < (c as u32) { return Err(self.err("bad character range")); }
                        items.push(ClassItem::Range(c, end));
                    } else {
                        items.push(ClassItem::Ch(c));
                    }
                }
            }
        }
        Ok(Node::Class { items, negated })
    }

    fn class_escape(&mut self) -> Result<ClassItem, ParseError> {
        let c = self.bump().ok_or(self.err("trailing backslash"))?;
        Ok(match c {
            'd' => ClassItem::Digit,
            'D' => ClassItem::NotDigit,
            'w' => ClassItem::Word,
            'W' => ClassItem::NotWord,
            's' => ClassItem::Space,
            'S' => ClassItem::NotSpace,
            'n' => ClassItem::Ch('\n'),
            't' => ClassItem::Ch('\t'),
            'r' => ClassItem::Ch('\r'),
            'f' => ClassItem::Ch('\u{0C}'),
            'v' => ClassItem::Ch('\u{0B}'),
            'a' => ClassItem::Ch('\u{07}'),
            'b' => ClassItem::Ch('\u{08}'), // backspace inside a set
            '0' => ClassItem::Ch('\0'),
            'x' => ClassItem::Ch(self.read_hex(2)?),
            'u' => ClassItem::Ch(self.read_hex(4)?),
            other => ClassItem::Ch(other), // lenient, escaped literal
        })
    }

    fn escape(&mut self) -> Result<Node, ParseError> {
        self.bump(); // backslash
        let c = self.bump().ok_or(self.err("trailing backslash"))?;
        Ok(match c {
            'd' => Node::Class { items: single(ClassItem::Digit), negated: false },
            'D' => Node::Class { items: single(ClassItem::NotDigit), negated: false },
            'w' => Node::Class { items: single(ClassItem::Word), negated: false },
            'W' => Node::Class { items: single(ClassItem::NotWord), negated: false },
            's' => Node::Class { items: single(ClassItem::Space), negated: false },
            'S' => Node::Class { items: single(ClassItem::NotSpace), negated: false },
            'b' => Node::WordBoundary,
            'B' => Node::NotWordBoundary,
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            'r' => Node::Char('\r'),
            'f' => Node::Char('\u{0C}'),
            'v' => Node::Char('\u{0B}'),
            'a' => Node::Char('\u{07}'),
            '0' => Node::Char('\0'),
            'x' => Node::Char(self.read_hex(2)?),
            'u' => Node::Char(self.read_hex(4)?),
            '1'..='9' => {
                let mut n = c.to_digit(10).unwrap() as usize;
                while let Some(d) = self.peek().and_then(|x| x.to_digit(10)) {
                    let cand = n * 10 + d as usize;
                    if cand <= self.group_count { n = cand; self.bump(); } else { break; }
                }
                Node::Backref(n)
            }
            l if l.is_ascii_alphabetic() => return Err(self.err("bad escape")),
            other => Node::Char(other), // escaped metacharacter
        })
    }

    fn read_hex(&mut self, n: usize) -> Result<char, ParseError> {
        let mut acc: u32 = 0;
        for _ in 0..n {
            let d = self.peek().and_then(|c| c.to_digit(16)).ok_or(self.err("bad hex escape"))?;
            acc = acc * 16 + d;
            self.bump();
        }
        char::from_u32(acc).ok_or(self.err("invalid codepoint"))
    }
}

fn single(item: ClassItem) -> Vec<ClassItem> {
    alloc::vec![item]
}

fn is_flag_char(c: char) -> bool {
    matches!(c, 'i' | 's' | 'm' | 'a' | 'L' | 'u' | 'x')
}
