use crate::s;

use super::Parser;
use super::types::builtin;
use super::types::{OpCode, Value, SSAChunk, Instruction};

use crate::lexer::{Token, TokenType};
use crate::util::hash::FxHashMap as HashMap;

use alloc::{string::{String, ToString}, vec::Vec};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /* `{}` is a dict/set literal or comprehension, always eat(Rbrace) to keep `bracket_stack` in sync. */
    pub(super) fn brace_literal(&mut self) {
        if matches!(self.peek(), Some(TokenType::Rbrace)) {
            self.advance();
            self.chunk.emit(OpCode::BuildDict, 0);
            return;
        }
        // `{**m, ...}`, leading mapping-unpack => dict built incrementally.
        if self.eat_if(TokenType::DoubleStar) {
            self.chunk.emit(OpCode::BuildDict, 0);
            self.expr();
            self.chunk.emit(OpCode::DictUpdate, 0);
            self.dict_tail(0, true);
            return;
        }
        // `{*s, ...}`, leading iterable-unpack => set built incrementally.
        if self.eat_if(TokenType::Star) {
            self.chunk.emit(OpCode::BuildSet, 0);
            self.expr();
            self.chunk.emit(OpCode::SetUpdate, 0);
            self.set_tail(0, true);
            return;
        }
        let key_start = self.chunk.instructions.len();
        self.expr();
        match self.peek() {
            Some(TokenType::Colon) => {
                self.advance();
                let val_start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::For)) {
                    let versions_before = self.ssa_versions.clone();
                    let val_ins: Vec<Instruction> = self.chunk.instructions.drain(val_start..).collect();
                    let key_ins: Vec<Instruction> = self.chunk.instructions.drain(key_start..).collect();
                    self.chunk.emit(OpCode::BuildDict, 0);
                    self.comprehension_loop(&[(key_start, key_ins), (val_start, val_ins)], OpCode::MapAdd, &versions_before);
                    self.eat(TokenType::Rbrace);
                } else {
                    // First pair already emitted, dict_tail consolidates if a later `**` appears.
                    self.dict_tail(1, false);
                }
            }
            _ => {
                if self.maybe_comprehension(key_start, OpCode::BuildSet, OpCode::SetAdd) {
                    self.eat(TokenType::Rbrace);
                } else {
                    // First element already emitted, set_tail consolidates if a later `*` appears.
                    self.set_tail(1, false);
                }
            }
        }
    }

    /* If `for` follows, lower [elem_start..] as a comprehension, true when consumed. */
    pub(super) fn maybe_comprehension(&mut self, elem_start: usize, build: OpCode, append: OpCode) -> bool {
        if !matches!(self.peek(), Some(TokenType::For)) { return false; }
        let versions_before = self.ssa_versions.clone();
        let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
        self.chunk.emit(build, 0);
        self.comprehension_loop(&[(elem_start, elem_ins)], append, &versions_before);
        true
    }

    /* `[]` is a list literal or list-comp, always eat(Rsqb) to keep `bracket_stack` in sync. */
    pub(super) fn list_literal(&mut self) {
        if matches!(self.peek(), Some(TokenType::Rsqb)) {
            self.advance();
            self.chunk.emit(OpCode::BuildList, 0);
            return;
        }
        // `[*it, ...]`, leading iterable-unpack => list built incrementally.
        if self.eat_if(TokenType::Star) {
            self.chunk.emit(OpCode::BuildList, 0);
            self.expr();
            self.chunk.emit(OpCode::ListExtend, 0);
            self.list_tail(0, true);
            return;
        }
        let elem_start = self.chunk.instructions.len();
        self.expr();
        if self.maybe_comprehension(elem_start, OpCode::BuildList, OpCode::ListAppend) {
            self.eat(TokenType::Rsqb);
        } else {
            // First element already emitted, list_tail consolidates if a later `*` appears.
            self.list_tail(1, false);
        }
    }

    /* Shared tail for `{}`/`[]` displays after the first element. `count` = loose elems on the stack, `incremental` = container already on the stack. First spread consolidates loose elems with `build count`, then merges use `update`/`add`. */
    #[allow(clippy::too_many_arguments)]
    fn container_tail(
        &mut self, mut count: u16, mut incremental: bool,
        close: TokenType, spread: TokenType,
        build: OpCode, update: OpCode, add: OpCode,
        elem: impl Fn(&mut Self),
    ) {
        while self.eat_if(TokenType::Comma) {
            if self.peek() == Some(close) { break; }
            if self.eat_if(spread) {
                if !incremental { self.chunk.emit(build, count); incremental = true; }
                self.expr();
                self.chunk.emit(update, 0);
            } else {
                elem(self);
                if incremental { self.chunk.emit(add, 0); } else { count += 1; }
            }
        }
        self.eat(close);
        if !incremental { self.chunk.emit(build, count); }
    }

    fn dict_tail(&mut self, pairs: u16, incremental: bool) {
        self.container_tail(pairs, incremental, TokenType::Rbrace, TokenType::DoubleStar,
            OpCode::BuildDict, OpCode::DictUpdate, OpCode::MapAdd,
            |s| { s.expr(); s.eat(TokenType::Colon); s.expr(); });
    }

    fn set_tail(&mut self, count: u16, incremental: bool) {
        self.container_tail(count, incremental, TokenType::Rbrace, TokenType::Star,
            OpCode::BuildSet, OpCode::SetUpdate, OpCode::SetAdd, |s| s.expr());
    }

    fn list_tail(&mut self, count: u16, incremental: bool) {
        self.container_tail(count, incremental, TokenType::Rsqb, TokenType::Star,
            OpCode::BuildList, OpCode::ListExtend, OpCode::ListAppend, |s| s.expr());
    }

    /* Shared comprehension scaffolding, parses the for/if clauses, returns (loop starts, ForIter patch sites, SSA remap for loop vars). */
    fn comp_header(&mut self, versions_before: &HashMap<String, u32>) -> (Vec<u16>, Vec<usize>, Vec<(u16, u16)>) {
        let mut loop_starts: Vec<u16> = Vec::new();
        let mut for_iters: Vec<usize> = Vec::new();
        let mut all_vars: Vec<String> = Vec::new();

        while self.eat_if(TokenType::For) {
            let mut vars: Vec<String> = Vec::new();
            loop {
                vars.push(self.advance_text());
                if !self.eat_if(TokenType::Comma) { break; }
                if matches!(self.peek(), Some(TokenType::In)) { break; }
            }

            self.eat(TokenType::In);
            self.expr_bp(1);
            self.chunk.emit(OpCode::GetIter, 0);

            let ls = self.chunk.instructions.len() as u16;
            let fi = self.emit_jump(OpCode::ForIter);

            if vars.len() == 1 {
                self.store_name(vars[0].clone());
            } else {
                self.emit_unpack_stores(&vars, None);
            }
            for v in &vars { all_vars.push(v.clone()); }

            while self.eat_if(TokenType::If) {
                self.expr_bp(1);
                self.chunk.emit(OpCode::JumpIfFalse, ls);
            }

            loop_starts.push(ls);
            for_iters.push(fi);
        }

        // Linear scan, size 1-5 beats HashMap and avoids monomorphizing for u16 keys.
        let mut var_map: Vec<(u16, u16)> = Vec::new();
        for var in &all_vars {
            let old_ver = versions_before.get(var).copied().unwrap_or(0);
            let new_ver = self.current_version(var);
            if old_ver == new_ver { continue; }
            let mut ob = [0u8; 128];
            let old_name = Self::ssa_name(var, old_ver, &mut ob);
            let Some(&old_slot) = self.chunk.name_index.get(old_name) else { continue };
            let mut nb = [0u8; 128];
            let new_slot = self.chunk.push_name(Self::ssa_name(var, new_ver, &mut nb));
            var_map.push((old_slot, new_slot));
        }
        (loop_starts, for_iters, var_map)
    }

    /* Re-emit captured element bodies inside the loop, remapping loop vars and shifting internal jumps. */
    fn replay_comp_bodies(&mut self, elem_bodies: &[(usize, Vec<Instruction>)], var_map: &[(u16, u16)]) {
        for (orig_base, body) in elem_bodies {
            // Body is relocated by this delta, internal jump targets (from `or`/`and`/membership) must shift with it.
            let delta = self.chunk.instructions.len() as i64 - *orig_base as i64;
            for ins in body {
                let operand = if matches!(ins.opcode, OpCode::LoadName | OpCode::StoreName) {
                    var_map.iter().find(|(k, _)| *k == ins.operand).map(|(_, v)| *v).unwrap_or(ins.operand)
                } else if matches!(ins.opcode, OpCode::Jump | OpCode::JumpIfFalse | OpCode::JumpIfFalseOrPop | OpCode::JumpIfTrueOrPop | OpCode::ForIter) {
                    (ins.operand as i64 + delta) as u16
                } else {
                    ins.operand
                };
                self.chunk.instructions.push(Instruction { opcode: ins.opcode, operand });
            }
        }
    }

    /* Emits for/if comprehension scaffolding, reinjects body with loop-bound SSA slots. */
    pub(super) fn comprehension_loop(&mut self, elem_bodies: &[(usize, Vec<Instruction>)], append_op: OpCode, versions_before: &HashMap<String, u32>) {
        let (loop_starts, for_iters, var_map) = self.comp_header(versions_before);
        self.replay_comp_bodies(elem_bodies, &var_map);
        self.chunk.emit(append_op, 0);

        for i in (0..for_iters.len()).rev() {
            self.chunk.emit(OpCode::Jump, loop_starts[i]);
            self.patch(for_iters[i]);
        }
    }

    /* `any(genexpr)` / `all(genexpr)`, same scaffolding but each element decides instead of appending, so evaluation stops at the first hit like Python. */
    pub(super) fn scan_comprehension(&mut self, elem_bodies: &[(usize, Vec<Instruction>)], find_true: bool, versions_before: &HashMap<String, u32>) {
        let (loop_starts, for_iters, var_map) = self.comp_header(versions_before);
        self.replay_comp_bodies(elem_bodies, &var_map);
        // `all` exits on falsy, invert so one JumpIfFalse serves both.
        if !find_true { self.chunk.emit(OpCode::Not, 0); }
        if let Some(&ls) = loop_starts.last() {
            self.chunk.emit(OpCode::JumpIfFalse, ls);
        }
        // Decided, unwind every active iterator before leaving the loops.
        for _ in &for_iters { self.chunk.emit(OpCode::PopIter, 0); }
        self.chunk.emit(if find_true { OpCode::LoadTrue } else { OpCode::LoadFalse }, 0);
        let done = self.emit_jump(OpCode::Jump);
        for i in (0..for_iters.len()).rev() {
            self.chunk.emit(OpCode::Jump, loop_starts[i]);
            self.patch(for_iters[i]);
        }
        self.chunk.emit(if find_true { OpCode::LoadFalse } else { OpCode::LoadTrue }, 0);
        self.patch(done);
    }

    /* f-string emits literal+expr parts until FstringEnd, returns the count, caller wraps in BuildString. `fs_start/fs_end` anchor unclosed-string errors. */
    pub(super) fn fstring(&mut self, fs_start: usize, fs_end: usize) -> u16 {
        let mut parts = 0u16;
        let mut got_end = false;
        // Raw f-strings (`rf"..."`) keep backslashes literal, plain ones decode escapes like a normal string.
        let is_raw = super::types::has_raw_prefix(&self.source[fs_start..fs_end]);
        if matches!(self.peek(), Some(TokenType::FstringEnd)) {
            self.advance();
            return 0;
        }
        loop {
            match self.peek() {
            Some(TokenType::FstringMiddle) => {
                let t = self.advance();
                let raw = self.lexeme(&t);
                let mut unescaped = String::with_capacity(raw.len());
                // Single pass so `{{` is seen in the raw text before any escape can produce a brace.
                let mut chars = raw.chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        '{' if chars.peek() == Some(&'{') => { chars.next(); unescaped.push('{'); }
                        '}' if chars.peek() == Some(&'}') => { chars.next(); unescaped.push('}'); }
                        '\\' if !is_raw => super::types::push_escape(&mut unescaped, &mut chars),
                        _ => unescaped.push(c),
                    }
                }
                self.emit_const(Value::Str(unescaped));
                parts += 1;
            }
                Some(TokenType::Lbrace) => {
                    self.advance();
                    // Capture span for `f"{expr=}"` debug prefix.
                    let expr_start_byte = self.tokens.peek().map(|t| t.start).unwrap_or(0);
                    let insn_start = self.chunk.instructions.len();
                    let saved_in_fstring = self.in_fstring_expr;
                    self.in_fstring_expr = true;
                    self.expr();
                    // Bare tuple in a replacement field, `f"{1,}"` builds (1,).
                    if matches!(self.peek(), Some(TokenType::Comma)) {
                        let n = self.tuple_rest(1, |s| matches!(s.peek(), Some(TokenType::Rbrace | TokenType::Colon | TokenType::Exclamation | TokenType::Equal) | None));
                        self.chunk.emit(OpCode::BuildTuple, n);
                    }
                    self.in_fstring_expr = saved_in_fstring;
                    let expr_end_byte = self.last_end;
                    /* FormatValue operand, bit0=has-spec, bits1-2=conversion (0=none,1=!r,2=!s,3=!a). */
                    let mut flags = 0u16;
                    // `=` debug emits "expr=" prefix, defaults to !r when no conv/spec given.
                    let mut debug_prefix: Option<String> = None;
                    if matches!(self.peek(), Some(TokenType::Equal)) {
                        self.advance();
                        let raw = &self.source[expr_start_byte..expr_end_byte];
                        debug_prefix = Some(s!(str raw, "="));
                    }
                    if matches!(self.peek(), Some(TokenType::Exclamation)) {
                        let bang = self.advance();
                        let conv_tok = self.advance();
                        let conv = self.lexeme(&conv_tok);
                        flags |= match conv {
                            "r" => 1 << 1,
                            "s" => 2 << 1,
                            "a" => 3 << 1,
                            _ => {
                                self.error_at(bang.start, conv_tok.end,
                                    "invalid f-string conversion (expected !r, !s, or !a)");
                                0
                            }
                        };
                    }
                    if debug_prefix.is_some() && (flags & 0b110) == 0 && !matches!(self.peek(), Some(TokenType::Colon)) {
                        flags |= 1 << 1; // default !r when `=` has no explicit conv/spec
                    }
                    // Drain expr bytecode, emit prefix const, re-emit expr so `stack=[prefix, value]`.
                    if let Some(prefix) = debug_prefix.take() {
                        let drained: Vec<Instruction> = self.chunk.instructions
                            .drain(insn_start..)
                            .collect();
                        self.emit_const(Value::Str(prefix));
                        parts += 1;
                        self.chunk.instructions.extend(drained);
                    }
                    if matches!(self.peek(), Some(TokenType::Colon)) {
                        let colon = self.advance();
                        let spec_start = colon.end;
                        loop {
                            match self.tokens.peek().map(|t| t.kind) {
                                Some(TokenType::Rbrace) | None => break,
                                _ => { self.tokens.next(); }
                            }
                        }
                        let spec_end = self.tokens.peek().map(|t| t.start).unwrap_or(spec_start);
                        let spec = self.source[spec_start..spec_end].to_string();
                        let idx = self.chunk.push_const(Value::Str(spec));
                        self.chunk.emit(OpCode::LoadConst, idx);
                        flags |= 1;
                    }
                    self.chunk.emit(OpCode::FormatValue, flags);
                    parts += 1;
                    if matches!(self.peek(), Some(TokenType::Rbrace)) {
                        self.advance();
                    }
                }
                Some(TokenType::FstringEnd) => {
                    self.advance();
                    got_end = true;
                    break;
                }
                _ => break
            }
        }
        if !got_end {
            self.error_at(fs_start, fs_end, "f-string was never closed");
        }
        parts
    }

    /* Dispatches call, print/range opcodes, imported natives (shadow builtins), builtins table, else LoadName+Call. */
    pub(super) fn call(&mut self, name: String) -> bool {
        let call_pos = self.last_end as u32;
        // A rebound builtin name must call the binding, not the fused opcode.
        if self.current_version(&name) > 0 || self.globals_decl.contains(&name) {
            let i = self.push_ssa_name(&name, self.current_version(&name));
            if self.globals_decl.contains(&name) {
                let gi = self.chunk.push_name(&name);
                self.chunk.emit(OpCode::LoadGlobal, gi);
            } else {
                self.chunk.emit(OpCode::LoadName, i);
            }
            self.chunk.emit(OpCode::BeginArgs, 0);
            let (pos, kw) = self.parse_args();
            self.chunk.emit(OpCode::Call, super::pack_call(pos, kw));
            self.chunk.record_call_pos(call_pos);
            return true;
        }
        if name == "print" {
            let (pos, kw) = self.parse_args();
            // Same packed layout as Call so the VM can split sep/end kwargs from positionals.
            self.chunk.emit(OpCode::CallPrint, super::pack_call(pos, kw));
            self.chunk.record_call_pos(call_pos);
            return false;
        }

        if name == "range" {
            self.call_range();
            return true;
        }

        // Imported natives shadow builtins, matching Python `from x import *` rebinding.
        if let Some(&extern_idx) = self.chunk.extern_index.get(&name) {
            let (pos, kw) = self.parse_args();
            // Operand packs extern_idx<<8 | kw<<4 | pos, same layout as Call.
            let encoded = (extern_idx << 8) | ((kw & 0xF) << 4) | (pos & 0xF);
            self.chunk.emit(OpCode::CallExtern, encoded);
            self.chunk.record_call_pos(call_pos);
            return true;
        }

        // dict()/min()/max()/enumerate() take keywords (`default=`/`key=`/`start=`), so keep positional and keyword counts distinct via the packed operand.
        if let Some(op) = match name.as_str() {
            "dict" => Some(OpCode::CallDict),
            "min" => Some(OpCode::CallMin),
            "max" => Some(OpCode::CallMax),
            "enumerate" => Some(OpCode::CallEnumerate),
            _ => None,
        } {
            let (pos, kw) = self.parse_args();
            self.chunk.emit(op, super::pack_call(pos, kw));
            self.chunk.record_call_pos(call_pos);
            return true;
        }

        // `any`/`all` over a genexpr lowers to a short-circuit scan, other shapes keep the fused opcode.
        if matches!(name.as_str(), "any" | "all") {
            let find_true = name == "any";
            self.advance();
            if !matches!(self.peek(), Some(TokenType::Rpar | TokenType::Star | TokenType::DoubleStar)) {
                let versions_before = self.ssa_versions.clone();
                let elem_start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::For)) {
                    let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
                    self.scan_comprehension(&[(elem_start, elem_ins)], find_true, &versions_before);
                    self.eat(TokenType::Rpar);
                    self.chunk.record_call_pos(call_pos);
                    return true;
                }
                let mut count = 1u16;
                while self.eat_if(TokenType::Comma) {
                    if matches!(self.peek(), Some(TokenType::Rpar)) { break; }
                    self.expr();
                    count = count.saturating_add(1);
                }
                self.eat(TokenType::Rpar);
                self.chunk.emit(if find_true { OpCode::CallAny } else { OpCode::CallAll }, count);
                self.chunk.record_call_pos(call_pos);
                return true;
            }
            let (pos, kw) = self.parse_args_body();
            self.chunk.emit(if find_true { OpCode::CallAny } else { OpCode::CallAll }, pos + kw);
            self.chunk.record_call_pos(call_pos);
            return true;
        }

        if let Some((op, leaves_value)) = builtin(name.as_str()) {
            let (pos, kw) = self.parse_args();
            self.chunk.emit(op, pos + kw);
            self.chunk.record_call_pos(call_pos);
            return leaves_value;
        }

        let i = self.push_ssa_name(&name, self.current_version(&name));
        self.chunk.emit(OpCode::LoadName, i);
        // Isolate this call's spread delta from any enclosing call.
        self.chunk.emit(OpCode::BeginArgs, 0);
        let (pos, kw) = self.parse_args();
        self.chunk.emit(OpCode::Call, super::pack_call(pos, kw));
        self.chunk.record_call_pos(call_pos);
        true
    }

    pub(super) fn call_range(&mut self) {
        let call_pos = self.last_end as u32;
        self.advance();
        let mut argc = 0u16;
        self.comma_list(|t| t == TokenType::Rpar, |s| {
            // Allow `range(*args)`, UnpackArgs spreads the iterable at runtime.
            if s.eat_if(TokenType::Star) { s.expr(); s.chunk.emit(OpCode::UnpackArgs, 1); }
            else { s.expr(); }
            argc = argc.saturating_add(1);
        });
        self.eat(TokenType::Rpar);
        self.chunk.emit(OpCode::CallRange, argc);
        self.chunk.record_call_pos(call_pos);
    }

    pub(super) fn parse_args(&mut self) -> (u16, u16) {
        self.advance();
        self.parse_args_body()
    }

    // Parse args after `(` already consumed.
    pub(super) fn parse_args_body(&mut self) -> (u16, u16) {
        let mut pos = 0u16;
        let mut kw = 0u16;
        self.comma_list(|t| t == TokenType::Rpar, |s| {
            let unpack = if s.eat_if(TokenType::DoubleStar) { Some(2u16) }
                else if s.eat_if(TokenType::Star) { Some(1u16) }
                else { None };
            if let Some(kind) = unpack {
                s.expr();
                // High bits carry preceding kw-pair count so the VM keeps positionals contiguous.
                s.chunk.emit(OpCode::UnpackArgs, (kw << 2) | kind);
                pos = pos.saturating_add(1);
            } else if matches!(s.peek(), Some(TokenType::Name)) {
                let t = s.advance();
                if matches!(s.peek(), Some(TokenType::Equal)) {
                    let kw_name = s.lexeme(&t).to_string();
                    s.advance();
                    let i = s.chunk.push_const(Value::Str(kw_name));
                    s.chunk.emit(OpCode::LoadConst, i);
                    s.expr();
                    kw = kw.saturating_add(1);
                } else {
                    let elem_start = s.chunk.instructions.len();
                    s.name(t);
                    s.infix_bp(0);
                    // Name-led arg bypasses expr(), parse a trailing ternary here too.
                    s.saw_newline = false;
                    s.ternary_tail(elem_start);
                    s.maybe_comprehension(elem_start, OpCode::BuildList, OpCode::ListAppend);
                    pos = pos.saturating_add(1);
                }
            } else {
                let elem_start = s.chunk.instructions.len();
                s.expr();
                s.maybe_comprehension(elem_start, OpCode::BuildList, OpCode::ListAppend);
                pos = pos.saturating_add(1);
            }
        });
        self.eat(TokenType::Rpar);
        (pos, kw)
    }

    /* class compiles body into fresh chunk, emits MakeClass+decorators+StoreName. */
    pub(super) fn class_def(&mut self) { self.class_def_with(0) }

    /* Consume the next Name, or emit a non-syncing diagnostic and return a synthetic name so parsing continues. */
    fn ident_or_missing(&mut self, msg: &str) -> String {
        if matches!(self.peek(), Some(TokenType::Name)) {
            self.advance_text()
        } else {
            self.diag_at_peek(msg);
            "<missing>".to_string()
        }
    }

    /* Emit one `Call(1)` per decorator, innermost first, each applied to the previous result. */
    fn emit_decorator_calls(&mut self, n: u16) {
        for _ in 0..n {
            let pos = self.last_end as u32;
            self.chunk.emit(OpCode::Call, 1);
            self.chunk.record_call_pos(pos);
        }
    }

    pub(super) fn class_def_with(&mut self, decorators: u16) {
        // Missing name, non-syncing diagnostic + synthetic name so body still parses.
        let cname = self.ident_or_missing("expected class name");

        // Bases are pushed left-to-right, `MakeClass` pops `num_bases` and stores them in the Class.
        let mut num_bases: u16 = 0;
        if self.eat_if(TokenType::Lpar) {
            while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
                self.expr();
                num_bases = num_bases.saturating_add(1);
                if !self.eat_if(TokenType::Comma) { break; }
            }
            self.eat(TokenType::Rpar);
        }

        self.eat(TokenType::Colon);

        let body = self.with_fresh_chunk(|s| s.compile_block());

        let ci = self.chunk.classes.len() as u16;
        // Operand packs `(num_bases << 8) | class_idx`, each field is one byte to keep the dispatch decode cheap.
        if ci > 0xFF { self.error("too many classes in this scope (limit 255)"); return; }
        if num_bases > 0xFF { self.error("too many base classes (limit 255)"); return; }
        self.chunk.classes.push(body);
        self.chunk.emit(OpCode::MakeClass, (num_bases << 8) | ci);

        // Each decorator Calls with the previous result, same as for functions.
        self.emit_decorator_calls(decorators);

        self.emit_store_new(&cname);
    }

    /* def/async def parses signature, compiles body, emits MakeFunction/MakeCoroutine+decorators+StoreName. */
    pub(super) fn func_def_inner(&mut self, decorators: u16, is_async: bool) {
        // Missing name, non-syncing diagnostic + synthetic name so signature+body still parse.
        let fname = self.ident_or_missing("expected function name");
        let (params, defaults) = self.parse_params();
        let body = self.compile_body(&params);

        // Propagate free names to parent chunk so nested defs capture grandparent vars.
        self.push_function(params, body, defaults, Some(&fname), if is_async { OpCode::MakeCoroutine } else { OpCode::MakeFunction });

        self.emit_decorator_calls(decorators);

        self.emit_store_new(&fname);
    }

    pub(super) fn parse_params(&mut self) -> (Vec<String>, u16) {
        // No `(`, diagnostic, consume `:` so compile_body starts at Indent correctly.
        if !matches!(self.peek(), Some(TokenType::Lpar)) {
            self.diag_at_peek("expected '('");
            self.eat_if(TokenType::Colon);
            return (Vec::new(), 0);
        }
        self.advance();
        let mut params = Vec::new();
        let mut defaults = 0u16;
        // Lone `*` flips kw_only, subsequent params get `~` prefix.
        let mut kw_only = false;
        // Break on Rarrow, signals end of params (return type follows).
        while !matches!(self.peek(), Some(TokenType::Rpar | TokenType::Rarrow) | None) {
            if self.eat_if(TokenType::Slash) {
                self.eat_if(TokenType::Comma);
                continue;
            }
            if self.eat_if(TokenType::Star) {
                // Lone `*`, flip kw-only, no param emitted.
                if matches!(self.peek(), Some(TokenType::Comma | TokenType::Rpar)) {
                    self.eat_if(TokenType::Comma);
                    kw_only = true;
                    continue;
                }
                let nm = self.advance_text();
                params.push(s!("*", str &nm));
                self.drain_annotation();
                self.eat_if(TokenType::Comma);
                continue;
            }
            if self.eat_if(TokenType::DoubleStar) {
                let nm = self.advance_text();
                params.push(s!("**", str &nm));
                self.drain_annotation();
                self.eat_if(TokenType::Comma);
                continue;
            }
            let prefix = if kw_only { "~" } else { "" };
            let nm = self.advance_text();
            params.push(if prefix.is_empty() { nm } else { s!(str prefix, str &nm) });
            self.drain_annotation();
            if self.eat_if(TokenType::Equal) {
                self.expr();
                defaults += 1;
                // Trailing `=` marks this param as carrying a default value.
                if let Some(last) = params.last_mut() { last.push('='); }
            }
            self.eat_if(TokenType::Comma);
        }
        self.eat(TokenType::Rpar);
        if self.eat_if(TokenType::Rarrow) {
            while !matches!(self.peek(), Some(TokenType::Colon) | None) { self.advance(); }
        }
        self.eat(TokenType::Colon);
        (params, defaults)
    }

    /* Drains annotation via `advance_raw` (keeps bracket_stack clean), breaks on Rarrow to avoid infinite drain. */
    pub(super) fn drain_annotation(&mut self) {
        if self.eat_if(TokenType::Colon) {
            let mut depth = 0u32;
            loop {
                match self.peek() {
                    None => break,
                    Some(TokenType::Rarrow) => break,
                    Some(TokenType::Lsqb | TokenType::Lpar | TokenType::Lbrace) => {
                        depth += 1;
                        self.advance_raw();
                    }
                    Some(TokenType::Rsqb | TokenType::Rpar | TokenType::Rbrace) => {
                        if depth == 0 { break; }
                        depth -= 1;
                        self.advance_raw();
                    }
                    Some(TokenType::Equal | TokenType::Comma) if depth == 0 => break,
                    _ => { self.advance_raw(); }
                }
            }
        }
    }

    pub(super) fn compile_body(&mut self, params: &[String]) -> SSAChunk {
        let mut body = self.with_fresh_chunk(|s| {
            for p in params {
                // Base name shadows the enclosing scope, prefix/`=` marker must be stripped.
                s.ssa_versions.insert(super::types::param_base_name(p).to_string(), 0);
                let _ = s.push_ssa_name(super::types::param_base_name(p), 0);
            }
            s.compile_block_body();
        });
        body.is_pure = !body.instructions.iter().any(|i| matches!(
            i.opcode,
            OpCode::CallPrint
            | OpCode::StoreItem
            | OpCode::DelItem
            | OpCode::DelAttr
            | OpCode::StoreAttr
            | OpCode::CallInput
            | OpCode::Global
            | OpCode::Nonlocal
            | OpCode::LoadAttr
            | OpCode::Raise
            | OpCode::RaiseFrom
            | OpCode::Yield
        )) && !Self::body_reads_free_name(&body, params);
        // Pre-compute is_generator to avoid O(n) scan per `exec_call`.
        body.is_generator = body.instructions.iter().any(|i| matches!(
            i.opcode,
            OpCode::Yield
        ));
        body
    }

    /* A body is memoizable-pure only if every name it loads is bound locally, free names (globals, builtins) introduce mutable state, making memoization stale. */
    fn body_reads_free_name(body: &SSAChunk, params: &[String]) -> bool {
        // SSA names are `base_version`, strip the trailing `_<digits>` to compare bases.
        fn base(n: &str) -> &str {
            match n.rfind('_') {
                Some(i) if i + 1 < n.len() && n[i + 1..].bytes().all(|b| b.is_ascii_digit()) => &n[..i],
                _ => n,
            }
        }
        let mut locals: crate::util::hash::FxHashSet<&str> =
            params.iter().map(|p| super::types::param_base_name(p)).collect();
        for ins in &body.instructions {
            if ins.opcode == OpCode::StoreName
                && let Some(n) = body.names.get(ins.operand as usize) {
                locals.insert(base(n));
            }
        }
        body.instructions.iter().any(|ins| match ins.opcode {
            OpCode::LoadGlobal => true,
            OpCode::LoadName => body.names.get(ins.operand as usize).is_none_or(|n| !locals.contains(base(n))),
            _ => false,
        })
    }
}
