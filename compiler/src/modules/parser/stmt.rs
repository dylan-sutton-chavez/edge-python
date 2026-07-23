use crate::s;

use super::Parser;
use super::types::{Instruction, OpCode};

use crate::modules::lexer::{Token, TokenType};

use alloc::{string::{String, ToString}, vec, vec::Vec};

/* Assignment-target shapes for sequence unpacking; complex ones carry their captured load prefix plus its original offset for jump-shifted replay. */
pub(super) enum UnpackTarget {
    Name(String),
    Attr(Vec<Instruction>, usize, u16),
    Item(Vec<Instruction>, usize),
    Nested(Vec<UnpackTarget>),
}

/* One element of a comma-led statement; plain names defer their load until proven expression. */
enum TupleElem {
    Named(String, bool),
    Range(usize, usize),
}

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
                self.rhs_tuple();
                // Leading star: equivalent to star at position 0.
                self.emit_unpack_stores(&targets, Some(0));
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
            _ => {
                // `(a, b) = rhs` / `[a, b] = rhs`: a display followed by `=` is a sequence-unpack target.
                let start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::Equal))
                    && let Some(targets) = self.display_targets(start) {
                    self.advance();
                    self.chunk.truncate_to(start);
                    self.rhs_tuple();
                    self.chunk.emit(OpCode::UnpackSequence, targets.len() as u16);
                    self.emit_target_stores(&targets);
                    return false;
                }
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

    /* Assignment RHS: one expression or a comma tuple, ends at the line boundary. */
    fn rhs_tuple(&mut self) {
        self.expr();
        let count = self.tuple_rest(1, |s| matches!(s.peek(), Some(TokenType::Newline | TokenType::Endmarker) | None));
        if count > 1 { self.chunk.emit(OpCode::BuildTuple, count); }
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

    /* Per-opcode stack effect for target segmentation; None = shape unknown, caller bails. */
    fn stack_delta(op: OpCode, operand: u16) -> Option<i32> {
        use OpCode::*;
        Some(match op {
            LoadConst | LoadName | LoadGlobal | LoadTrue | LoadFalse | LoadNone | LoadEllipsis => 1,
            LoadAttr | Minus | Not | BitNot | Pos => 0,
            Add | Sub | Mul | Div | Mod | Pow | FloorDiv | Eq | NotEq | Lt | Gt | LtEq | GtEq
            | BitAnd | BitOr | BitXor | Shl | Shr | In | NotIn | Is | IsNot | GetItem => -1,
            BuildTuple | BuildList | BuildSet | BuildString | BuildSlice => 1 - operand as i32,
            BuildDict => 1 - 2 * (operand as i32),
            _ => return None,
        })
    }

    /* Split an n-element display body into per-element ranges. Scans backward: each element is the minimal suffix with net stack effect +1, which is unambiguous even for nested displays. */
    fn split_display(&self, lo: usize, hi: usize, n: usize) -> Option<Vec<(usize, usize)>> {
        let mut ranges = vec![(0usize, 0usize); n];
        let mut end = hi;
        for k in (0..n).rev() {
            let mut need = 1i32;
            let mut i = end;
            while need > 0 {
                if i == lo { return None; }
                i -= 1;
                let ins = self.chunk.instructions[i];
                need -= Self::stack_delta(ins.opcode, ins.operand)?;
            }
            ranges[k] = (i, end);
            end = i;
        }
        (end == lo).then_some(ranges)
    }

    /* Reinterpret emitted load instructions as one assignment target; None if not assignable. */
    fn range_target(&self, lo: usize, hi: usize) -> Option<UnpackTarget> {
        if hi <= lo || hi > self.chunk.instructions.len() { return None; }
        let (last, prefix) = self.chunk.instructions[lo..hi].split_last()?;
        // Jumps relocate via `push_shifted`; Phi cannot move, it is anchored by `phi_map`.
        let relocatable = !prefix.iter().any(|i| matches!(i.opcode, OpCode::Phi));
        match last.opcode {
            OpCode::LoadName if prefix.is_empty() => {
                let raw = self.chunk.names.get(last.operand as usize)?;
                Some(UnpackTarget::Name(super::types::ssa_strip(raw).to_string()))
            }
            OpCode::LoadGlobal if prefix.is_empty() => {
                Some(UnpackTarget::Name(self.chunk.names.get(last.operand as usize)?.clone()))
            }
            OpCode::LoadAttr if relocatable => Some(UnpackTarget::Attr(prefix.to_vec(), lo, last.operand)),
            OpCode::GetItem if relocatable => Some(UnpackTarget::Item(prefix.to_vec(), lo)),
            OpCode::BuildTuple | OpCode::BuildList => {
                let n = last.operand as usize;
                if n == 0 { return None; }
                let ranges = self.split_display(lo, hi - 1, n)?;
                let ts: Option<Vec<UnpackTarget>> = ranges.iter()
                    .map(|&(a, b)| self.range_target(a, b)).collect();
                Some(UnpackTarget::Nested(ts?))
            }
            _ => None,
        }
    }

    /* Decodes a just-emitted `(a, b)` / `[a, b]` display into unpack targets. */
    fn display_targets(&self, start: usize) -> Option<Vec<UnpackTarget>> {
        match self.range_target(start, self.chunk.instructions.len())? {
            UnpackTarget::Nested(ts) => Some(ts),
            _ => None,
        }
    }

    /* Re-emit a captured target prefix at the current position, shifting its jumps. */
    fn replay_prefix(&mut self, prefix: &[Instruction], lo: usize) {
        let delta = self.chunk.instructions.len() as i64 - lo as i64;
        self.push_shifted(prefix.to_vec(), delta);
    }

    /* One store per target; values arrive top-first, complex targets replay their captured loads. */
    fn emit_target_stores(&mut self, targets: &[UnpackTarget]) {
        for t in targets {
            match t {
                UnpackTarget::Name(n) => self.store_name(n.clone()),
                UnpackTarget::Attr(prefix, lo, attr_idx) => {
                    self.replay_prefix(prefix, *lo);
                    self.chunk.emit(OpCode::Swap, 0);
                    self.chunk.emit(OpCode::StoreAttr, *attr_idx);
                }
                UnpackTarget::Item(prefix, lo) => {
                    self.replay_prefix(prefix, *lo);
                    self.chunk.emit(OpCode::Rot3, 0);
                    self.chunk.emit(OpCode::StoreItem, 0);
                }
                UnpackTarget::Nested(ts) => {
                    self.chunk.emit(OpCode::UnpackSequence, ts.len() as u16);
                    self.emit_target_stores(ts);
                }
            }
        }
    }

    /* Emit deferred loads for plain-name elements still pending; keeps tuple element order. */
    fn flush_named(&mut self, elems: &mut [TupleElem]) {
        for e in elems.iter_mut() {
            if let TupleElem::Named(n, emitted) = e
                && !*emitted {
                    self.emit_load_ssa(n.clone());
                    *emitted = true;
                }
        }
    }

    /* Comma after one element: unpack assignment or tuple expression. `first` carries a plain leading name whose load is deferred; None means the element is already emitted at `start`. */
    pub(super) fn unpack_or_tuple(&mut self, start: usize, first: Option<String>) -> bool {
        let mut elems: Vec<TupleElem> = Vec::new();
        match first {
            Some(n) => elems.push(TupleElem::Named(n, false)),
            None => elems.push(TupleElem::Range(start, self.chunk.instructions.len())),
        }
        let mut star_pos: Option<usize> = None;
        // Elements may be targets: `=` must not parse as embedded assignment.
        self.in_target_list = true;
        while self.eat_if(TokenType::Comma) {
            if matches!(self.peek(), Some(TokenType::Newline | TokenType::Endmarker | TokenType::Equal) | None) { break; }
            if self.eat_if(TokenType::Star) {
                star_pos = Some(elems.len());
                let nm = self.advance_text();
                if matches!(self.peek(), Some(TokenType::Dot | TokenType::Lsqb)) {
                    self.error("starred assignment target must be a name");
                }
                elems.push(TupleElem::Named(nm, false));
                continue;
            }
            if matches!(self.peek(), Some(TokenType::Name)) {
                let t = self.advance();
                let nm = self.lexeme(&t).to_string();
                if matches!(self.peek(), Some(TokenType::Comma | TokenType::Equal | TokenType::Newline | TokenType::Endmarker) | None) {
                    // Plain name element: defer the load, no phantom SSA slot on assignment.
                    elems.push(TupleElem::Named(nm, false));
                    continue;
                }
                // Name-led complex element: flush deferred loads, then continue its expression.
                self.flush_named(&mut elems);
                let lo = self.chunk.instructions.len();
                self.emit_load_ssa(nm);
                self.expr_tails();
                elems.push(TupleElem::Range(lo, self.chunk.instructions.len()));
                continue;
            }
            self.flush_named(&mut elems);
            let lo = self.chunk.instructions.len();
            self.expr();
            elems.push(TupleElem::Range(lo, self.chunk.instructions.len()));
        }
        self.in_target_list = false;
        if !matches!(self.peek(), Some(TokenType::Equal)) {
            if star_pos.is_some() { self.error("cannot use starred expression here"); }
            self.flush_named(&mut elems);
            self.chunk.emit(OpCode::BuildTuple, elems.len() as u16);
            return true;
        }
        let targets: Option<Vec<UnpackTarget>> = elems.iter().map(|e| match e {
            TupleElem::Named(n, _) => Some(UnpackTarget::Name(n.clone())),
            TupleElem::Range(lo, hi) => self.range_target(*lo, *hi),
        }).collect();
        let eq = self.advance();
        self.chunk.truncate_to(start);
        self.rhs_tuple();
        let Some(targets) = targets else {
            self.error_at(eq.start, eq.end, "cannot assign to this expression");
            return true;
        };
        if let Some(sp) = star_pos {
            // Starred lists keep the historic name-only path through UnpackEx.
            let names: Option<Vec<String>> = targets.iter().map(|t| match t {
                UnpackTarget::Name(n) => Some(n.clone()),
                _ => None,
            }).collect();
            match names {
                Some(ns) => self.emit_unpack_stores(&ns, Some(sp)),
                None => self.error("starred assignment supports only plain name targets"),
            }
        } else {
            self.chunk.emit(OpCode::UnpackSequence, targets.len() as u16);
            self.emit_target_stores(&targets);
        }
        false
    }

    /* Name-led statement: assign, augmented-op, attr, index, call, or tuple unpack. */
    pub(super) fn name_stmt(&mut self, t: Token) -> bool {
        let name = self.lexeme(&t).to_string();
        let start = self.chunk.instructions.len();

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
                    if matches!(self.peek(), Some(TokenType::Comma)) {
                        return self.unpack_or_tuple(start, None);
                    }
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
                    if matches!(self.peek(), Some(TokenType::Comma)) {
                        return self.unpack_or_tuple(start, None);
                    }
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
                    if matches!(self.peek(), Some(TokenType::Comma)) {
                        return self.unpack_or_tuple(start, None);
                    }
                    true
                }
            }
            Some(TokenType::Comma) => {
                self.unpack_or_tuple(start, Some(name))
            }
            Some(TokenType::Lpar) => {
                // `name(...)` at statement level: allow postfix chains like `super().__init__(x)`.
                let leaves = self.call(name);
                if leaves {
                    self.expr_tails();
                    if matches!(self.peek(), Some(TokenType::Comma)) {
                        return self.unpack_or_tuple(start, None);
                    }
                }
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
