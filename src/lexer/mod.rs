pub mod tables;
pub use tables::utf8_char_len;

mod scan;

use scan::Scanner;
use alloc::vec::Vec;

const MAX_SOURCE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub struct Token {
    pub kind: TokenType,
    pub line: usize,
    pub start: usize,
    pub end: usize,
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
            alloc::vec![Token { kind: TokenType::Endmarker, line: 0, start: len, end: len }],
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

    let mut tokens = Vec::with_capacity(raw.len());
    let mut ended = false;
    for i in 0..raw.len() {
        let (tok, line, start, end) = raw[i];
        if ended { break; }
        if tok == TokenType::Endmarker { ended = true; }

        /* `match` and `case` stay keywords only in a header, `type` only in an alias. */
        let kind = match tok {
            TokenType::Match | TokenType::Case if !(starts_stmt(&raw, i) && ends_in_colon(&raw, i)) => TokenType::Name,
            TokenType::Type if !(starts_stmt(&raw, i) && matches!(raw.get(i + 1), Some(&(TokenType::Name, ..)))) => TokenType::Name,
            _ => tok,
        };
        tokens.push(Token { kind, line, start, end });
    }
    (tokens, scanner.errors)
}

fn starts_stmt(raw: &[(TokenType, usize, usize, usize)], i: usize) -> bool {
    i == 0 || matches!(raw[i - 1].0, TokenType::Newline | TokenType::Indent | TokenType::Dedent | TokenType::Semi)
}

/* Only a colon outside brackets counts, so a slice or a dict never makes a header. */
fn ends_in_colon(raw: &[(TokenType, usize, usize, usize)], i: usize) -> bool {
    let mut depth = 0usize;
    for j in i + 1..raw.len() {
        match raw[j].0 {
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace => depth += 1,
            TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace => depth = depth.saturating_sub(1),
            TokenType::Newline | TokenType::Semi | TokenType::Endmarker if depth == 0 => {
                return j > i + 1 && raw[j - 1].0 == TokenType::Colon;
            }
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
