use crate::s;
use crate::util::hash::FxHashMap as HashMap;
use crate::value::ExternFn;

use alloc::{string::{String, ToString}, vec, vec::Vec};

pub(crate) const MAX_EXPR_DEPTH: usize = 200;
pub(crate) const MAX_INSTRUCTIONS: usize = 65_535;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)] // <256 variants; guarantees a 1-byte tag for stable bytecode / transmute / jump-table dispatch
pub enum OpCode {
    LoadConst, LoadName, StoreName, Call, PopTop, ReturnValue, BuildString, CallPrint, CallLen, 
    FormatValue, CallAbs, Minus, CallStr, CallInt, CallRange, Phi, CallChr, CallType, MakeFunction, 
    Add, Sub, Mul, Div, Eq, CallFloat, CallBool, CallRound, CallMin, CallMax, CallSum, CallSorted, 
    CallEnumerate, CallZip, CallList, CallTuple, CallDict, CallIsInstance, CallSet, CallInput, 
    CallOrd, BuildDict, BuildList, NotEq, Lt, Gt, LtEq, GtEq, And, Or, Not, JumpIfFalse, Jump, 
    GetIter, ForIter, GetItem, Mod, Pow, FloorDiv, LoadTrue, LoadFalse, LoadNone, LoadAttr, StoreAttr, 
    BuildSlice, MakeClass, SetupExcept, PopExcept, Raise, BitAnd, BitOr, BitXor,
    BitNot, Shl, Shr, In, NotIn, Is, IsNot, UnpackSequence, BuildTuple, WithEnter, WithExit, Yield,
    /* Finally/with block stack: setup pushes a cleanup frame, BeginFinally marks a normal entry, EndFinally resumes the exit. */
    SetupFinally, BeginFinally, EndFinally,
    /* break/continue across N finally/with blocks before its jump; operand is N. */
    UnwindFinally,
    Del, Assert, Global, Nonlocal, UnpackArgs, ListAppend, SetAdd, MapAdd, BuildSet, RaiseFrom,
    UnpackEx, LoadEllipsis, Await, MakeCoroutine, StoreItem, Dup2,
    JumpIfFalseOrPop, JumpIfTrueOrPop, Dup, CallMethod, CallMethodArgs, CallAll, CallAny, CallBin,
    CallOct, CallHex, CallDivmod, CallPow, CallRepr, CallReversed, CallCallable, CallId, CallHash,
    PopIter, DelItem, DelAttr, CallExtern,
    /* Pushes HeapObj::Extern; operand indexes extern_table. Used by native `import X`. */
    LoadExtern,
    /* Builds HeapObj::Module from stack: name + operand (attr_name, attr_value) pairs. */
    BuildModule,
    /* Constant-time lookup of chunk.imports[operand] from `vm.module_table`. */
    LoadModule,
    /* Read/write a `global`-declared name from/to `self.globals`; operand indexes the bare name in `chunk.names`. */
    LoadGlobal, StoreGlobal,
    /* Literal unpacking: pop a source value and merge it into the container left below it on the stack. `{**m}` / `{*s}` / `[*it]`. */
    DictUpdate, SetUpdate, ListExtend,
    /* Pop a value, push whether it matches a sequence pattern (list/tuple; str/bytes excluded). */
    MatchSeq,
    /* `name += rhs`: list+list extends in place (so aliases observe it, matching Python's __iadd__); every other type behaves as Add. */
    InPlaceAdd,
    /* Unary plus: calls `__pos__`, coerces bool to int, else identity on numbers. */
    Pos,
    /* Pushes the value `yield from` produces: the exhausted subiterator's return / StopIteration value. */
    LoadYieldFrom,
    // Augmented set bitwise `|=` `&=` `^=`: mutate the left set in place.
    InPlaceBitOr, InPlaceBitAnd, InPlaceBitXor,
    // Push a fresh spread-delta frame.
    BeginArgs,
    /* Consume the staged `__exit__` result; truthy suppresses a pending re-raise. */
    WithJudge,
    // Swap the top two stack values.
    Swap,
    // Lift the third stack value to the top.
    Rot3,
}

