use crate::s;

use super::Parser;
use super::types::OpCode;

use crate::lexer::{Token, TokenType};

use alloc::{vec, vec::Vec, string::{String, ToString}};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /* if/elif/else compiler, emits JumpIfFalse/Jump and patches branch join targets */

    pub(super) fn if_stmt(&mut self) {
        self.advance();
        self.enter_block();
        self.if_body();
        self.commit_block();
    }

    pub(super) fn if_body(&mut self) {
        self.expr();
        let jf = self.emit_jump(OpCode::JumpIfFalse);

        self.eat(TokenType::Colon);
        self.compile_block();

        match self.peek() {
            Some(TokenType::Elif) => {
                self.advance();
                let jmp = self.emit_jump(OpCode::Jump);
                self.mid_block();
                self.patch(jf);
                self.if_body();
                self.patch(jmp);
            }
            Some(TokenType::Else) => {
                self.advance();
                let jmp = self.emit_jump(OpCode::Jump);
                self.mid_block();
                self.patch(jf);
                self.eat(TokenType::Colon);
                self.compile_block();
                self.patch(jmp);
            }
            _ => {
                self.patch(jf);
            }
        }
    }

    /* match/case, literals, captures, wildcards, OR, guards, sequences, emits subject-load + pattern + guard + Jump-end. */
    pub(super) fn match_stmt(&mut self) {
        self.advance();
        self.expr();

        let ver = self.increment_version(super::SSA_TMP_MATCH);
        let subj = self.chunk.push_name(&s!(str super::SSA_TMP_MATCH, int ver));
        self.chunk.emit(OpCode::StoreName, subj);

        self.eat(TokenType::Colon);
        self.eat_if(TokenType::Indent);

        let mut end_jumps = Vec::new();

        while matches!(self.peek(), Some(TokenType::Case)) {
            self.advance();

            let mut fail_jumps: Vec<usize> = Vec::new();
            self.parse_pattern(subj, &mut fail_jumps);

            // Guard fail joins pattern fails, both land at the next case.
            if self.eat_if(TokenType::If) {
                self.expr();
                fail_jumps.push(self.emit_jump(OpCode::JumpIfFalse));
            }

            self.eat(TokenType::Colon);
            self.compile_block();

            end_jumps.push(self.emit_jump(OpCode::Jump));

            for j in fail_jumps { self.patch(j); }
        }

        self.eat_if(TokenType::Dedent);

        for pos in end_jumps { self.patch(pos); }
    }

    /* Emits bytecode for one pattern, appends case-fail jumps to `fail_jumps`, reloads subject from subj. */
    pub(super) fn parse_pattern(&mut self, subj: u16, fail_jumps: &mut Vec<usize>) {
        // OR pattern, each alt gets a success-jump landing past all alts.
        let mut alts: Vec<Vec<usize>> = Vec::new();
        let mut succ_jumps: Vec<usize> = Vec::new();

        loop {
            let mut this_alt_fails: Vec<usize> = Vec::new();
            self.parse_simple_pattern(subj, &mut this_alt_fails);
            // On match, jump past remaining alts.
            succ_jumps.push(self.emit_jump(OpCode::Jump));
            // On mismatch, redirect fails to next alt, only last alt propagates to case-fail.
            alts.push(this_alt_fails);
            if !matches!(self.peek(), Some(TokenType::Vbar)) {
                break;
            }
            // Previous alt's fails land at next alt entry.
            let here = self.chunk.instructions.len();
            for j in alts.last_mut().unwrap().drain(..) {
                let target = here as u16;
                self.patch_to(j, target);
            }
            self.advance(); // consume `|`
        }

        // Last alt's fails become the case-fail exits.
        if let Some(last) = alts.last_mut() {
            fail_jumps.append(last);
        }

        // All success jumps land here, past the OR.
        for j in succ_jumps { self.patch(j); }
    }

    /* Dispatches single pattern alternative by token, wildcard, capture (StoreName), or literal equality. */
    fn parse_simple_pattern(&mut self, subj: u16, fail_jumps: &mut Vec<usize>) {
        match self.peek() {
            // Wildcard always succeeds, no binding.
            Some(TokenType::Underscore) => { self.advance(); }
            // Sequence pattern.
            Some(TokenType::Lsqb) => { self.parse_sequence_pattern(subj, fail_jumps); }
            // Capture binds subject to name, always succeeds.
            Some(TokenType::Name) => {
                let t = self.advance();
                let name = self.lexeme(&t).to_string();
                self.chunk.emit(OpCode::LoadName, subj);
                self.emit_store_new(&name);
            }
            // Literal/expr, equality-test against subject, precedence > bitwise-or keeps `1|2|3` as OR pattern.
            _ => {
                self.chunk.emit(OpCode::LoadName, subj);
                self.expr_bp(11);
                self.chunk.emit(OpCode::Eq, 0);
                fail_jumps.push(self.emit_jump(OpCode::JumpIfFalse));
            }
        }
    }

    /* Sequence pattern length-checks subject, indexes items into fresh slots, star captures middle slice. */
    fn parse_sequence_pattern(&mut self, subj: u16, fail_jumps: &mut Vec<usize>) {
        self.advance(); // consume `[`

        // Pass 1 buffers tokens to count items and locate the star.
        let mut buffered: Vec<crate::lexer::Token> = Vec::new();
        let mut depth: i32 = 0;
        let mut commas = 0;
        let mut empty = true;
        let mut star_count = 0;
        loop {
            match self.peek() {
                Some(TokenType::Rsqb) if depth == 0 => break,
                None => break,
                Some(TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace) => {
                    depth += 1; empty = false;
                    buffered.push(self.advance());
                }
                Some(TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace) => {
                    depth -= 1;
                    buffered.push(self.advance());
                }
                Some(TokenType::Comma) if depth == 0 => {
                    commas += 1; empty = false;
                    buffered.push(self.advance());
                }
                Some(TokenType::Star) if depth == 0 => {
                    star_count += 1; empty = false;
                    buffered.push(self.advance());
                }
                _ => { empty = false; buffered.push(self.advance()); }
            }
        }
        let item_count = if empty { 0 } else { commas + 1 };
        self.eat(TokenType::Rsqb);

        if star_count > 1 {
            self.error_at(buffered[0].start, buffered.last().unwrap().end, "multiple stars in sequence pattern");
        }

        // Type guard, a non-sequence subject fails the pattern instead of erroring on len().
        self.chunk.emit(OpCode::LoadName, subj);
        self.chunk.emit(OpCode::MatchSeq, 0);
        fail_jumps.push(self.emit_jump(OpCode::JumpIfFalse));

        // Length check, exact without star, >= (count-1) with star.
        let len_min = if star_count > 0 { item_count - 1 } else { item_count };
        self.chunk.emit(OpCode::LoadName, subj);
        self.chunk.emit(OpCode::CallLen, 1);
        let ci = self.chunk.push_const(super::types::Value::Int(len_min as i64));
        self.chunk.emit(OpCode::LoadConst, ci);
        let cmp = if star_count > 0 { OpCode::GtEq } else { OpCode::Eq };
        self.chunk.emit(cmp, 0);
        fail_jumps.push(self.emit_jump(OpCode::JumpIfFalse));

        // Pass 2 walks buffered tokens, emitting per-item bytecode.
        let saved: Vec<crate::lexer::Token> = buffered;
        let mut idx = 0usize;
        let total = saved.len();

        // Fresh slot to use as the per-item sub-subject.
        let item_ver = self.increment_version(super::SSA_TMP_MATCH_ITEM);
        let item_subj = self.chunk.push_name(&s!(str super::SSA_TMP_MATCH_ITEM, int item_ver));

        // Walk items, split on top-level commas.
        let mut item_idx: i64 = 0;
        let mut star_idx_seen: Option<i64> = None;
        while idx < total {
            // Check for star prefix.
            let is_star = matches!(saved.get(idx), Some(t) if t.kind == TokenType::Star);
            if is_star { idx += 1; }
            // Find item end at next depth-0 comma or EOF.
            let item_start = idx;
            let mut d: i32 = 0;
            while idx < total {
                let k = saved[idx].kind;
                if d == 0 && k == TokenType::Comma { break; }
                if matches!(k, TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace) { d += 1; }
                if matches!(k, TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace) { d -= 1; }
                idx += 1;
            }
            let item_end = idx;
            // Consume trailing comma.
            if idx < total && saved[idx].kind == TokenType::Comma { idx += 1; }

            // Index, positive before star, star gets slice, negative after.
            if is_star {
                star_idx_seen = Some(item_idx);
                // Slice subj[item_idx : len-suffix]
                let suffix = (item_count as i64) - item_idx - 1;
                self.chunk.emit(OpCode::LoadName, subj);
                let cs = self.chunk.push_const(super::types::Value::Int(item_idx));
                self.chunk.emit(OpCode::LoadConst, cs);
                self.chunk.emit(OpCode::LoadName, subj);
                self.chunk.emit(OpCode::CallLen, 1);
                let cend = self.chunk.push_const(super::types::Value::Int(suffix));
                self.chunk.emit(OpCode::LoadConst, cend);
                self.chunk.emit(OpCode::Sub, 0);
                self.chunk.emit(OpCode::LoadNone, 0);
                self.chunk.emit(OpCode::BuildSlice, 3);
                self.chunk.emit(OpCode::GetItem, 0);
                self.chunk.emit(OpCode::CallList, 1);
                self.chunk.emit(OpCode::StoreName, item_subj);
            } else {
                // Negative index for items after the star.
                let physical_idx: i64 = if star_idx_seen.is_some() {
                    -((item_count as i64) - item_idx)
                } else {
                    item_idx
                };
                self.chunk.emit(OpCode::LoadName, subj);
                let cidx = self.chunk.push_const(super::types::Value::Int(physical_idx));
                self.chunk.emit(OpCode::LoadConst, cidx);
                self.chunk.emit(OpCode::GetItem, 0);
                self.chunk.emit(OpCode::StoreName, item_subj);
            }

            // Dispatch wildcard, capture, or literal. No nested sequences, use if/match instead.
            let toks = &saved[item_start..item_end];
            if toks.is_empty() || (toks.len() == 1 && toks[0].kind == TokenType::Underscore) {
                // Wildcard discards stored `item_subj`.
            } else if toks.len() == 1 && toks[0].kind == TokenType::Name {
                let name = self.source[toks[0].start..toks[0].end].to_string();
                if name != "_" {
                    self.chunk.emit(OpCode::LoadName, item_subj);
                    self.emit_store_new(&name);
                }
            } else {
                // Literal replays tokens, supports Int/Float/Str/Bool/None/negation, else error.
                let mut pos = 0;
                let mut neg = false;
                if pos < toks.len() && toks[pos].kind == TokenType::Minus { neg = true; pos += 1; }
                if pos < toks.len() {
                    let t = &toks[pos];
                    let raw = &self.source[t.start..t.end];
                    let val = match t.kind {
                        TokenType::Int => {
                            let cleaned = raw.replace('_', "");
                            // i64 first, fall back to i128 when literal is wider.
                            cleaned.parse::<i64>().ok().map(super::types::Value::Int)
                                .or_else(|| cleaned.parse::<i128>().ok().map(super::types::Value::LongInt))
                        }
                        TokenType::Float => raw.replace('_', "").parse::<f64>().ok().map(super::types::Value::Float),
                        TokenType::String => Some(super::types::Value::Str(super::types::parse_string(raw))),
                        TokenType::True => Some(super::types::Value::Bool(true)),
                        TokenType::False => Some(super::types::Value::Bool(false)),
                        TokenType::None => Some(super::types::Value::None),
                        _ => None,
                    };
                    if let Some(mut v) = val {
                        if neg {
                            v = match v {
                                super::types::Value::Int(i) => super::types::Value::Int(-i),
                                // i128::MIN.neg() overflows, trap as parse error to keep sub-pattern semantics aligned with literal materialisation.
                                super::types::Value::LongInt(i) => match i.checked_neg() {
                                    Some(n) => super::types::Value::LongInt(n),
                                    None => super::types::Value::LongInt(i),
                                },
                                super::types::Value::Float(f) => super::types::Value::Float(-f),
                                other => other,
                            };
                        }
                        self.chunk.emit(OpCode::LoadName, item_subj);
                        let ci = self.chunk.push_const(v);
                        self.chunk.emit(OpCode::LoadConst, ci);
                        self.chunk.emit(OpCode::Eq, 0);
                        fail_jumps.push(self.emit_jump(OpCode::JumpIfFalse));
                    } else {
                        self.error_at(toks[0].start, toks.last().unwrap().end, "unsupported sub-pattern in sequence (use literals, names, or _)");
                    }
                }
            }
            item_idx += 1;
        }
    }

    /* while, cond + body + back-edge, optional else when cond falsifies. */

    pub(super) fn while_stmt(&mut self) {
        self.advance();
        self.enter_block();

        let loop_start = self.chunk.instructions.len() as u16;
        self.loop_starts.push(loop_start);
        self.loop_breaks.push(vec![]);
        self.loop_kinds.push(false);
        self.loop_cleanup_base.push(self.cleanup_count);

        self.expr();
        let jf = self.emit_jump(OpCode::JumpIfFalse);

        self.eat(TokenType::Colon);
        self.compile_block();

        self.chunk.emit(OpCode::Jump, loop_start);
        self.patch(jf);

        // Pop loop state before the else so its break/continue target the enclosing loop.
        self.loop_starts.pop();
        self.loop_kinds.pop();
        self.loop_cleanup_base.pop();
        let breaks = self.loop_breaks.pop().unwrap_or_default();

        if self.eat_if(TokenType::Else) {
            self.eat(TokenType::Colon);
            self.compile_block();
        }

        // Loop breaks land past the else clause.
        for pos in breaks { self.patch(pos); }

        self.commit_block();
    }

    /* for / async for, with optional tuple/star unpacking. */

    pub(super) fn for_stmt_inner(&mut self, is_async: bool) {
        self.advance();

        let parens = self.eat_if(TokenType::Lpar);
        let mut vars = Vec::new();
        let mut star_pos: Option<usize> = None;
        loop {
            if self.eat_if(TokenType::Star) { star_pos = Some(vars.len()); }
            vars.push(self.advance_text());
            if !self.eat_if(TokenType::Comma) { break; }
            if matches!(self.peek(), Some(TokenType::In | TokenType::Rpar)) { break; }
        }
        if parens {
            self.eat(TokenType::Rpar);
        }

        self.eat(TokenType::In);
        self.expr();
        // Unparenthesized tuple iterable, `for x in 1, 2:` iterates the tuple.
        let n = self.tuple_rest(1, |s| matches!(s.peek(), Some(TokenType::Colon) | None));
        if n > 1 { self.chunk.emit(OpCode::BuildTuple, n); }
        self.chunk.emit(OpCode::GetIter, is_async as u16);

        self.enter_block();

        let loop_start = self.chunk.instructions.len() as u16;
        self.loop_starts.push(loop_start);
        self.loop_breaks.push(vec![]);
        self.loop_kinds.push(true);
        self.loop_cleanup_base.push(self.cleanup_count);

        let fi = self.emit_jump(OpCode::ForIter);

        if vars.len() == 1 && star_pos.is_none() {
            self.store_name(vars[0].clone());
        } else {
            self.emit_unpack_stores(&vars, star_pos);
        }

        self.eat(TokenType::Colon);
        self.compile_block();

        self.chunk.emit(OpCode::Jump, loop_start);
        self.patch(fi);

        // Pop loop state before the else so its break/continue target the enclosing loop.
        self.loop_starts.pop();
        self.loop_kinds.pop();
        self.loop_cleanup_base.pop();
        let breaks = self.loop_breaks.pop().unwrap_or_default();

        if !is_async && self.eat_if(TokenType::Else) {
            self.eat(TokenType::Colon);
            self.compile_block();
        }

        // Loop breaks land past the else clause.
        for pos in breaks { self.patch(pos); }

        self.commit_block();
    }

    /* try / except / else / finally with exception arm chaining. */

    pub(super) fn try_stmt(&mut self) {
        self.advance();
        self.eat(TokenType::Colon);

        // Outer cleanup frame, the finally runs on every exit (normal, return, break, raise).
        let fin_setup = self.emit_jump(OpCode::SetupFinally);
        self.cleanup_count += 1;
        let setup = self.emit_jump(OpCode::SetupExcept);

        self.enter_block();
        self.compile_block();

        self.chunk.emit(OpCode::PopExcept, 0);
        let success_jump = self.emit_jump(OpCode::Jump);

        self.mid_block();

        self.patch(setup);

        let mut end_jumps: Vec<usize> = Vec::new();
        let mut next_arm_jump: Option<usize> = None;
        let mut had_bare = false;
        let mut had_except = false;

        while self.eat_if(TokenType::Except) {
            had_except = true;
            if let Some(j) = next_arm_jump.take() { self.patch(j); }
            if had_bare {
                self.error("default 'except:' must be last");
                break;
            }

            let mut as_name: Option<String> = None;
            if matches!(self.peek(), Some(TokenType::Colon)) {
                had_bare = true;
                self.chunk.emit(OpCode::PopTop, 0);
            } else {
                self.chunk.emit(OpCode::Dup, 0);
                self.expr();
                let isinst_pos = self.last_end as u32;
                self.chunk.emit(OpCode::CallIsInstance, 2);
                self.chunk.record_call_pos(isinst_pos);
                next_arm_jump = Some(self.emit_jump(OpCode::JumpIfFalse));

                if self.eat_if(TokenType::As) {
                    let n = self.advance_text();
                    self.store_name(n.clone());
                    as_name = Some(n);
                } else { self.chunk.emit(OpCode::PopTop, 0); }
            }
            self.eat(TokenType::Colon);
            self.compile_block();
            // Python implicitly unbinds the `except ... as e` name after the block.
            if let Some(n) = as_name {
                let idx = self.push_ssa_name(&n, self.current_version(&n));
                self.chunk.emit(OpCode::Del, idx);
            }

            // Handled arms jump past the `else` block, a bare last arm just falls through.
            let more = matches!(
                self.peek(),
                Some(TokenType::Except | TokenType::Else | TokenType::Finally)
            );
            if !had_bare || more {
                end_jumps.push(self.emit_jump(OpCode::Jump));
            }
        }

        if let Some(j) = next_arm_jump {
            self.patch(j);
            self.chunk.emit(OpCode::Raise, 0);
        } else if !had_except {
            // Pure try/finally, the handler just re-raises so the outer finally still runs.
            self.chunk.emit(OpCode::Raise, 0);
        }

        // Success path falls into `else`, handled-exception arms skip it.
        self.patch(success_jump);
        if self.eat_if(TokenType::Else) {
            self.eat(TokenType::Colon);
            self.compile_block();
        }
        for j in end_jumps {
            self.patch(j);
        }

        // Normal path pops the frame and marks the entry, unwind exits jump straight to the body.
        self.cleanup_count -= 1;
        self.chunk.emit(OpCode::PopExcept, 0);
        self.chunk.emit(OpCode::BeginFinally, 0);
        let fin_body = self.chunk.instructions.len() as u16;
        self.patch_to(fin_setup, fin_body);
        if self.eat_if(TokenType::Finally) {
            self.eat(TokenType::Colon);
            self.compile_block();
        }
        self.chunk.emit(OpCode::EndFinally, 0);

        self.commit_block();
    }

    /* with / async with, each CM is a SetupFinally cleanup frame whose handler stages `WithExit`. Dunders run as staged plain Calls so host deferrals can park. */

    pub(super) fn with_stmt_inner(&mut self, is_async: bool) {
        self.advance();
        let operand = is_async as u16;
        let mut setups: Vec<usize> = Vec::new();
        loop {
            self.expr();
            self.chunk.emit(OpCode::WithEnter, operand);
            self.chunk.emit(OpCode::Call, 1);
            if self.eat_if(TokenType::As) {
                let name = self.advance_text();
                self.store_name(name);
            } else {
                // Discard the unbound `__enter__` result.
                self.chunk.emit(OpCode::PopTop, 0);
            }
            setups.push(self.emit_jump(OpCode::SetupFinally));
            self.cleanup_count += 1;
            if !self.eat_if(TokenType::Comma) { break; }
        }
        self.eat(TokenType::Colon);
        self.compile_block();

        // Cleanups innermost-first, normal path pops the frame and marks the entry, then runs `__exit__`.
        for s in setups.into_iter().rev() {
            self.cleanup_count -= 1;
            self.chunk.emit(OpCode::PopExcept, 0);
            self.chunk.emit(OpCode::BeginFinally, 0);
            let h = self.chunk.instructions.len() as u16;
            self.patch_to(s, h);
            self.chunk.emit(OpCode::WithExit, operand);
            self.chunk.emit(OpCode::Call, 4);
            self.chunk.emit(OpCode::WithJudge, 0);
            self.chunk.emit(OpCode::EndFinally, 0);
        }
    }

}
