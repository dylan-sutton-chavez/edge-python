/* 
lexer.rs
  Reads raw bytes, emits tokens with start/end positions.
  No strings, no copies — offsets into the original buffer only.
*/

use logos::{Logos, Lexer};
use std::collections::VecDeque;
use std::cmp::Ordering;

#[derive(Default)]
pub struct LexerExtras {

  /*
  Shared mutable state between lexer callbacks: buffers pending tokens, tracks indentation depth and open brackets.
  */

  pending: VecDeque<TokenType>,
  indent_stack: Vec<usize>,
  nesting: u32

}

fn open_bracket(lex: &mut Lexer<TokenType>) {
    lex.extras.nesting += 1;
}

fn close_bracket(lex: &mut Lexer<TokenType>) {
    lex.extras.nesting = lex.extras.nesting.saturating_sub(1);
}
    
const MAX_INDENT_DEPTH: usize = 100; // A04:2021 prevents asymetric DoS via deeply nested indentation.

fn on_newline(lex: &mut Lexer<TokenType>) -> logos::Skip {

  /* 
  Decides if \n is a statement boundary or inside brackets.
  */

  let src = lex.remainder();
  let pending = &mut lex.extras.pending;

  if lex.extras.nesting > 0 {
    pending.push_back(TokenType::Nl);
    return logos::Skip;
  }

  let spaces = src.bytes().take_while(|&b| b == b' ').count();
  let tabs = src.bytes().take_while(|&b| b == b'\t').count();
  let next = src.trim_start_matches([' ', '\t']).chars().next();

  if spaces > 0 && tabs > 0 { pending.push_back(TokenType::Endmarker); return logos::Skip; }
  if matches!(next, Some('\n') | Some('\r') | Some('#')) { pending.push_back(TokenType::Nl); return logos::Skip; }

  let level = spaces + tabs;
  let current = *lex.extras.indent_stack.last().unwrap_or(&0);

  pending.push_back(TokenType::Newline);

  match level.cmp(&current) {

    Ordering::Greater => {
      if lex.extras.indent_stack.len() >= MAX_INDENT_DEPTH { pending.push_back(TokenType::Endmarker); return logos::Skip; }
        lex.extras.indent_stack.push(level);
        pending.push_back(TokenType::Indent);
    }
    Ordering::Less  => while lex.extras.indent_stack.last().is_some_and(|&t| t > level) {
      lex.extras.indent_stack.pop();
      pending.push_back(TokenType::Dedent);
    },
    Ordering::Equal => {}

  }

  logos::Skip

}

const MAX_FSTRING_DEPTH: usize = 200; // A04:2021 prevents asymetric DoS via deeply nested f-string brace expressions.

fn scan_fstring(lex: &mut Lexer<TokenType>, quote: u8, triple: bool) {

  /*
  Scans f-string body, pushing FstringMiddle and FstringEnd to pending.
  */

  let mut depth = 0usize;
  let mut had_expr = false;
  let mut pos = 0usize;
  let bytes = lex.remainder().as_bytes();

  while pos < bytes.len() {
    let closes = if triple {
      bytes.get(pos..pos + 3) == Some(&[quote, quote, quote])
    } else {
      bytes[pos] == quote && depth == 0
    };

    if closes {
      if had_expr {
        lex.extras.pending.push_back(TokenType::FstringMiddle);
      }
      lex.bump(pos + if triple { 3 } else { 1 });  // ← bump contenido + cierre
      lex.extras.pending.push_back(TokenType::FstringEnd);
      return;
    }

    match bytes[pos] {
      b'\\' => pos += 2,
      b'{' => { had_expr = true; depth = (depth + 1).min(MAX_FSTRING_DEPTH); pos += 1; }
      b'}' => { depth = depth.saturating_sub(1); pos += 1; }
      _ => pos += 1
    }
  }
}