// Python builtin name -> (specialised OpCode, `leaves_value_on_stack`).
pub(super) fn builtin(name: &str) -> Option<(OpCode, bool)> {
    match name {
        "len" => Some((OpCode::CallLen, true)),
        "abs" => Some((OpCode::CallAbs, true)),
        "str" => Some((OpCode::CallStr, true)),
        "int" => Some((OpCode::CallInt, true)),
        "type" => Some((OpCode::CallType, true)),
        "float" => Some((OpCode::CallFloat, true)),
        "bool" => Some((OpCode::CallBool, true)),
        "round" => Some((OpCode::CallRound, true)),
        "sum" => Some((OpCode::CallSum, true)),
        // dict/min/max/enumerate and sorted need the keyword-aware path in `call()`, not this table.
        "zip" => Some((OpCode::CallZip, true)),
        "list" => Some((OpCode::CallList, true)),
        "tuple" => Some((OpCode::CallTuple, true)),
        "set" => Some((OpCode::CallSet, true)),
        "input" => Some((OpCode::CallInput, true)),
        "isinstance" => Some((OpCode::CallIsInstance, true)),
        "chr" => Some((OpCode::CallChr, true)),
        "ord" => Some((OpCode::CallOrd, true)),
        "all" => Some((OpCode::CallAll, true)),
        "any" => Some((OpCode::CallAny, true)),
        "bin" => Some((OpCode::CallBin, true)),
        "oct" => Some((OpCode::CallOct, true)),
        "hex" => Some((OpCode::CallHex, true)),
        "divmod" => Some((OpCode::CallDivmod, true)),
        "pow" => Some((OpCode::CallPow, true)),
        "repr" => Some((OpCode::CallRepr, true)),
        "reversed" => Some((OpCode::CallReversed, true)),
        "callable" => Some((OpCode::CallCallable, true)),
        "id" => Some((OpCode::CallId, true)),
        "hash" => Some((OpCode::CallHash, true)),
        _ => None,
    }
}

// Constant literals stored in the bytecode constants pool.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Bytes(alloc::vec::Vec<u8>),
    Int(i64),
    LongInt(i128), // Wide integer literal: value outside ±2^47 but inside ±2^127. Materialised as `HeapObj::LongInt` at constant-pool construction.
    Float(f64),
    Bool(bool),
    None,
}

// One bytecode instruction: opcode + 16-bit operand.
#[derive(Debug, Clone, Copy)]
pub struct Instruction {
    pub opcode: OpCode,
    pub operand: u16,
}

/* Parse-time import entry. VM dedupes by spec at `run()` start, runs Code modules once; LoadModule is then O(1). Native skips execution; Module Val built from bindings. */
#[derive(Clone)]
pub struct ImportEntry {
    pub spec: alloc::string::String,
    pub kind: ImportKind,
}

/* Synthesised class spec: name + method externs; init.rs builds a HeapObj::Class from this. */
#[derive(Clone)]
pub struct NativeClassEntry {
    pub name: String,
    pub methods: Vec<crate::value::ExternFn>,
}

#[derive(Clone)]
pub enum ImportKind {
    Code(alloc::rc::Rc<SSAChunk>),
    Native { funcs: Vec<crate::value::ExternFn>, classes: Vec<NativeClassEntry>, consts: Vec<crate::value::ExternFn> },
}

