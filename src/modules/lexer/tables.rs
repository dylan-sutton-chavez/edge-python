use super::TokenType;

/* Byte-class flags packed into a u8; combine with bitwise OR for membership tests. */
pub const ID_START: u8 = 1;
pub const ID_CONT: u8 = 2;
pub const DIGIT: u8 = 4;
pub const SPACE: u8 = 8;

// 256-byte LUT: replaces per-byte branches with indexed flag loads.
pub static BYTE_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let b = i as u8;
        if b == b' ' || b == b'\t' || b == b'\r' { t[i] = SPACE; }
        if b >= b'0' && b <= b'9' { t[i] = DIGIT | ID_CONT; }
        if (b >= b'a' && b <= b'z') || (b >= b'A' && b <= b'Z') {
            t[i] = ID_START | ID_CONT;
        }
        if b == b'_' { t[i] = ID_START | ID_CONT; }
        if b >= 0x80 { t[i] = ID_START | ID_CONT; } // Non-ASCII always starts/continues an identifier.
        i += 1;
    }
    t
};

/* Single-char operator dispatch: byte -> index into SINGLE_MAP. */
pub static SINGLE_TOK: [u8; 128] = {
    let mut t = [0u8; 128];
    t[b'(' as usize] = 1; t[b')' as usize] = 2;
    t[b'[' as usize] = 3; t[b']' as usize] = 4;
    t[b'{' as usize] = 5;
    t[b'!' as usize] = 6; t[b'%' as usize] = 7;
    t[b'&' as usize] = 8; t[b'*' as usize] = 9;
    t[b'+' as usize] = 10; t[b'-' as usize] = 11;
    t[b'.' as usize] = 12; t[b'/' as usize] = 13;
    t[b'<' as usize] = 14; t[b'=' as usize] = 15;
    t[b'>' as usize] = 16; t[b'@' as usize] = 17;
    t[b'^' as usize] = 18; t[b'|' as usize] = 19;
    t[b'~' as usize] = 20; t[b',' as usize] = 21;
    t[b':' as usize] = 22; t[b';' as usize] = 23;
    t
};

/* SINGLE_TOK index -> TokenType. Index 0 = not an operator. */
pub const SINGLE_MAP: [TokenType; 24] = [
    TokenType::Endmarker, // 0 = not found
    TokenType::Lpar, TokenType::Rpar, TokenType::Lsqb, // 1-3
    TokenType::Rsqb, TokenType::Lbrace, // 4-5
    TokenType::Exclamation, TokenType::Percent, // 6-7
    TokenType::Amper, TokenType::Star, TokenType::Plus, // 8-10
    TokenType::Minus, TokenType::Dot, TokenType::Slash, // 11-13
    TokenType::Less, TokenType::Equal, TokenType::Greater, // 14-16
    TokenType::At, TokenType::Circumflex, TokenType::Vbar, // 17-19
    TokenType::Tilde, TokenType::Comma, TokenType::Colon, // 20-22
    TokenType::Semi, // 23
];

