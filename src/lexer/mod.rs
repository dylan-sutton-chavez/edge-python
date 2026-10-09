pub mod tables;
pub use tables::utf8_char_len;

mod scan;

use scan::Scanner;
use alloc::vec::Vec;

const MAX_SOURCE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: TokenType,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    /* Starts the element of a comprehension, so the parser compiles its clauses first. */
    pub comp: bool,
}

/* Lex-time diagnostic. Static message since errors are a fixed set, parser boundary upgrades it to a richer Diagnostic. */
#[derive(Debug)]
pub struct LexError {
    pub start: usize,
    pub end: usize,
    pub msg: &'static str,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TokenType {
    // Keywords
    False, None, True, And, As, Assert, Async, Await, Break, Class, Continue, Def, Del,
    Elif, Else, Except, Finally, For, From, Global, If, Import, In, Is, Lambda, Nonlocal,
    Not, Or, Pass, Raise, Return, Try, While, With, Yield,
    // Soft keywords
    Case, Match, Type, Underscore,
    // Operators (3-char)
    DoubleStarEqual, DoubleSlashEqual, LeftShiftEqual, RightShiftEqual,
    // Operators (2-char)
    NotEqual, PercentEqual, AmperEqual, DoubleStar, StarEqual, PlusEqual, MinEqual,
    Rarrow, Ellipsis, DoubleSlash, SlashEqual, ColonEqual, LeftShift, LessEqual,
    EqEqual, GreaterEqual, RightShift, AtEqual, CircumflexEqual, VbarEqual,
    // Operators (1-char)
    Exclamation, Percent, Amper, Star, Plus, Minus, Dot, Slash, Less, Equal, Greater,
    At, Circumflex, Vbar, Tilde, Comma, Colon, Semi,
    // Delimiters
    Lpar, Rpar, Lsqb, Rsqb, Lbrace, Rbrace,
    // Literals
    Name, Float, Int, String, Bytes,
    // F-string
    FstringStart, FstringMiddle, FstringEnd,
    // Whitespace and structure
    Comment, Newline, Indent, Dedent, Nl, Endmarker,
}

/* Parser-ready tokens with indentation handled. Returns lex diagnostics alongside so the caller folds them into the parser's error stream. */
pub fn lex(source: &str) -> (Vec<Token>, Vec<LexError>) {
    let bytes = source.as_bytes();
    let len = source.len();
    let mut scanner = Scanner::new(bytes);
    // Skip UTF-8 BOM so it doesn't fuse into the first identifier.
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { scanner.pos = 3; }

    if len > MAX_SOURCE_SIZE {
        scanner.errors.push(LexError {
            start: 0, end: 0,
            msg: "source file exceeds maximum size (10 MiB)",
        });
        return (
            alloc::vec![Token { kind: TokenType::Endmarker, line: 0, start: len, end: len, comp: false }],
            scanner.errors,
        );
    }

    let mut raw: Vec<(TokenType, usize, usize, usize)> = Vec::new();
    while let Some(t) = scanner.next_token() {
        raw.push(t);
    }
    // Drain dangling indents at EOF so blocks always close cleanly.
    while scanner.indent_stack.pop().is_some() {
        raw.push((TokenType::Dedent, scanner.line, len, len));
    }
    raw.push((TokenType::Endmarker, scanner.line, len, len));

    let mut tokens: Vec<Token> = Vec::with_capacity(raw.len());
    let mut ended = false;
    for i in 0..raw.len() {
        let (tok, line, start, end) = raw[i];
        if ended { break; }
        if tok == TokenType::Endmarker { ended = true; }

        /* `match` and `case` stay keywords only in a header, `type` only in an alias. */
        let kind = match tok {
            TokenType::Match | TokenType::Case if !(starts_stmt(&raw, i) && opens_header(&raw, i)) => TokenType::Name,
            TokenType::Type if !(starts_stmt(&raw, i) && matches!(raw.get(i + 1), Some(&(TokenType::Name, ..)))) => TokenType::Name,
            _ => tok,
        };
        if kind == TokenType::For && let Some(first) = comp_start(&tokens) { tokens[first].comp = true; }
        tokens.push(Token { kind, line, start, end, comp: false });
    }
    (tokens, scanner.errors)
}

/* The first token of the item a `for` ends inside brackets, else None. */
fn comp_start(tokens: &[Token]) -> Option<usize> {
    // Commas between a lambda and its colon separate its parameters, not items.
    let (mut depth, mut item, mut params) = (0usize, None, false);
    for (i, t) in tokens.iter().enumerate().rev() {
        match t.kind {
            TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace => depth += 1,
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace if depth > 0 => depth -= 1,
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace => { item = item.or(Some(i + 1)); break; }
            TokenType::Colon if depth == 0 => params = true,
            TokenType::Lambda if depth == 0 => params = false,
            TokenType::Comma if depth == 0 && !params => item = item.or(Some(i + 1)),
            TokenType::Newline | TokenType::Indent | TokenType::Dedent | TokenType::Semi => return None,
            _ => {}
        }
        if i == 0 { return None; }
    }
    (item?..tokens.len()).find(|&j| !matches!(tokens[j].kind, TokenType::Nl | TokenType::Comment))
}

fn starts_stmt(raw: &[(TokenType, usize, usize, usize)], i: usize) -> bool {
    i == 0 || matches!(raw[i - 1].0, TokenType::Newline | TokenType::Indent | TokenType::Dedent | TokenType::Semi)
}

/* A header colon sits outside brackets and before any `=`, so a body may follow it. */
fn opens_header(raw: &[(TokenType, usize, usize, usize)], i: usize) -> bool {
    let mut depth = 0usize;
    for (j, &(tok, ..)) in raw.iter().enumerate().skip(i + 1) {
        match tok {
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace => depth += 1,
            TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace => depth = depth.saturating_sub(1),
            TokenType::Colon if depth == 0 => return j > i + 1,
            TokenType::Equal | TokenType::Newline | TokenType::Semi | TokenType::Endmarker if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

impl TokenType {
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        tables::token_to_str(self)
    }
}