// SSA chunk: instructions, constant/name pools, Phi metadata, nested functions/classes.
#[derive(Default, Clone)]
pub struct SSAChunk {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub names: Vec<String>,
    pub functions: Vec<(Vec<String>, SSAChunk, u16, u16)>,
    pub phi_sources: Vec<(u16, u16)>,
    pub classes: Vec<SSAChunk>,
    pub is_pure: bool,
    pub is_generator: bool,
    pub overflow: bool,
    pub prev_slots: Vec<Option<u16>>,
    pub alias_groups: Vec<Vec<u16>>,
    pub phi_map: Vec<usize>,
    pub nonlocals: Vec<String>,
    pub(super) name_index: HashMap<String, u16>,
    /* stmt ip->byte_offset map; binary-searched on error path; hot dispatch never touches it. */
    pub stmt_pos: Vec<(u32, u32)>,
    /* Call ip->byte_offset map; finer than stmt_pos; traceback caret lands under the call. */
    pub call_byte_pos: Vec<(u32, u32)>,
    /* Source text; shared via Arc across sub-chunks. Empty for manually constructed chunks. */
    pub source: alloc::sync::Arc<alloc::string::String>,
    /* Display path for tracebacks; empty string suppresses the file: prefix. */
    pub path: alloc::sync::Arc<alloc::string::String>,
    /* Native bindings from `from <pkg> import`. CallExtern `operand=(idx<<8)|argc`; per-chunk. */
    pub extern_table: Vec<ExternFn>,
    pub(crate) extern_index: HashMap<String, u16>,
    /* Chunk's import list; LoadModule operands index here; each spec becomes one Module Val at init. */
    pub imports: Vec<ImportEntry>,
}

impl SSAChunk {
    /* Binary-searches stmt_pos to map ip->byte offset; statement-level precision. */
    pub fn resolve(&self, ip: u32) -> Option<u32> {
        let i = self.stmt_pos.partition_point(|&(s, _)| s <= ip).checked_sub(1)?;
        Some(self.stmt_pos[i].1)
    }

    /* Finer than `resolve()`: returns call-site byte offset or None (caller falls back to resolve). */
    pub fn resolve_call(&self, ip: u32) -> Option<u32> {
        let i = self.call_byte_pos.partition_point(|&(s, _)| s < ip);
        let (recorded_ip, byte) = *self.call_byte_pos.get(i)?;
        if recorded_ip == ip { Some(byte) } else { None }
    }

    pub(super) fn emit(&mut self, op: OpCode, operand: u16) {
        // Overflow: set flag for post-parse diagnostic rather than panic.
        if self.instructions.len() >= MAX_INSTRUCTIONS {
            self.overflow = true;
            return;
        }
        self.instructions.push(Instruction { opcode: op, operand });
    }

    /* Truncate instructions and drop call positions past the cut, keeps `call_byte_pos` sorted. */
    pub(super) fn truncate_to(&mut self, start: usize) {
        self.instructions.truncate(start);
        self.call_byte_pos.retain(|&(ip, _)| (ip as usize) < start);
    }

    /* Records (ip, `byte_pos`) for the last emitted call so traceback caret lands on it. */
    pub(super) fn record_call_pos(&mut self, byte_pos: u32) {
        if self.instructions.is_empty() { return; }
        let ip = (self.instructions.len() - 1) as u32;
        self.call_byte_pos.push((ip, byte_pos));
    }

    pub(super) fn push_const(&mut self, v: Value) -> u16 {
        if self.constants.len() >= u16::MAX as usize {
            return 0;
        }
        self.constants.push(v);
        (self.constants.len() - 1) as u16
    }

    pub(super) fn push_name(&mut self, n: &str) -> u16 {
        if let Some(&i) = self.name_index.get(n) { return i; }
        if self.names.len() >= u16::MAX as usize {
            return 0;
        }
        let i = self.names.len() as u16;
        self.names.push(n.to_string());
        self.name_index.insert(n.to_string(), i);
        i
    }