fn on_name(lex: &mut Lexer<TokenType>) -> Option<()> {

  /*
  Detects f-string prefixes within identifier matches and delegates to scan_fstring.
  */

  let slice = lex.slice();
  let is_fprefix = matches!(slice.to_ascii_lowercase().as_str(), "f" | "fr" | "rf");

  if !is_fprefix {
    return Some(());
  }

  if let Some(&q) = lex.remainder().as_bytes().first() {
    if matches!(q, b'"' | b'\'') {
      let triple = lex.remainder().as_bytes().get(1) == Some(&q);
      lex.bump(if triple { 3 } else { 1 });
      lex.extras.pending.push_back(TokenType::FstringStart);
      scan_fstring(lex, q, triple);
      return None;
    }
  }

  return Some(());

}

#[derive(Logos, Debug, PartialEq)]
#[logos(extras = LexerExtras)]
#[logos(skip r"[ \t\r]+")]
pub enum TokenType {

  /* 
  Keywords
  */

  #[token("False")] False,
  #[token("None")] None,
  #[token("True")] True,
  #[token("and")] And,
  #[token("as")] As,
  #[token("assert")] Assert,
  #[token("async")] Async,
  #[token("await")] Await,
  #[token("break")] Break,
  #[token("class")] Class,
  #[token("continue")] Continue,
  #[token("def")] Def,
  #[token("del")] Del,
  #[token("elif")] Elif,
  #[token("else")] Else,
  #[token("except")] Except,
  #[token("finally")] Finally,
  #[token("for")] For,
  #[token("from")] From,
  #[token("global")] Global,
  #[token("if")] If,
  #[token("import")] Import,
  #[token("in")] In,
  #[token("is")] Is,
  #[token("lambda")] Lambda,
  #[token("nonlocal")] Nonlocal,
  #[token("not")] Not,
  #[token("or")] Or,
  #[token("pass")] Pass,
  #[token("raise")] Raise,
  #[token("return")] Return,
  #[token("try")] Try,
  #[token("while")] While,
  #[token("with")] With,
  #[token("yield")] Yield,

  /*
  Soft keywords
  */

  #[token("case")] Case,
  #[token("match")] Match,
  #[token("type")] Type,
  #[token("_", priority = 3)] Underscore,

  /*
  Operators
  */

  #[token("**=")] DoubleStarEqual,
  #[token("//=")] DoubleSlashEqual,
  #[token("<<=")] LeftShiftEqual,
  #[token(">>=")] RightShiftEqual,

  #[token("!=")] NotEqual,
  #[token("%=")] PercentEqual,
  #[token("&=")] AmperEqual,
  #[token("**")] DoubleStar,
  #[token("*=")] StarEqual,
  #[token("+=")] PlusEqual,
  #[token("-=")] MinEqual,
  #[token("->")] Rarrow,
  #[token("...")] Ellipsis,
  #[token("//")] DoubleSlash,
  #[token("/=")] SlashEqual,
  #[token(":=")] ColonEqual,
  #[token("<<")] LeftShift,
  #[token("<=")] LessEqual,
  #[token("==")] EqEqual,
  #[token(">=")] GreaterEqual,
  #[token(">>")] RightShift,
  #[token("@=")] AtEqual,
  #[token("^=")] CircumflexEqual,
  #[token("|=")] VbarEqual,

  #[token("!")] Exclamation,
  #[token("%")] Percent,
  #[token("&")] Amper,
  #[token("*")] Star,
  #[token("+")] Plus,
  #[token("-")] Minus,
  #[token(".")] Dot,
  #[token("/")] Slash,
  #[token("<")] Less,
  #[token("=")] Equal,
  #[token(">")] Greater,
  #[token("@")] At,
  #[token("^")] Circumflex,
  #[token("|")] Vbar,
  #[token("~")] Tilde,
  #[token(",")] Comma,
  #[token(":")] Colon,
  #[token(";")] Semi,

  /*
  Delimitors
  */

  #[token("(", open_bracket)]  Lpar,
  #[token(")", close_bracket)] Rpar,
  #[token("[", open_bracket)]  Lsqb,
  #[token("]", close_bracket)] Rsqb,
  #[token("{", open_bracket)]  Lbrace,
  #[token("}", close_bracket)] Rbrace,

  /*
  Token names
  */

  #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", on_name)] Name,

  #[regex(r"[0-9]+[jJ]")]
  #[regex(r"[0-9]+\.[0-9]*([eE][+-]?[0-9]+)?[jJ]")]
  #[regex(r"\.[0-9]+([eE][+-]?[0-9]+)?[jJ]")]
  Complex,

