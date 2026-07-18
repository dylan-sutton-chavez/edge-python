use crate::s;

use super::Parser;
use super::types::OpCode;

use crate::modules::lexer::{Token, TokenType};

use alloc::{string::{String, ToString}, vec, vec::Vec};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /* Statement dispatch; returns true if a value is left on the stack for caller to PopTop. */
    pub(super) fn stmt(&mut self) -> bool {
        // Record ip->source offset before any emit so `resolve()` can map ip to source.
        let ip = self.chunk.instructions.len() as u32;
        let pos = self.tokens.peek().map(|t| t.start as u32).unwrap_or(self.last_end as u32);
        self.chunk.stmt_pos.push((ip, pos));

        match self.peek() {
            Some(TokenType::If) => {
                self.if_stmt();
                false
            }
            Some(TokenType::For) => {
                self.for_stmt_inner(false);
                false
            }
            Some(TokenType::Def) => {
                self.advance();
                self.func_def_inner(0, false);
                false
            }
            Some(TokenType::With) => {
                self.with_stmt_inner(false);
                false
            }
            Some(TokenType::While) => {
                self.while_stmt();
                false
            }
            Some(TokenType::Match) => {
                self.match_stmt();
                false
            }
            Some(TokenType::Yield) => {
                self.advance();
                self.emit_yield();
                true
            }
            Some(TokenType::Async) => {
                self.advance();
                match self.peek() {
                    Some(TokenType::Def) => {
                        self.advance();
                        self.func_def_inner(0, true);
                    }
                    Some(TokenType::For) => { self.for_stmt_inner(true); }
                    Some(TokenType::With) => { self.with_stmt_inner(true); }
                    _ => {}
                }
                false
            }
            Some(TokenType::Await) => {
                self.advance();
                self.expr();
                self.chunk.emit(OpCode::Await, 0);
                true
            }
            Some(TokenType::At) => {
                let mut count = 0u16;
                while self.eat_if(TokenType::At) {
                    self.expr();
                    count += 1;
                }
                if self.eat_if(TokenType::Async) {
                    self.advance();
                    self.func_def_inner(count, true);
                } else if matches!(self.peek(), Some(TokenType::Class)) {
                    self.advance();
                    self.class_def_with(count);
                } else {
                    self.advance();
                    self.func_def_inner(count, false);
                }
                false
            }
            Some(TokenType::Class) => {
                self.advance();
                self.class_def();
                false
            }
            Some(TokenType::Pass) => {
                self.advance();
                false
            }
            Some(TokenType::Try) => {
                self.try_stmt();
                false
            }
            Some(TokenType::Import) => {
                self.import_stmt();
                false
            }
            Some(TokenType::From) => {
                self.parse_from_stmt();
                false
            }
            Some(TokenType::Global) => {
                self.emit_name_list(OpCode::Global);
                false
            }
            Some(TokenType::Nonlocal) => {
                self.emit_name_list(OpCode::Nonlocal);
                false
            }
            Some(TokenType::Assert) => {
                self.advance();
                self.expr();
                if self.eat_if(TokenType::Comma) {
                    // `assert cond, msg` desugars to lazy `if not cond: raise AssertionError(msg)`.
                    let to_raise = self.emit_jump(OpCode::JumpIfFalse);
                    let to_end = self.emit_jump(OpCode::Jump);
                    self.patch(to_raise);
                    let call_pos = self.last_end as u32;
                    let idx = self.chunk.push_name("AssertionError");
                    self.chunk.emit(OpCode::LoadName, idx);
                    self.expr(); // message, only evaluated when the assertion fails
                    self.chunk.emit(OpCode::Call, 1);
                    self.chunk.record_call_pos(call_pos);
                    self.chunk.emit(OpCode::Raise, 0);
                    self.patch(to_end);
                } else {
                    self.chunk.emit(OpCode::Assert, 0);
                }
                false
            }
            Some(TokenType::Del) => {
                self.advance();
                loop {
                    self.parse_del_target();
                    if !self.eat_if(TokenType::Comma) { break; }
                }
                false
            }
            Some(TokenType::Raise) => {
                self.advance();
                // `peek_same_line` keeps the line boundary so a bare `raise` is detected, not parsed as an expr.
                if self.peek_same_line().is_some() {
                    self.expr();
                    if self.eat_if(TokenType::From) {
                        self.expr();
                        self.chunk.emit(OpCode::RaiseFrom, 0);
                    } else {
                        self.chunk.emit(OpCode::Raise, 0);
                    }
                } else {
                    // Bare `raise`: operand 1 tells the VM to re-raise the active exception.
                    self.chunk.emit(OpCode::Raise, 1);
                }
                false
            }
            Some(TokenType::Break) => {
                self.advance();
                if self.loop_breaks.is_empty() {
                    self.error("'break' outside loop");
                } else {
                    self.emit_loop_unwind();
                    // For-loop: PopIter before break so `iter_stack` stays clean.
                    if let Some(true) = self.loop_kinds.last() {
                        self.chunk.emit(OpCode::PopIter, 0);
                    }
                    let j = self.emit_jump(OpCode::Jump);
                    if let Some(breaks) = self.loop_breaks.last_mut() {
                        breaks.push(j);
                    }
                }
                false
            }
            Some(TokenType::Continue) => {
                self.advance();
                if let Some(&start) = self.loop_starts.last() {
                    self.emit_loop_unwind();
                    self.chunk.emit(OpCode::Jump, start);
                } else {
                    self.error("'continue' outside loop");
                }
                false
            }
            Some(TokenType::Star) => {
                self.advance();
                let head = self.advance_text();
                let mut targets = vec![s!("*", str &head)];
                while self.eat_if(TokenType::Comma) {
                    if !matches!(self.peek(), Some(TokenType::Name)) { break; }
                    targets.push(self.advance_text());
                }
                self.eat(TokenType::Equal);
                self.expr();
                let after = (targets.len() - 1) as u16;
                self.chunk.emit(OpCode::UnpackEx, after);
                for target in targets { self.store_name(target.trim_start_matches('*').to_string()); }
                false
            }
            Some(TokenType::Return) => {
                self.advance();
                if matches!(self.peek(), Some(TokenType::Newline | TokenType::Endmarker | TokenType::Dedent) | None) {
                    self.chunk.emit(OpCode::LoadNone, 0);
                } else {
                    self.expr();
                    let count = self.tuple_rest(1, |_| false);
                    if count > 1 { self.chunk.emit(OpCode::BuildTuple, count); }
                }
                self.chunk.emit(OpCode::ReturnValue, 0);
                false
            }
            Some(TokenType::Name) => {
                let t = self.advance();
                self.name_stmt(t)
            }
            // Dangling Indent from a prior error: skip the entire block silently.
            Some(TokenType::Indent) => {
                self.tokens.next();
                let mut depth = 1u32;
                while depth > 0 {
                    match self.tokens.peek().map(|t| t.kind) {
                        Some(TokenType::Indent) => { self.tokens.next(); depth += 1; }
                        Some(TokenType::Dedent) => { self.tokens.next(); depth -= 1; }
                        None | Some(TokenType::Endmarker) => break,
                        _ => { self.tokens.next(); }
                    }
                }
                false
            }
            // Stray Dedent from error recovery: skip silently.
            Some(TokenType::Dedent) => {
                self.tokens.next();
                false
            }
            Some(TokenType::Lsqb) => {
                // `[a, b] = rhs`: a list display followed by `=` is a sequence-unpack target.
                let start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::Equal))
                    && let Some(targets) = self.list_display_targets(start) {
                    self.advance();
                    self.chunk.instructions.truncate(start); // drop the display's loads + BuildList
                    self.expr();
                    let count = self.tuple_rest(1, |s| matches!(s.peek(), Some(TokenType::Newline | TokenType::Endmarker) | None));
                    if count > 1 { self.chunk.emit(OpCode::BuildTuple, count); }
                    self.chunk.emit(OpCode::UnpackSequence, targets.len() as u16);
                    for t in targets { self.store_name(t); }
                    false
                } else {
                    true // plain list display / comprehension statement
                }
            }
            _ => {
                self.expr();
                self.diag_stray_colon();
                true
            }
        }
    }

    /* Runs the finally/with blocks a break/continue crosses before its jump. */
    fn emit_loop_unwind(&mut self) {
        let base = self.loop_cleanup_base.last().copied().unwrap_or(0);
        let count = self.cleanup_count - base;
        if count > 0 {
            self.chunk.emit(OpCode::UnwindFinally, count as u16);
        }
    }

    /* Emits Global/Nonlocal opcodes for a comma-separated name list. Global registers each name so loads/stores route through LoadGlobal/StoreGlobal; Nonlocal records it in `chunk.nonlocals`. */
    pub(super) fn emit_name_list(&mut self, op: OpCode) {
        self.advance();
        loop {
            let name = self.advance_text();
            let idx = self.chunk.push_name(&name);
            self.chunk.emit(op, idx);
            match op {
                OpCode::Global => { self.globals_decl.insert(name); }
                OpCode::Nonlocal if !self.chunk.nonlocals.contains(&name) => self.chunk.nonlocals.push(name),
                _ => {}
            }
            if !self.eat_if(TokenType::Comma) { break; }
        }
    }

    /* Eats `, expr` until `stop`; returns the element count. */
    pub(super) fn tuple_rest(&mut self, mut count: u16, stop: impl Fn(&mut Self) -> bool) -> u16 {
        while self.eat_if(TokenType::Comma) {
            if stop(self) { break; }
            self.expr();
            count += 1;
        }
        count
    }

    // `expr:` at statement level: suggest the missing keyword.
    fn diag_stray_colon(&mut self) {
        if matches!(self.peek(), Some(TokenType::Colon)) {
            let t = self.advance();
            self.error_at(
                t.start, t.end,
                "unexpected ':' (missing 'if', 'while', 'for', or other statement keyword?)",
            );
        }
    }

    pub(super) fn compile_block(&mut self) { self.compile_block_inner(false); }
    pub(super) fn compile_block_body(&mut self) { self.compile_block_inner(true); }

    /* Compiles Indent/Dedent block; is_body=true stops after ReturnValue to skip dead code. */
    fn compile_block_inner(&mut self, is_body: bool) {
        let indented = self.eat_if(TokenType::Indent);
        loop {
            while self.eat_if(TokenType::Semi) {}
            if self.at_end() { break; }
            if matches!(self.peek(), Some(TokenType::Dedent)) {
                self.advance();
                break;
            }
            let produced_value = self.stmt();
            if produced_value {
                self.chunk.emit(OpCode::PopTop, 0);
            }
            if indented { continue; }
            if is_body {
                let just_returned = self.chunk.instructions.last().is_some_and(|i| i.opcode == OpCode::ReturnValue);
                if just_returned || !matches!(self.peek(), Some(TokenType::Semi)) { break; }
            } else if !matches!(self.peek(), Some(TokenType::Semi)) { break; }
        }
    }

    /* Annotation: discard tokens up to `=`; Edge Python is dynamically typed. Returns true when an assignment follows. */
    fn skip_annotation(&mut self) -> bool {
        while !matches!(
            self.peek(),
            Some(TokenType::Equal | TokenType::Dedent | TokenType::Endmarker) | None
        ) {
            self.advance();
        }
        matches!(self.peek(), Some(TokenType::Equal))
    }

    /* Decodes a just-emitted `[a, b, ...]` display into bare target names; None unless every element is a plain name load. */
    fn list_display_targets(&self, start: usize) -> Option<Vec<String>> {
        let (last, loads) = self.chunk.instructions[start..].split_last()?;
        if last.opcode != OpCode::BuildList { return None; }
        let n = last.operand as usize;
        if n == 0 || loads.len() != n { return None; }
        let mut names = Vec::with_capacity(n);
        for ld in loads {
            let raw = self.chunk.names.get(ld.operand as usize)?;
            match ld.opcode {
                OpCode::LoadName => names.push(super::types::ssa_strip(raw).to_string()),
                OpCode::LoadGlobal => names.push(raw.clone()),
                _ => return None,
            }
        }
        Some(names)
    }

    /* Name-led statement: assign, augmented-op, attr, index, call, or tuple unpack. */
    pub(super) fn name_stmt(&mut self, t: Token) -> bool {
        let name = self.lexeme(&t).to_string();

    if self.eat_if(TokenType::Colon) && !self.skip_annotation() {
        return false;
    }

        match self.peek() {
            Some(TokenType::Equal) => {
                self.assign(name);
                false
            }
            Some(t) if Self::augmented_op(&t).is_some() => {
                let op = Self::augmented_op(&t).unwrap();
                self.advance();
                self.emit_load_ssa(name.clone());
                self.expr();
                // In-place variants so list `+=` and set `|=`/`&=`/`^=` mutate the shared object (alias-visible).
                let op = match op {
                    OpCode::Add => OpCode::InPlaceAdd,
                    OpCode::BitOr => OpCode::InPlaceBitOr,
                    OpCode::BitAnd => OpCode::InPlaceBitAnd,
                    OpCode::BitXor => OpCode::InPlaceBitXor,
                    other => other,
                };
                self.chunk.emit(op, 0);
                self.store_name(name);
                false
            }
            Some(TokenType::Lsqb) => {
                self.emit_load_ssa(name);
                self.advance();
                // Slice form: BuildSlice+StoreItem; runtime recognises HeapObj::Slice as splice index.
                if self.parse_subscript() {
                    if matches!(self.peek(), Some(TokenType::Equal)) {
                        self.advance();
                        self.expr();
                        self.chunk.emit(OpCode::StoreItem, 0);
                        return false;
                    }
                    self.chunk.emit(OpCode::GetItem, 0);
                    self.expr_tails();
                    return true;
                }
                if matches!(self.peek(), Some(TokenType::Equal)) {
                    self.advance();
                    self.expr();
                    self.chunk.emit(OpCode::StoreItem, 0);
                    false
                } else if let Some(op) = self.peek().and_then(|t| Self::augmented_op(&t)) {
                    self.emit_augmented_subscript(op);
                    false
                } else {
                    self.chunk.emit(OpCode::GetItem, 0);
                    self.expr_tails();
                    true
                }
            }
            Some(TokenType::Dot) => {
                // Collect the whole `a.b.c` attribute chain; the last attr is the target.
                self.advance();
                let mut attrs = vec![self.advance_text()];
                while matches!(self.peek(), Some(TokenType::Dot)) {
                    self.advance();
                    attrs.push(self.advance_text());
                }
                if self.eat_if(TokenType::Colon) && !self.skip_annotation() {
                    return false;
                }
                let last = attrs.pop().unwrap();
                // Receiver = base object plus every intermediate attribute load.
                self.emit_load_ssa(name.clone());
                for a in &attrs {
                    let idx = self.chunk.push_name(a);
                    self.chunk.emit(OpCode::LoadAttr, idx);
                }
                if matches!(self.peek(), Some(TokenType::Equal)) {
                    self.advance();
                    self.expr();
                    let idx = self.chunk.push_name(&last);
                    self.chunk.emit(OpCode::StoreAttr, idx);
                    false
                } else if let Some(op) = self.peek().and_then(|t| Self::augmented_op(&t)) {
                    self.advance();
                    let idx = self.chunk.push_name(&last);
                    // Need the receiver twice: reload a plain name, else Dup the computed object.
                    if attrs.is_empty() { self.emit_load_ssa(name); } else { self.chunk.emit(OpCode::Dup, 0); }
                    self.chunk.emit(OpCode::LoadAttr, idx);
                    self.expr();
                    self.chunk.emit(op, 0);
                    self.chunk.emit(OpCode::StoreAttr, idx);
                    false
                } else {
                    let idx = self.chunk.push_name(&last);
                    self.chunk.emit(OpCode::LoadAttr, idx);
                    if matches!(self.peek(), Some(TokenType::Lpar)) {
                        let call_pos = self.last_end as u32;
                        let (pos, kw) = self.parse_args();
                        self.chunk.emit(OpCode::Call, super::pack_call(pos, kw));
                        self.chunk.record_call_pos(call_pos);
                    } else if matches!(self.peek(), Some(TokenType::Lsqb)) {
                        self.advance();
                        self.expr();
                        self.eat(TokenType::Rsqb);
                        if matches!(self.peek(), Some(TokenType::Equal)) {
                            self.advance();
                            self.expr();
                            self.chunk.emit(OpCode::StoreItem, 0);
                            return false;
                        } else if let Some(op) = self.peek().and_then(|t| Self::augmented_op(&t)) {
                            self.emit_augmented_subscript(op);
                            return false;
                        } else {
                            self.chunk.emit(OpCode::GetItem, 0);
                        }
                    }
                    self.expr_tails();
                    true
                }
            }
            Some(TokenType::Comma) => {
                let mut targets = vec![name];
                let mut star_pos: Option<usize> = None;
                while self.eat_if(TokenType::Comma) {
                    if self.eat_if(TokenType::Star) {
                        star_pos = Some(targets.len());
                        let nm = self.advance_text();
                        targets.push(s!("*", str &nm));
                    } else if matches!(self.peek(), Some(TokenType::Name)) {
                        targets.push(self.advance_text());
                    } else {
                        break;
                    }
                }
                if matches!(self.peek(), Some(TokenType::Equal)) {
                    self.advance();
                    self.expr();
                    let count = self.tuple_rest(1, |s| matches!(s.peek(), Some(TokenType::Newline | TokenType::Endmarker) | None));
                    if count > 1 {
                        self.chunk.emit(OpCode::BuildTuple, count);
                    }
                    if let Some(sp) = star_pos {
                        let before = sp as u16;
                        let after = (targets.len() - sp - 1) as u16;
                        self.chunk.emit(OpCode::UnpackEx, (before << 8) | after);
                    } else {
                        self.chunk.emit(OpCode::UnpackSequence, targets.len() as u16);
                    }
                    for target in targets {
                        self.store_name(target.trim_start_matches('*').to_string());
                    }
                    false
                } else {
                    for t in &targets {
                        self.emit_load_ssa(t.clone());
                    }
                    self.chunk.emit(OpCode::BuildTuple, targets.len() as u16);
                    true
                }
            }
            Some(TokenType::Lpar) => {
                // `name(...)` at statement level: allow postfix chains like `super().__init__(x)`.
                let leaves = self.call(name);
                if leaves { self.expr_tails(); }
                leaves
            }
            _ => {
                self.emit_load_ssa(name);
                self.expr_tails();
                self.diag_stray_colon();
                true
            }
        }
    }

    /* `x[i] op= rhs`: Dup2 preserves container+index; GetItem, apply op, StoreItem. */
    pub(super) fn emit_augmented_subscript(&mut self, op: OpCode) {
        self.advance();
        self.chunk.emit(OpCode::Dup2, 0);
        self.chunk.emit(OpCode::GetItem, 0);
        self.expr();
        self.chunk.emit(op, 0);
        self.chunk.emit(OpCode::StoreItem, 0);
    }

    pub(super) fn augmented_op(tok: &TokenType) -> Option<OpCode> {
        match tok {
            TokenType::PlusEqual => Some(OpCode::Add),
            TokenType::MinEqual => Some(OpCode::Sub),
            TokenType::StarEqual => Some(OpCode::Mul),
            TokenType::SlashEqual => Some(OpCode::Div),
            TokenType::DoubleSlashEqual => Some(OpCode::FloorDiv),
            TokenType::PercentEqual => Some(OpCode::Mod),
            TokenType::DoubleStarEqual => Some(OpCode::Pow),
            TokenType::AmperEqual => Some(OpCode::BitAnd),
            TokenType::VbarEqual => Some(OpCode::BitOr),
            TokenType::CircumflexEqual => Some(OpCode::BitXor),
            TokenType::LeftShiftEqual => Some(OpCode::Shl),
            TokenType::RightShiftEqual => Some(OpCode::Shr),
            _ => None,
        }
    }

    /* Parses one `del` target: name, subscript, attribute, or a parenthesized group. */
    fn parse_del_target(&mut self) {
        if matches!(self.peek(), Some(TokenType::Name)) {
            let name = self.advance_text();
            if self.eat_if(TokenType::Lsqb) {
                // del `x[k]` or `x[a:b]`: BuildSlice so DelItem sees HeapObj::Slice.
                self.emit_load_ssa(name);
                self.parse_subscript();
                // Chained subscripts (`d[0][0]`): all but the last index in place.
                while self.eat_if(TokenType::Lsqb) {
                    self.chunk.emit(OpCode::GetItem, 0);
                    self.parse_subscript();
                }
                self.chunk.emit(OpCode::DelItem, 0);
            } else if matches!(self.peek(), Some(TokenType::Dot)) {
                // del `obj.attr` (chained): load object, LoadAttr intermediates, DelAttr last.
                self.emit_load_ssa(name);
                self.eat(TokenType::Dot);
                let mut attr = self.advance_text();
                while matches!(self.peek(), Some(TokenType::Dot)) {
                    let idx = self.chunk.push_name(&attr);
                    self.chunk.emit(OpCode::LoadAttr, idx);
                    self.eat(TokenType::Dot);
                    attr = self.advance_text();
                }
                let idx = self.chunk.push_name(&attr);
                self.chunk.emit(OpCode::DelAttr, idx);
            } else {
                let idx = self.push_ssa_name(&name, self.current_version(&name));
                self.chunk.emit(OpCode::Del, idx);
            }
        } else {
            // Parse as an expression, then rewrite the trailing access into its delete form.
            self.expr();
            match self.chunk.instructions.last().map(|i| i.opcode) {
                Some(OpCode::GetItem) => self.chunk.instructions.last_mut().unwrap().opcode = OpCode::DelItem,
                Some(OpCode::LoadAttr) => self.chunk.instructions.last_mut().unwrap().opcode = OpCode::DelAttr,
                Some(OpCode::LoadName) => self.chunk.instructions.last_mut().unwrap().opcode = OpCode::Del,
                // `del (a, b)` / `del [a, b]`: a target group unbinds each plain name.
                Some(OpCode::BuildTuple | OpCode::BuildList) => self.del_group_targets(),
                _ => {}
            }
        }
    }

    /* Rewrites a just-built tuple/list of name loads into individual unbinds. */
    fn del_group_targets(&mut self) {
        let last = self.chunk.instructions.len() - 1;
        let n = self.chunk.instructions[last].operand as usize;
        if n >= 1 && last >= n
            && self.chunk.instructions[last - n..last].iter().all(|i| i.opcode == OpCode::LoadName) {
            self.chunk.instructions.truncate(last);
            for ins in &mut self.chunk.instructions[last - n..] { ins.opcode = OpCode::Del; }
        }
    }

    /* Emit `yield` / `yield from` / bare `yield`, leaving the produced value on the stack so it works in both statement and expression position. Assumes the `yield` keyword was already consumed. */
    pub(super) fn emit_yield(&mut self) {
        // No value when a line boundary or a closing token follows (`(yield)`, `f(yield)`).
        let bare = matches!(
            self.peek_same_line(),
            None | Some(TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace
                | TokenType::Comma | TokenType::Colon)
        );
        if bare {
            self.chunk.emit(OpCode::LoadNone, 0);
            self.chunk.emit(OpCode::Yield, 0);
        } else if self.eat_if(TokenType::From) {
            // `yield from`: GetIter+ForIter+Yield loop; LoadYieldFrom pushes the subiterator's return value.
            self.expr();
            self.chunk.emit(OpCode::GetIter, 0);
            let loop_start = self.chunk.instructions.len() as u16;
            let fi = self.emit_jump(OpCode::ForIter);
            self.chunk.emit(OpCode::Yield, 0);
            self.chunk.emit(OpCode::PopTop, 0);
            self.chunk.emit(OpCode::Jump, loop_start);
            self.patch(fi);
            self.chunk.emit(OpCode::LoadYieldFrom, 0);
        } else {
            self.expr();
            self.chunk.emit(OpCode::Yield, 0);
        }
    }

    pub(super) fn assign(&mut self, name: String) {
        self.advance();
        self.expr();
        // `x = 1,` / `x = 1, 2`: a trailing comma builds a tuple right-hand side.
        if matches!(self.peek_same_line(), Some(TokenType::Comma)) {
            // A line boundary ends the tuple; `peek_same_line` won't cross the Newline.
            let count = self.tuple_rest(1, |s| s.peek_same_line().is_none());
            self.chunk.emit(OpCode::BuildTuple, count);
        }
        self.store_name(name);
    }
}
