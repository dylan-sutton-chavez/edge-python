use crate::s;

use super::Parser;
use super::types::{OpCode, Value, MAX_EXPR_DEPTH, Instruction};

use super::types::{parse_string, parse_bytes_literal};
use crate::modules::lexer::{Token, TokenType};

use alloc::{string::ToString, vec::Vec, string::String};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /* Entry: Pratt parse, then optional ternary. Recursion is bounded inside `expr_bp`. */
    pub(super) fn expr(&mut self) {
        self.saw_newline = false;
        let val_start = self.chunk.instructions.len();
        self.expr_bp(0);
        self.ternary_tail(val_start);
    }

    /* Ternary: value was emitted first (single-pass), so reorder `[value][cond]` into `[cond][JumpIfFalse][value][Jump][else]` for CPython evaluation order. */
    pub(super) fn ternary_tail(&mut self, val_start: usize) {
        if self.saw_newline || !matches!(self.peek(), Some(TokenType::If)) { return; }
        self.advance();
        let cond_start = self.chunk.instructions.len();
        self.expr_bp(0);

        let cond_ins: Vec<Instruction> = self.chunk.instructions.drain(cond_start..).collect();
        let val_ins: Vec<Instruction> = self.chunk.instructions.drain(val_start..).collect();
        let calls_from = self.chunk.call_byte_pos.partition_point(|&(ip, _)| (ip as usize) < val_start);
        let cond_delta = val_start as i64 - cond_start as i64;

        self.push_shifted(cond_ins, cond_delta);
        let jf = self.emit_jump(OpCode::JumpIfFalse);
        let val_delta = self.chunk.instructions.len() as i64 - val_start as i64;
        self.push_shifted(val_ins, val_delta);

        // Remap recorded call ips; re-sort so resolve_call's binary search holds.
        for e in &mut self.chunk.call_byte_pos[calls_from..] {
            let d = if (e.0 as usize) < cond_start { val_delta } else { cond_delta };
            e.0 = (e.0 as i64 + d) as u32;
        }
        self.chunk.call_byte_pos[calls_from..].sort_unstable_by_key(|&(ip, _)| ip);

        let jmp = self.emit_jump(OpCode::Jump);
        self.patch(jf);
        self.eat(TokenType::Else);
        // Recurse so a chained conditional in the else-branch parses (right-associative).
        self.expr();
        self.patch(jmp);
    }

    /* Re-append drained instructions, shifting internal jump targets by `delta`. */
    fn push_shifted(&mut self, ins: Vec<Instruction>, delta: i64) {
        for i in ins {
            let operand = if matches!(i.opcode, OpCode::Jump | OpCode::JumpIfFalse | OpCode::JumpIfFalseOrPop | OpCode::JumpIfTrueOrPop | OpCode::ForIter) {
                (i.operand as i64 + delta) as u16
            } else {
                i.operand
            };
            self.chunk.instructions.push(Instruction { opcode: i.opcode, operand });
        }
    }

    pub(super) fn expr_tails(&mut self) {
        self.postfix_tail();
        self.infix_bp(0);
    }

    /* Pratt parser: unary prefix then infix loop via `binding_power` table. Bounds every recursive descent (prefix `-`/`+`/`~`/`await`/`not`, right-associative `**`, infix right operands) so deep chains raise instead of overflowing the native/WASM stack. */
    pub(super) fn expr_bp(&mut self, min_bp: u8) {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            self.error("expression too deeply nested");
            return;
        }
        match self.peek() {
            Some(TokenType::Not) => {
                self.advance();
                self.expr_bp(5);
                self.chunk.emit(OpCode::Not, 0);
            }
            _ => self.parse_unary(),
        }
        self.infix_bp(min_bp);
        self.expr_depth -= 1;
    }

    pub(super) fn infix_bp(&mut self, min_bp: u8) {
        while let Some(tok) = self.peek() {
            if tok == TokenType::Is {
                if 7 < min_bp { break; }
                self.advance();
                if self.eat_if(TokenType::Not) {
                    self.expr_bp(8);
                    self.chunk.emit(OpCode::IsNot, 0);
                } else {
                    self.expr_bp(8);
                    self.chunk.emit(OpCode::Is, 0);
                }
                continue;
            }

            if tok == TokenType::Not {
                if 7 < min_bp { break; }
                self.advance();
                self.eat(TokenType::In);
                self.expr_bp(8);
                self.chunk.emit(OpCode::NotIn, 0);
                continue;
            }

            let Some((l_bp, r_bp, op)) = Self::binding_power(&tok) else { break };
            if l_bp < min_bp { break; }
            self.advance();

            if matches!(op, OpCode::And | OpCode::Or) {
                let jump_op = if op == OpCode::And { OpCode::JumpIfFalseOrPop } else { OpCode::JumpIfTrueOrPop };
                let jmp = self.emit_jump(jump_op);
                self.expr_bp(r_bp);
                self.patch(jmp);
                continue;
            }

            self.expr_bp(r_bp);

            if matches!(op, OpCode::Eq | OpCode::NotEq | OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq)
                && let Some(next_tok) = self.peek()
                && matches!(next_tok, TokenType::Less | TokenType::Greater | TokenType::LessEqual | TokenType::GreaterEqual | TokenType::EqEqual | TokenType::NotEqual)
            {
                let ver = self.increment_version(super::SSA_TMP_CMP);
                let tmp = self.push_ssa_name(super::SSA_TMP_CMP, ver);
                self.chunk.emit(OpCode::StoreName, tmp);
                self.chunk.emit(OpCode::LoadName, tmp);
                self.chunk.emit(op, 0);
                let jmp = self.emit_jump(OpCode::JumpIfFalseOrPop);
                self.chunk.emit(OpCode::LoadName, tmp);
                self.infix_bp(min_bp);
                self.patch(jmp);
                return;
            }

            self.chunk.emit(op, 0);
        }
    }

    pub(super) fn binding_power(tok: &TokenType) -> Option<(u8, u8, OpCode)> {
        match tok {
            TokenType::Or => Some((1, 2, OpCode::Or)),
            TokenType::And => Some((3, 4, OpCode::And)),
            TokenType::EqEqual => Some((7, 8, OpCode::Eq)),
            TokenType::NotEqual => Some((7, 8, OpCode::NotEq)),
            TokenType::Less => Some((7, 8, OpCode::Lt)),
            TokenType::Greater => Some((7, 8, OpCode::Gt)),
            TokenType::LessEqual => Some((7, 8, OpCode::LtEq)),
            TokenType::GreaterEqual => Some((7, 8, OpCode::GtEq)),
            TokenType::In => Some((7, 8, OpCode::In)),
            TokenType::Vbar => Some((9, 10, OpCode::BitOr)),
            TokenType::Circumflex => Some((11, 12, OpCode::BitXor)),
            TokenType::Amper => Some((13, 14, OpCode::BitAnd)),
            TokenType::LeftShift => Some((15, 16, OpCode::Shl)),
            TokenType::RightShift => Some((15, 16, OpCode::Shr)),
            TokenType::Plus => Some((17, 18, OpCode::Add)),
            TokenType::Minus => Some((17, 18, OpCode::Sub)),
            TokenType::Star => Some((19, 20, OpCode::Mul)),
            TokenType::Slash => Some((19, 20, OpCode::Div)),
            TokenType::Percent => Some((19, 20, OpCode::Mod)),
            TokenType::DoubleSlash => Some((19, 20, OpCode::FloorDiv)),
            TokenType::DoubleStar => Some((22, 21, OpCode::Pow)),
            _ => None,
        }
    }

    pub(super) fn parse_unary(&mut self) {
        match self.peek() {
            Some(TokenType::Minus) => {
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::Minus, 0);
            }
            Some(TokenType::Plus) => {
                // Unary plus calls `__pos__` and coerces bool to int.
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::Pos, 0);
            }
            Some(TokenType::Tilde) => {
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::BitNot, 0);
            }
            Some(TokenType::Await) => {
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::Await, 0);
            }
            _ => self.parse_atom()
        }
    }

    /* Atoms: literals, names, numbers, strings, f-strings, containers. */
    pub(super) fn parse_atom(&mut self) {
        let errs_before = self.errors.len();
        let t = self.advance();
        match t.kind {
            TokenType::Name => self.name(t),
            TokenType::String => {
                let mut s = parse_string(self.lexeme(&t));
                while matches!(self.peek(), Some(TokenType::String)) {
                    let t = self.advance();
                    s.push_str(&parse_string(self.lexeme(&t)));
                }
                self.emit_const(Value::Str(s));
            }
            TokenType::Bytes => {
                // Adjacent bytes literals concat; mixing with str surfaces a diagnostic.
                let mut buf = parse_bytes_literal(self.lexeme(&t));
                while matches!(self.peek(), Some(TokenType::Bytes)) {
                    let t = self.advance();
                    buf.extend_from_slice(&parse_bytes_literal(self.lexeme(&t)));
                }
                self.emit_const(Value::Bytes(buf));
            }
            TokenType::Int | TokenType::Float => {
                self.parse_number(self.lexeme(&t), t.kind);
            }
            TokenType::True => self.chunk.emit(OpCode::LoadTrue, 0),
            TokenType::False => self.chunk.emit(OpCode::LoadFalse, 0),
            TokenType::None => self.chunk.emit(OpCode::LoadNone, 0),
            TokenType::Ellipsis => self.chunk.emit(OpCode::LoadEllipsis, 0),
            TokenType::FstringStart => self.fstring(t.start, t.end),
            TokenType::Lbrace => self.brace_literal(),
            TokenType::Lsqb => self.list_literal(),
            TokenType::Lpar => {
                if matches!(self.peek(), Some(TokenType::Rpar)) {
                    self.advance();
                    self.chunk.emit(OpCode::BuildTuple, 0);
                } else {
                    let elem_start = self.chunk.instructions.len();
                    self.expr();
                    if self.maybe_comprehension(elem_start, OpCode::BuildList, OpCode::ListAppend) {
                        self.advance();
                    } else if self.eat_if(TokenType::Comma) {
                        let mut count = 1u16;
                        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
                            self.expr();
                            count += 1;
                            if !self.eat_if(TokenType::Comma) { break; }
                        }
                        self.eat(TokenType::Rpar);
                        self.chunk.emit(OpCode::BuildTuple, count);
                    } else {
                        self.eat(TokenType::Rpar);
                    }
                }
            }
            // `yield` / `yield from` as an expression value (keyword already consumed).
            TokenType::Yield => self.emit_yield(),
            TokenType::Lambda => self.parse_lambda(),
            // Caret at consumed token; skip if `advance()` already reported the error.
            _ => {
                if self.errors.len() == errs_before {
                    self.error_at(t.start, t.end, "expected expression");
                }
            }
        }
        self.postfix_tail();
    }

    /* Name: assignment, walrus `:=`, call, or plain load. */
    pub(super) fn name(&mut self, t: Token) {
        let name = self.lexeme(&t).to_string();
        match self.peek() {
            // In f-string context, `=` is the debug marker `f"{x=}"`, not assignment.
            Some(TokenType::Equal) if !self.in_fstring_expr => {
                self.assign(name.clone());
                self.emit_load_ssa(name);
            }
            Some(TokenType::ColonEqual) => {
                self.advance();
                self.expr();
                if self.globals_decl.contains(&name) {
                    // Walrus must honor an enclosing `global` declaration.
                    let i = self.chunk.push_name(&name);
                    self.chunk.emit(OpCode::StoreGlobal, i);
                    self.chunk.emit(OpCode::LoadGlobal, i);
                } else {
                    let i = self.emit_store_new(&name);
                    self.chunk.emit(OpCode::LoadName, i);
                }
            }
            Some(TokenType::Lpar) => {
                // A void call (`print(...)`) in value position must still leave a value; materialise its None.
                if !self.call(name) { self.emit_const(Value::None); }
            }
            _ => self.emit_load_ssa(name),
        }
        self.postfix_tail();
    }

    fn parse_int_prefix(s: &str) -> (&str, u32) {
        if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) { (h, 16) }
        else if let Some(o) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) { (o, 8) }
        else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) { (b, 2) }
        else { (s, 10) }
    }

    pub(super) fn parse_number(&mut self, raw: &str, kind: TokenType) {
        let s = raw.replace('_', "");
        if kind == TokenType::Float {
            // A malformed float (e.g. empty exponent `1e`) is a syntax error, not 0.0.
            match s.parse() {
                Ok(f) => self.emit_const(Value::Float(f)),
                Err(_) => self.error("invalid float literal"),
            }
            return;
        }
        let (digits, base) = Self::parse_int_prefix(&s);
        // No leading zeros; all-zero runs still valid.
        if base == 10 && digits.len() > 1 && digits.starts_with('0') && digits.bytes().any(|b| b != b'0') {
            self.error("leading zeros in decimal integer literals are not permitted");
            return;
        }
        let parsed_i64 = if base == 10 {
            digits.parse::<i64>().ok()
        } else {
            i64::from_str_radix(digits, base).ok()
        };
        if let Some(v) = parsed_i64 {
            self.emit_const(Value::Int(v));
            return;
        }
        // Doesn't fit in i64, try i128 for the wide-int path.
        let parsed_i128 = if base == 10 {
            digits.parse::<i128>().ok()
        } else {
            i128::from_str_radix(digits, base).ok()
        };
        match parsed_i128 {
            Some(v) => self.emit_const(Value::LongInt(v)),
            None => self.error("integer literal too large to represent (max ±2^127)"),
        }
    }

    /* Subscript after `[`: plain index or `a:b:c` slice with None defaults. Eats the closing `]`; emits BuildSlice for slices. Returns true when a slice was built. */
    pub(super) fn parse_subscript(&mut self) -> bool {
        if matches!(self.peek(), Some(TokenType::Colon)) {
            self.chunk.emit(OpCode::LoadNone, 0);
        } else {
            self.expr();
        }
        if self.eat_if(TokenType::Colon) {
            let mut parts = 2u16;
            if matches!(self.peek(), Some(TokenType::Colon | TokenType::Rsqb)) {
                self.chunk.emit(OpCode::LoadNone, 0);
            } else {
                self.expr();
            }
            if self.eat_if(TokenType::Colon) {
                parts = 3;
                if matches!(self.peek(), Some(TokenType::Rsqb)) {
                    self.chunk.emit(OpCode::LoadNone, 0);
                } else {
                    self.expr();
                }
            }
            self.eat(TokenType::Rsqb);
            self.chunk.emit(OpCode::BuildSlice, parts);
            true
        } else {
            self.eat(TokenType::Rsqb);
            false
        }
    }

    /* Postfix trailers: `.attr`, [i], [s:e], (args), chained. A trailer must start on the same line, so a statement boundary ends the chain (else `x = []` ⏎ `[i]` parses as `[][i]`). */
    pub(super) fn postfix_tail(&mut self) {
        loop {
            match self.peek_same_line() {
                Some(TokenType::Lsqb) => {
                    self.advance();
                    self.parse_subscript();
                    // Subscript assignment: StoreItem; for slices runtime replaces the range.
                    if matches!(self.peek(), Some(TokenType::Equal)) {
                        self.advance();
                        self.expr();
                        self.chunk.emit(OpCode::StoreItem, 0);
                        self.chunk.emit(OpCode::LoadNone, 0);
                        return;
                    }
                    self.chunk.emit(OpCode::GetItem, 0);
                }
                Some(TokenType::Dot) => {
                    self.advance();
                    let t = self.advance();
                    let (start, end) = (t.start, t.end);
                    let idx = self.chunk.push_name(&self.source[start..end]);
                    // LoadAttr adjacent to Call lets `fuse_method_calls` collapse them.
                    self.chunk.emit(OpCode::LoadAttr, idx);
                }
                Some(TokenType::Lpar) => {
                    // Call after any trailer.
                    let call_pos = self.last_end as u32;
                    let is_method = matches!(self.chunk.instructions.last().map(|i| i.opcode), Some(OpCode::LoadAttr));
                    self.advance();
                    // Skip only zero-arg methods so LoadAttr+Call(0) fusion survives.
                    let empty = matches!(self.peek(), Some(TokenType::Rpar));
                    if !(is_method && empty) { self.chunk.emit(OpCode::BeginArgs, 0); }
                    let (pos, kw) = self.parse_args_body();
                    self.chunk.emit(OpCode::Call, super::pack_call(pos, kw));
                    self.chunk.record_call_pos(call_pos);
                }
                _ => break
            }
        }
    }

    /* lambda: fresh chunk, compiles body to Return, emits MakeFunction. */
    pub(super) fn parse_lambda(&mut self) {
        let mut params = Vec::new();
        let mut defaults = 0u16;
        if !matches!(self.peek(), Some(TokenType::Colon)) {
            loop {
                // Match parse_params prefix detection so *args/**kw names align with ParamKind.
                let prefix = if self.eat_if(TokenType::DoubleStar) { "**" }
                    else if self.eat_if(TokenType::Star) { "*"  }
                    else { "" };
                let nm = self.advance_text();
                params.push(if prefix.is_empty() { nm } else { s!(str prefix, str &nm) });
                if prefix.is_empty() && self.eat_if(TokenType::Equal) {
                    self.expr();
                    defaults += 1;
                    // Trailing `=` marks this param as carrying a default value.
                    if let Some(last) = params.last_mut() { last.push('='); }
                }
                if !self.eat_if(TokenType::Comma) {
                    break;
                }
            }
        }
        self.eat(TokenType::Colon);

        let outer_versions = self.ssa_versions.clone();

        let body = self.with_fresh_chunk(|s| {
            s.ssa_versions = outer_versions;
            // Base name shadows the enclosing scope; prefix/`=` marker must be stripped.
            for p in &params { s.ssa_versions.insert(super::types::param_base_name(p).to_string(), 0); }
            s.expr();
            s.chunk.emit(OpCode::ReturnValue, 0);
        });

        let param_slots: crate::util::fx::FxHashSet<String> = params.iter().map(|p| s!(str super::types::param_base_name(p), "_0")).collect();
        for name in &body.names {
            if !param_slots.contains(name.as_str()) {
                self.chunk.push_name(name);
            }
        }

        let fi = self.chunk.functions.len() as u16;
        self.chunk.functions.push((params, body, defaults, u16::MAX));
        self.chunk.emit(OpCode::MakeFunction, fi);
    }
}