  #[regex(r"[0-9]+\.[0-9]*([eE][+-]?[0-9]+)?")]
  #[regex(r"\.[0-9]+([eE][+-]?[0-9]+)?")]
  #[regex(r"[0-9]+[eE][+-]?[0-9]+")]
  Float,

  #[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*")]
  #[regex(r"0[oO][0-7][0-7_]*")]
  #[regex(r"0[bB][01][01_]*")]
  #[regex(r"[1-9][0-9_]*|0")]
  Int,

  #[regex(r#"[bBrRuU]{0,2}"""([^"\\]|\\.)*""""#)]
  #[regex(r#"[bBrRuU]{0,2}'''([^'\\]|\\.)*'''"#)]
  #[regex(r#"[bBrRuU]{0,2}"([^"\\\n]|\\.)*""#)]
  #[regex(r#"[bBrRuU]{0,2}'([^'\\\n]|\\.)*'"#)]
  String,

  FstringStart,
  FstringMiddle,
  FstringEnd,

  #[regex(r"#[^\n]*", allow_greedy = true)] Comment,

  #[token("\n", on_newline)] Newline,

  Indent,
  Dedent,

  Nl,

  Endmarker

}

struct TokenStream<'src> {

  /*
  Thin iterator over logos lexer, drains pending queue before pulling next token.
  */

  inner: Lexer<'src, TokenType>,
  done: bool

}

impl<'src> TokenStream<'src> {

  /*
  Constructs TokenStream from raw source string, initializing logos lexer.
  */

  fn new(source: &'src str) -> Self {
    Self { inner: TokenType::lexer(source), done: false }
  }

}

impl Iterator for TokenStream<'_> {

  /*
  Emits pending tokens first, then logos tokens, then Endmarker on exhaustion.
  */

  type Item = TokenType;

  fn next(&mut self) -> Option<TokenType> {

    if let Some(tok) = self.inner.extras.pending.pop_front() { return Some(tok); }

    let result = match self.inner.next() {
      Some(Ok(tok)) => Some(tok),
      Some(Err(_)) if !self.inner.extras.pending.is_empty() => None,
      Some(Err(_)) => Some(TokenType::Endmarker),
      None if !self.done => { self.done = true; Some(TokenType::Endmarker) }
      None => None
    };

    if !self.inner.extras.pending.is_empty() {
      if let Some(tok) = result {
        self.inner.extras.pending.push_back(tok);
      }
      return self.inner.extras.pending.pop_front();
    }

    result

  }

}

struct SoftKeywordTransformer<I: Iterator<Item = TokenType>> {

  /*
  Wraps token iterator, converts soft keywords to Name in identifier position.
  */

  inner: I,
  pending: Option<TokenType>

}

impl<I: Iterator<Item = TokenType>> SoftKeywordTransformer<I> {

  /*
  Constructs transformer wrapping any upstream token iterator.
  */

  fn new(inner: I) -> Self {
    Self { inner, pending: None }
  }

}

impl<I: Iterator<Item = TokenType>> Iterator for SoftKeywordTransformer<I> {

  /*
  Peeks one token ahead to resolve soft keyword vs identifier context.
  */

  type Item = TokenType;

  fn next(&mut self) -> Option<TokenType> {

    let tok = self.pending.take().or_else(|| self.inner.next())?;

    match tok {

      TokenType::Match | TokenType::Case | TokenType::Type => {

        let next = self.inner.next();

        let as_name = matches!(next,
          Some(TokenType::Lpar) | Some(TokenType::Colon) |
          Some(TokenType::Equal) | Some(TokenType::Comma) |
          Some(TokenType::Rpar) | Some(TokenType::Rsqb) |
          Some(TokenType::Newline) | None
        );

        self.pending = next;
        if as_name { Some(TokenType::Name) } else { Some(tok) }

      }

      _ => Some(tok)

    }

  }

}

pub fn lexer(source: &str) -> impl Iterator<Item = TokenType> + '_ {

  /*
  Tokenizes Python source into a complete, parser ready token stream.
  */

  SoftKeywordTransformer::new(TokenStream::new(source))

}