    /* Builds `prev_slots`, coalesces SSA versions to canonical root, rewrites operands, builds `phi_map`. */
    pub fn finalize_prev_slots(&mut self) {
        let n = self.names.len();

        // `prev_slots[i]`: slot of name i at version-1, if any.
        let mut ps: Vec<Option<u16>> = vec![None; n];
        for (i, name) in self.names.iter().enumerate() {
            if let Some(parsed) = SsaName::parse(name)
                && parsed.version > 0
            {
                let prev = s!(str parsed.bare, "_", int parsed.version as i64 - 1);
                if let Some(&j) = self.name_index.get(&prev) {
                    ps[i] = Some(j);
                }
            }
        }

        // Coalesce: walk each version chain to its root.
        let mut canonical: Vec<u16> = (0..n as u16).collect();
        for (i, item) in canonical.iter_mut().enumerate().take(n) {
            let mut root = i;
            while let Some(Some(p)) = ps.get(root) {
                let p = *p as usize;
                if p == root { break; }
                root = p;
            }
            *item = root as u16;
        }

        for ins in &mut self.instructions {
            match ins.opcode {
                OpCode::LoadName | OpCode::StoreName | OpCode::Del | OpCode::Phi => {
                    // Malformed input can leave an out-of-range operand; keep it as-is then.
                    if let Some(&c) = canonical.get(ins.operand as usize) { ins.operand = c; }
                }
                _ => {}
            }
        }
        for (a, b) in &mut self.phi_sources {
            if let Some(&c) = canonical.get(*a as usize) { *a = c; }
            if let Some(&c) = canonical.get(*b as usize) { *b = c; }
        }

        self.prev_slots = ps;
        self.alias_groups = (0..n).map(|i| vec![canonical[i]]).collect();

        for (_, body, _, _) in &mut self.functions {
            body.finalize_prev_slots();
        }
        for body in &mut self.classes {
            body.finalize_prev_slots();
        }

        let phi_count = self.instructions.iter().filter(|i| i.opcode == OpCode::Phi).count();
        if phi_count > 0 {
            self.phi_map = vec![0; self.instructions.len()];
            let mut phi_idx = 0;
            for (i, ins) in self.instructions.iter().enumerate() {
                if ins.opcode == OpCode::Phi {
                    self.phi_map[i] = phi_idx;
                    phi_idx += 1;
                }
            }
        }
    }
}

// SSA version snapshots for branch join; `then` is None until mid_block runs.
pub(crate) struct JoinNode {
    pub(super) backup: HashMap<String, u32>,
    pub(super) then: Option<HashMap<String, u32>>,
}

/* Synthetic SSA temps for multi-step desugarings. Leading `#` hides them from `globals()`/`locals()`; centralised so a typo becomes a compile error, not a misnamed slot. */
pub const SSA_TMP_CMP: &str = "#cmp";
pub const SSA_TMP_MATCH: &str = "#match";
pub const SSA_TMP_MATCH_ITEM: &str = "#match_item";

// Param name without `*`/`**`/`~` marker prefixes.
pub fn param_base_name(p: &str) -> &str {
    p.trim_start_matches(['*', '~']).trim_end_matches('=')
}

/* Parsed view of a `<bare>_<digits>` SSA-suffixed name, avoids re-inlining the rfind('_') + ascii-digit + parse dance at every call site. */
pub struct SsaName<'a> {
    pub bare: &'a str,
    pub version: u32,
}

impl<'a> SsaName<'a> {
    // Some when `name` matches `<bare>_<digits>`; None for synthetic temps and non-SSA names.
    pub fn parse(name: &'a str) -> Option<Self> {
        let pos = name.rfind('_')?;
        if pos + 1 >= name.len() { return None; }
        let suffix = &name[pos + 1..];
        if !suffix.bytes().all(|b| b.is_ascii_digit()) { return None; }
        let version = suffix.parse().ok()?;
        Some(Self { bare: &name[..pos], version })
    }

    // (bare, version) for any name, defaulting to (name, 0) when no SSA suffix is present.
    pub fn parse_or_bare(name: &'a str) -> (&'a str, u32) {
        Self::parse(name)
            .map(|s| (s.bare, s.version))
            .unwrap_or((name, 0))
    }
}

/* Strips `_<digits>` SSA suffix for user-facing diagnostics; returns input unchanged if absent. */
pub fn ssa_strip(name: &str) -> &str {
    SsaName::parse(name).map(|s| s.bare).unwrap_or(name)
}

/* Diagnostic with byte offsets; line/col computed at render time (UTF-8 safe). */
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub msg: String,
}