/* Routes by (length, first byte) to skip most memcmps. */
pub fn keyword(s: &[u8]) -> Option<TokenType> {
    match (s.len(), s[0]) {
        (1, b'_') => Some(TokenType::Underscore),

        (2, b'a') => if s[1] == b's' { Some(TokenType::As) } else { None },
        (2, b'i') => match s[1] {
            b'f' => Some(TokenType::If),
            b'n' => Some(TokenType::In),
            b's' => Some(TokenType::Is),
            _ => None,
        },
        (2, b'o') => if s[1] == b'r' { Some(TokenType::Or) } else { None },

        (3, b'a') => if s[1] == b'n' && s[2] == b'd' { Some(TokenType::And) } else { None },
        (3, b'd') => if s[1] == b'e' {
            match s[2] { b'f' => Some(TokenType::Def), b'l' => Some(TokenType::Del), _ => None }
        } else { None },
        (3, b'f') => if s[1] == b'o' && s[2] == b'r' { Some(TokenType::For) } else { None },
        (3, b'n') => if s[1] == b'o' && s[2] == b't' { Some(TokenType::Not) } else { None },
        (3, b't') => if s[1] == b'r' && s[2] == b'y' { Some(TokenType::Try) } else { None },

        (4, b'c') => if s == b"case" { Some(TokenType::Case) } else { None },
        (4, b'e') => match s[2] {
            b'i' => if s == b"elif" { Some(TokenType::Elif) } else { None },
            b's' => if s == b"else" { Some(TokenType::Else) } else { None },
            _ => None,
        },
        (4, b'f') => if s == b"from" { Some(TokenType::From) } else { None },
        (4, b'N') => if s == b"None" { Some(TokenType::None) } else { None },
        (4, b'p') => if s == b"pass" { Some(TokenType::Pass) } else { None },
        (4, b'T') => if s == b"True" { Some(TokenType::True) } else { None },
        (4, b't') => if s == b"type" { Some(TokenType::Type) } else { None },
        (4, b'w') => if s == b"with" { Some(TokenType::With) } else { None },

        (5, b'a') => match s[1] {
            b's' => if s == b"async" { Some(TokenType::Async) } else { None },
            b'w' => if s == b"await" { Some(TokenType::Await) } else { None },
            _ => None,
        },
        (5, b'b') => if s == b"break" { Some(TokenType::Break) } else { None },
        (5, b'c') => if s == b"class" { Some(TokenType::Class) } else { None },
        (5, b'F') => if s == b"False" { Some(TokenType::False) } else { None },
        (5, b'm') => if s == b"match" { Some(TokenType::Match) } else { None },
        (5, b'r') => if s == b"raise" { Some(TokenType::Raise) } else { None },
        (5, b'w') => if s == b"while" { Some(TokenType::While) } else { None },
        (5, b'y') => if s == b"yield" { Some(TokenType::Yield) } else { None },

        (6, b'a') => if s == b"assert" { Some(TokenType::Assert) } else { None },
        (6, b'e') => if s == b"except" { Some(TokenType::Except) } else { None },
        (6, b'g') => if s == b"global" { Some(TokenType::Global) } else { None },
        (6, b'i') => if s == b"import" { Some(TokenType::Import) } else { None },
        (6, b'l') => if s == b"lambda" { Some(TokenType::Lambda) } else { None },
        (6, b'r') => if s == b"return" { Some(TokenType::Return) } else { None },

        (7, b'f') => if s == b"finally" { Some(TokenType::Finally) } else { None },

        (8, b'c') => if s == b"continue" { Some(TokenType::Continue) } else { None },
        (8, b'n') => if s == b"nonlocal" { Some(TokenType::Nonlocal) } else { None },

        _ => None,
    }
}


/* f-string prefix: f, F, fr, Fr, fR, FR, rf, rF, Rf, RF. */
#[inline]
pub fn is_fstring_prefix(s: &[u8]) -> bool {
    match s.len() {
        1 => matches!(s[0], b'f' | b'F'),
        2 => matches!(
            (s[0], s[1]),
            (b'f' | b'F', b'r' | b'R') | (b'r' | b'R', b'f' | b'F')
        ),
        _ => false,
    }
}

/* Regular string prefix: r, R, u, U. The b/B variants live in `is_bytes_prefix` to emit a distinct `Bytes` token. */
#[inline]
pub fn is_string_prefix(s: &[u8]) -> bool {
    match s.len() {
        1 => matches!(s[0], b'r' | b'R' | b'u' | b'U'),
        _ => false,
    }
}

/* Bytes literal prefix: b, B, br, Br, bR, BR, rb, Rb, rB, RB. Emitted as `Bytes` so the parser produces `Value::Bytes`. */
#[inline]
pub fn is_bytes_prefix(s: &[u8]) -> bool {
    match s.len() {
        1 => matches!(s[0], b'b' | b'B'),
        2 => matches!(
            (s[0], s[1]),
            (b'b' | b'B', b'r' | b'R') | (b'r' | b'R', b'b' | b'B')
        ),
        _ => false,
    }
}

