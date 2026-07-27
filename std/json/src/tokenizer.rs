/*
Streaming JSON tokenizer. `next_token` advances one token, tracks byte offset. `Int`/`Float` carry source slice for `parse_int`/`parse_float`. `Constant` covers CPython tokens `NaN`/`Infinity`/`-Infinity`.
*/

use alloc::string::{String, ToString};

pub enum Token {
    LBrace, RBrace, LBracket, RBracket, Comma, Colon,
    Null, True, False,
    Str(String),
    Int(i128, String),
    Float(f64, String),
    Constant(String),
    Eof,
}

pub struct JsonError {
    pub msg: String,
    pub pos: usize,
}

pub struct Tokenizer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src: src.as_bytes(), pos: 0 }
    }

    pub fn pos(&self) -> usize { self.pos }

    fn err(&self, msg: impl Into<String>) -> JsonError {
        JsonError { msg: msg.into(), pos: self.pos }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token, JsonError> {
        self.skip_ws();
        if self.pos >= self.src.len() { return Ok(Token::Eof); }
        let c = self.src[self.pos];
        match c {
            b'{' => { self.pos += 1; Ok(Token::LBrace) }
            b'}' => { self.pos += 1; Ok(Token::RBrace) }
            b'[' => { self.pos += 1; Ok(Token::LBracket) }
            b']' => { self.pos += 1; Ok(Token::RBracket) }
            b',' => { self.pos += 1; Ok(Token::Comma) }
            b':' => { self.pos += 1; Ok(Token::Colon) }
            b'"' => self.read_string(),
            b'-' => self.read_number_or_neg_infinity(),
            b'0'..=b'9' => self.read_number(),
            b't' => self.read_keyword(b"true", Token::True),
            b'f' => self.read_keyword(b"false", Token::False),
            b'n' => self.read_keyword(b"null", Token::Null),
            b'N' => self.read_keyword(b"NaN", Token::Constant("NaN".to_string())),
            b'I' => self.read_keyword(b"Infinity", Token::Constant("Infinity".to_string())),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn read_keyword(&mut self, kw: &[u8], tok: Token) -> Result<Token, JsonError> {
        if self.src.len() < self.pos + kw.len() || &self.src[self.pos..self.pos + kw.len()] != kw {
            return Err(self.err("unknown literal"));
        }
        self.pos += kw.len();
        Ok(tok)
    }

    fn read_number_or_neg_infinity(&mut self) -> Result<Token, JsonError> {
        // `-Infinity` is the only number-like token that starts with `-` but isn't a digit sequence.
        if self.src.len() >= self.pos + 9 && &self.src[self.pos..self.pos + 9] == b"-Infinity" {
            self.pos += 9;
            return Ok(Token::Constant("-Infinity".to_string()));
        }
        self.read_number()
    }

    fn read_string(&mut self) -> Result<Token, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c == b'"' { self.pos += 1; return Ok(Token::Str(out)); }
            if c == b'\\' {
                self.pos += 1;
                if self.pos >= self.src.len() { return Err(self.err("dangling escape")); }
                let esc = self.src[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let cp = self.read_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if self.src.len() < self.pos + 2 || self.src[self.pos] != b'\\' || self.src[self.pos + 1] != b'u' {
                                return Err(self.err("unpaired high surrogate"));
                            }
                            self.pos += 2;
                            let low = self.read_hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(self.err("invalid low surrogate"));
                            }
                            let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                            out.push(char::from_u32(combined).ok_or_else(|| self.err("invalid surrogate pair"))?);
                        } else if (0xDC00..=0xDFFF).contains(&cp) {
                            return Err(self.err("unexpected low surrogate"));
                        } else {
                            out.push(char::from_u32(cp).ok_or_else(|| self.err("invalid unicode escape"))?);
                        }
                    }
                    _ => return Err(self.err("invalid escape")),
                }
                continue;
            }
            if c < 0x20 { return Err(self.err("control character in string")); }
            let len = utf8_len(c).ok_or_else(|| self.err("invalid utf-8 lead byte"))?;
            if self.pos + len > self.src.len() { return Err(self.err("truncated utf-8")); }
            let bytes = &self.src[self.pos..self.pos + len];
            let s = core::str::from_utf8(bytes).map_err(|_| self.err("invalid utf-8"))?;
            out.push_str(s);
            self.pos += len;
        }
        Err(self.err("unterminated string"))
    }

    fn read_hex4(&mut self) -> Result<u32, JsonError> {
        if self.pos + 4 > self.src.len() { return Err(self.err("truncated \\u escape")); }
        let mut acc: u32 = 0;
        for _ in 0..4 {
            let d = (self.src[self.pos] as char).to_digit(16).ok_or_else(|| self.err("invalid hex digit"))?;
            acc = acc * 16 + d;
            self.pos += 1;
        }
        Ok(acc)
    }

    fn read_number(&mut self) -> Result<Token, JsonError> {
        let start = self.pos;
        let mut is_float = false;
        if self.src[self.pos] == b'-' { self.pos += 1; }
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() { self.pos += 1; }
        if self.pos < self.src.len() && self.src[self.pos] == b'.' {
            is_float = true;
            self.pos += 1;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() { self.pos += 1; }
        }
        if self.pos < self.src.len() && matches!(self.src[self.pos], b'e' | b'E') {
            is_float = true;
            self.pos += 1;
            if self.pos < self.src.len() && matches!(self.src[self.pos], b'+' | b'-') { self.pos += 1; }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() { self.pos += 1; }
        }
        let text = core::str::from_utf8(&self.src[start..self.pos]).map_err(|_| self.err("non-utf8 number"))?.to_string();
        if is_float {
            text.parse::<f64>().map(|f| Token::Float(f, text.clone())).map_err(|_| JsonError { msg: "invalid float".to_string(), pos: start })
        } else {
            text.parse::<i128>().map(|i| Token::Int(i, text.clone())).map_err(|_| JsonError { msg: "integer overflow".to_string(), pos: start })
        }
    }
}

fn utf8_len(b: u8) -> Option<usize> {
    if b < 0x80 { Some(1) }
    else if b & 0xE0 == 0xC0 { Some(2) }
    else if b & 0xF0 == 0xE0 { Some(3) }
    else if b & 0xF8 == 0xF0 { Some(4) }
    else { None }
}