/* UAX#11 display width: 0=combining, 2=CJK/emoji, 1=other; keeps caret aligned in diagnostics. */
const fn char_width(c: char) -> usize {
    let cp = c as u32;
    if matches!(cp,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF
        | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7 | 0x0610..=0x061A
        | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8 | 0x06EA..=0x06ED | 0x0711 | 0x0730..=0x074A
        | 0x07A6..=0x07B0 | 0x07EB..=0x07F3 | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF | 0xE0100..=0xE01EF)
    {
        0
    } else if matches!(cp,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF | 0x20000..=0x3FFFD)
    {
        2
    } else {
        1
    }
}

#[inline]
fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

impl Diagnostic {
    /* Byte offset -> (line, col), 1-indexed; col counts display cells for wide-char alignment. */
    fn line_col(src: &str, byte: usize) -> (usize, usize) {
        let byte = byte.min(src.len());
        let line = src[..byte].matches('\n').count() + 1;
        let line_start = src[..byte].rfind('\n').map_or(0, |p| p + 1);
        let col = display_width(&src[line_start..byte]) + 1;
        (line, col)
    }
}

impl Diagnostic {
    /* rustc-style render: error+arrow+source line+caret; path defaults to `<input>`. */
    pub fn render(&self, src: &str, path: Option<&str>) -> alloc::string::String {
        let path = path.unwrap_or("<input>");
        let s_off = self.start.min(src.len());
        let e_off = self.end.min(src.len()).max(s_off);
        let (line_no, col) = Self::line_col(src, s_off);
        let line_start = src[..s_off].rfind('\n').map_or(0, |p| p + 1);
        let line_end = src[s_off..].find('\n').map_or(src.len(), |p| s_off + p);
        let line_txt = &src[line_start..line_end];
        let mark = display_width(&src[s_off..e_off]).max(1);
        let mut buf = itoa::Buffer::new();
        let pad_len = buf.format(line_no).len();
        let pad: String = " ".repeat(pad_len);
        let mut o = alloc::string::String::with_capacity(line_txt.len() + 96);
        o.push_str("error: "); o.push_str(&self.msg); o.push('\n');
        o.push_str(&pad); o.push_str(" --> ");
        o.push_str(path); o.push(':');
        o.push_str(buf.format(line_no)); o.push(':'); o.push_str(buf.format(col)); o.push('\n');
        o.push_str(&pad); o.push_str(" |\n");
        o.push_str(buf.format(line_no)); o.push_str(" | "); o.push_str(line_txt); o.push('\n');
        o.push_str(&pad); o.push_str(" | ");
        for _ in 1..col { o.push(' '); }
        for _ in 0..mark { o.push('^'); }
        o.push('\n');
        o
    }
}


/* Scan only the prefix chars before the opening quote; the body itself may legally contain 'r'/'R'. */
pub(super) fn has_raw_prefix(s: &str) -> bool {
    s.bytes()
        .take_while(|b| !matches!(b, b'"' | b'\''))
        .any(|b| matches!(b, b'r' | b'R'))
}

// Strip prefix + quotes and unescape (skipped for raw strings).
pub(super) fn parse_string(s: &str) -> String {
    let is_raw = has_raw_prefix(s);
    let s = s.trim_start_matches(|c: char| "bBrRuU".contains(c));
    let inner = if s.starts_with("\"\"\"") || s.starts_with("'''") {
        s.get(3..s.len().saturating_sub(3)).unwrap_or("")
    } else {
        s.get(1..s.len().saturating_sub(1)).unwrap_or("")
    };
    // Python normalizes source CR and CRLF to LF before building the literal.
    let owned;
    let inner: &str = if inner.contains('\r') {
        owned = inner.replace("\r\n", "\n").replace('\r', "\n");
        &owned
    } else { inner };
    if is_raw { inner.to_string() } else { unescape(inner) }
}