/* UTF-8 byte length from lead byte; supports non-ASCII identifiers. */
#[inline]
pub fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/* Reverse lookup for diagnostics: TokenType -> quoted display name. */
pub const fn token_to_str(kind: &TokenType) -> &'static str {
    match kind {
        // Keywords
        TokenType::False => "'False'", TokenType::None => "'None'", TokenType::True => "'True'",
        TokenType::And => "'and'", TokenType::As => "'as'", TokenType::Assert => "'assert'",
        TokenType::Async => "'async'", TokenType::Await => "'await'", TokenType::Break => "'break'",
        TokenType::Class => "'class'", TokenType::Continue => "'continue'", TokenType::Def => "'def'",
        TokenType::Del => "'del'", TokenType::Elif => "'elif'", TokenType::Else => "'else'",
        TokenType::Except => "'except'", TokenType::Finally => "'finally'", TokenType::For => "'for'",
        TokenType::From => "'from'", TokenType::Global => "'global'", TokenType::If => "'if'",
        TokenType::Import => "'import'", TokenType::In => "'in'", TokenType::Is => "'is'",
        TokenType::Lambda => "'lambda'", TokenType::Nonlocal => "'nonlocal'", TokenType::Not => "'not'",
        TokenType::Or => "'or'", TokenType::Pass => "'pass'", TokenType::Raise => "'raise'",
        TokenType::Return => "'return'", TokenType::Try => "'try'", TokenType::While => "'while'",
        TokenType::With => "'with'", TokenType::Yield => "'yield'",

        // Soft keywords
        TokenType::Case => "'case'", TokenType::Match => "'match'", 
        TokenType::Type => "'type'", TokenType::Underscore => "'_'",

        // Operators (3-char)
        TokenType::DoubleStarEqual => "'**='", TokenType::DoubleSlashEqual => "'//='", 
        TokenType::LeftShiftEqual => "'<<='", TokenType::RightShiftEqual => "'>>='",

        // Operators (2-char)
        TokenType::NotEqual => "'!='", TokenType::PercentEqual => "'%='", TokenType::AmperEqual => "'&='", 
        TokenType::DoubleStar => "'**'", TokenType::StarEqual => "'*='", TokenType::PlusEqual => "'+='", 
        TokenType::MinEqual => "'-='", TokenType::Rarrow => "'->'", TokenType::Ellipsis => "'...'", 
        TokenType::DoubleSlash => "'//'", TokenType::SlashEqual => "'/='", TokenType::ColonEqual => "':='", 
        TokenType::LeftShift => "'<<'", TokenType::LessEqual => "'<='", TokenType::EqEqual => "'=='", 
        TokenType::GreaterEqual => "'>='", TokenType::RightShift => "'>>'", TokenType::AtEqual => "'@='", 
        TokenType::CircumflexEqual => "'^='", TokenType::VbarEqual => "'|='",

        // Operators (1-char)
        TokenType::Exclamation => "'!'", TokenType::Percent => "'%'", TokenType::Amper => "'&'", 
        TokenType::Star => "'*'", TokenType::Plus => "'+'", TokenType::Minus => "'-'", 
        TokenType::Dot => "'.'", TokenType::Slash => "'/'", TokenType::Less => "'<'", 
        TokenType::Equal => "'='", TokenType::Greater => "'>'", TokenType::At => "'@'", 
        TokenType::Circumflex => "'^'", TokenType::Vbar => "'|'", TokenType::Tilde => "'~'", 
        TokenType::Comma => "','", TokenType::Colon => "':'", TokenType::Semi => "';'",

        // Delimiters
        TokenType::Lpar => "'('", TokenType::Rpar => "')'", 
        TokenType::Lsqb => "'['", TokenType::Rsqb => "']'", 
        TokenType::Lbrace => "'{'", TokenType::Rbrace => "'}'",

        // Literals
        TokenType::Name => "identifier",
        TokenType::Float => "float",
        TokenType::Int => "integer",
        TokenType::String => "string",
        TokenType::Bytes => "bytes literal",

        // F-string
        TokenType::FstringStart => "f-string start", 
        TokenType::FstringMiddle => "f-string content", 
        TokenType::FstringEnd => "f-string end",

        // Whitespace and structure
        TokenType::Comment => "comment", 
        TokenType::Newline | TokenType::Nl => "newline", 
        TokenType::Indent => "indentation", 
        TokenType::Dedent => "dedentation", 
        TokenType::Endmarker => "EOF",
    }
}