/* Parses b"..." to raw bytes: non-ASCII pass through; \xHH=single byte; \u/\U/\N rejected. */
pub(super) fn parse_bytes_literal(s: &str) -> alloc::vec::Vec<u8> {
    let bytes = s.as_bytes();
    let is_raw = has_raw_prefix(s);
    // Skip b/B/r/R prefix chars.
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b'b' | b'B' | b'r' | b'R') {
        i += 1;
    }
    // Strip triple or single quotes.
    let body: &[u8] = if bytes.len() >= i + 6
        && (bytes[i..i + 3] == *b"\"\"\"" || bytes[i..i + 3] == *b"'''")
    {
        &bytes[i + 3..bytes.len() - 3]
    } else {
        bytes.get(i + 1..bytes.len().saturating_sub(1)).unwrap_or(&[])
    };
    if is_raw { return body.to_vec(); }

    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(body.len());
    let mut j = 0;
    while j < body.len() {
        if body[j] != b'\\' { out.push(body[j]); j += 1; continue; }
        if j + 1 >= body.len() { out.push(b'\\'); break; }
        match body[j + 1] {
            b'n' => { out.push(b'\n'); j += 2; }
            b't' => { out.push(b'\t'); j += 2; }
            b'r' => { out.push(b'\r'); j += 2; }
            b'a' => { out.push(0x07); j += 2; }
            b'b' => { out.push(0x08); j += 2; }
            b'f' => { out.push(0x0C); j += 2; }
            b'v' => { out.push(0x0B); j += 2; }
            b'\\' => { out.push(b'\\'); j += 2; }
            b'\'' => { out.push(b'\''); j += 2; }
            b'"' => { out.push(b'"'); j += 2; }
            b'0' => { out.push(0); j += 2; }
            b'x' => {
                // \xHH: exactly two hex digits.
                if j + 3 < body.len() {
                    let hi = (body[j + 2] as char).to_digit(16);
                    let lo = (body[j + 3] as char).to_digit(16);
                    if let (Some(hi), Some(lo)) = (hi, lo) {
                        out.push((hi as u8) * 16 + lo as u8);
                        j += 4;
                        continue;
                    }
                }
                // Malformed \x: emit verbatim.
                out.push(b'\\'); out.push(b'x'); j += 2;
            }
            other => { out.push(b'\\'); out.push(other); j += 2; }
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' { push_escape(&mut out, &mut chars); } else { out.push(c); }
    }
    out
}

/* Decodes one backslash escape (cursor already past the `\`) into `out`; unknown escapes keep the backslash. */
pub(super) fn push_escape(out: &mut String, chars: &mut core::iter::Peekable<core::str::Chars>) {
    let take_hex = |chars: &mut core::iter::Peekable<core::str::Chars>, n: usize| -> char {
        let hex: String = chars.by_ref().take(n).collect();
        u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}')
    };
    match chars.next() {
        Some('n') => out.push('\n'),
        Some('t') => out.push('\t'),
        Some('r') => out.push('\r'),
        Some('a') => out.push('\u{07}'),
        Some('b') => out.push('\u{08}'),
        Some('f') => out.push('\u{0C}'),
        Some('v') => out.push('\u{0B}'),
        Some('\\') => out.push('\\'),
        Some('\'') => out.push('\''),
        Some('"') => out.push('"'),
        Some('x') => out.push(take_hex(chars, 2)),
        Some('u') => out.push(take_hex(chars, 4)),
        Some('U') => out.push(take_hex(chars, 8)),
        // Octal: up to 3 digits.
        Some(c @ '0'..='7') => {
            let mut digits = String::from(c);
            while digits.len() < 3 && matches!(chars.peek(), Some('0'..='7')) {
                digits.push(chars.next().unwrap());
            }
            let code = u32::from_str_radix(&digits, 8).unwrap_or(0);
            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
        }
        Some(c) => { out.push('\\'); out.push(c); }
        None => out.push('\\'),
    }
}

// Builtin types registered as Type heap objects at VM init.
pub const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "str", "bytes", "bool", "list",
    "tuple", "dict", "set", "frozenset", "range", "slice", "type", "NoneType", "object",
    "Exception", "BaseException",
    "ValueError", "TypeError", "NameError", "KeyError",
    "IndexError", "AttributeError", "RuntimeError",
    "ZeroDivisionError", "OverflowError", "MemoryError",
    "RecursionError", "StopIteration", "NotImplementedError",
    "OSError", "IOError", "ImportError", "ModuleNotFoundError",
    "AssertionError", "ArithmeticError", "LookupError",
    "CancelledError", "TimeoutError", "SystemExit",
];
