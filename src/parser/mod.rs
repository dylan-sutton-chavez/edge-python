pub(super) mod types;

mod stmt;
mod control;
mod expr;
mod literals;
mod imports;

pub use types::*;

use crate::s;
use crate::lexer::{Token, TokenType};
use crate::util::fx::FxHashMap as HashMap;
use crate::packages::{Resolver, NoopResolver};

use alloc::{boxed::Box, string::{String, ToString}, vec::Vec};
use core::iter::Peekable;

// Bracket table for either side: (opener, open str, close str).
#[inline]
pub(super) const fn bracket_pair(k: TokenType) -> (TokenType, &'static str, &'static str) {
    match k {
        TokenType::Lpar | TokenType::Rpar => (TokenType::Lpar, "'('", "')'"),
        TokenType::Lsqb | TokenType::Rsqb => (TokenType::Lsqb, "'['", "']'"),
        TokenType::Lbrace | TokenType::Rbrace => (TokenType::Lbrace, "'{'", "'}'"),
        _ => (k, "bracket", "bracket"),
    }
}
#[inline]
pub(super) const fn open_str(k: TokenType) -> &'static str { bracket_pair(k).1 }
#[inline]
pub(super) const fn close_str(k: TokenType) -> &'static str { bracket_pair(k).2 }
#[inline]
pub(super) const fn match_close_str(open: TokenType) -> &'static str { bracket_pair(open).2 }

// Call operand: high byte keyword count, low byte positional count.
#[inline]
pub(super) const fn pack_call(pos: u16, kw: u16) -> u16 {
    ((kw & 0xFF) << 8) | (pos & 0xFF)
}

// Shared spec -> compiled-chunk cache; a Vec (linear scan) avoids a hashbrown monomorphization.
pub(crate) type ModuleCache = alloc::rc::Rc<core::cell::RefCell<Vec<(String, alloc::rc::Rc<SSAChunk>)>>>;

pub struct Parser<'src, I: Iterator<Item = Token>> {
    pub(super) source: &'src str,
    pub(super) tokens: Peekable<I>,
    pub(super) chunk: SSAChunk,
    pub(super) ssa_versions: HashMap<String, u32>,
    /* Names declared `global` in the current function body; redirects load/store to `self.globals`. */
    pub(super) globals_decl: crate::util::fx::FxHashSet<String>,
    pub(super) join_stack: Vec<JoinNode>,
    pub(super) loop_starts: Vec<u16>,
    pub(super) last_line: usize,
    /* Last token's end offset; anchors diagnostics when `peek()` already skipped a Newline. */
    pub(super) last_end: usize,
    pub(super) loop_breaks: Vec<Vec<usize>>,
    // `true=for` (PopIter on break), false=while; parallels loop_starts/loop_breaks.
    pub(super) loop_kinds: Vec<bool>,
    /* Open finally/with cleanup blocks in the current function; break/continue cross these. */
    pub(super) cleanup_count: usize,
    // `cleanup_count` snapshot at each loop's entry; parallels loop_starts.
    pub(super) loop_cleanup_base: Vec<usize>,
    pub(super) expr_depth: usize,
    pub(super) saw_newline: bool,
    /* True inside f-string brace expr; disables `=` assignment so `f"{x=}"` parses as debug form. */
    pub(super) in_fstring_expr: bool,
    /* True while parsing unpack-target elements; disables `=` assignment inside expressions. */
    pub(super) in_target_list: bool,
    /* Unclosed brackets with error count at open; anchors "never closed" and drops cascade errors. */
    pub(super) bracket_stack: Vec<(TokenType, usize, usize, usize)>,
    pub errors: Vec<Diagnostic>,
    /* Host resolver; defaults to NoopResolver so import-free call sites work unchanged. */
    pub(super) resolver: Box<dyn Resolver>,
    // Shared module cache; sub-parsers inherit the Rc so each spec parses once.
    pub(super) module_cache: ModuleCache,
}

// SSA versioning helpers: version tracking and suffixed name emission.

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub(super) fn current_version(&self, name: &str) -> u32 {
        self.ssa_versions.get(name).copied().unwrap_or(0)
    }

    pub(super) fn ssa_name<'a>(name: &str, ver: u32, buf: &'a mut [u8; 128]) -> &'a str {
        let name_bytes = name.as_bytes();
        let cap = buf.len();
        let mut n = name_bytes.len().min(cap);
        buf[..n].copy_from_slice(&name_bytes[..n]);
        if n < cap {
            buf[n] = b'_';
            n += 1;
        }
        let mut tmp = itoa::Buffer::new();
        let s = tmp.format(ver).as_bytes();
        let take = s.len().min(cap - n);
        buf[n..n + take].copy_from_slice(&s[..take]);
        n += take;
        unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
    }

    pub(super) fn increment_version(&mut self, name: &str) -> u32 {
        let cur = self.current_version(name);
        let new = cur + 1;
        self.ssa_versions.insert(name.to_string(), new);
        new
    }

    pub(super) fn push_ssa_name(&mut self, name: &str, ver: u32) -> u16 {
        let mut buf = [0u8; 128];
        self.chunk.push_name(Self::ssa_name(name, ver, &mut buf))
    }

    pub(super) fn emit_load_ssa(&mut self, name: String) {
        if self.globals_decl.contains(&name) {
            let i = self.chunk.push_name(&name);
            self.chunk.emit(OpCode::LoadGlobal, i);
            return;
        }
        let i = self.push_ssa_name(&name, self.current_version(&name));
        self.chunk.emit(OpCode::LoadName, i);
    }

    pub(super) fn emit_const(&mut self, v: Value) {
        let i = self.chunk.push_const(v);
        self.chunk.emit(OpCode::LoadConst, i);
    }

    /* Emit a jump with a placeholder operand; returns its instruction index for later patching. */
    pub(super) fn emit_jump(&mut self, op: OpCode) -> usize {
        self.chunk.emit(op, 0);
        self.chunk.instructions.len() - 1
    }

    pub(super) fn store_name(&mut self, name: String) {
        if self.globals_decl.contains(&name) {
            let i = self.chunk.push_name(&name);
            self.chunk.emit(OpCode::StoreGlobal, i);
            return;
        }
        let ver = self.increment_version(&name);
        let i = self.push_ssa_name(&name, ver);
        self.chunk.emit(OpCode::StoreName, i);
    }

    /* Emit UnpackEx (starred) or UnpackSequence then store each target; a leading `*` on a starred name is stripped. */
    pub(super) fn emit_unpack_stores(&mut self, targets: &[String], star_pos: Option<usize>) {
        if let Some(sp) = star_pos {
            let before = sp as u16;
            let after = (targets.len() - sp - 1) as u16;
            self.chunk.emit(OpCode::UnpackEx, (before << 8) | after);
        } else {
            self.chunk.emit(OpCode::UnpackSequence, targets.len() as u16);
        }
        for target in targets { self.store_name(target.trim_start_matches('*').to_string()); }
    }

    /* Propagate the body's free names to the parent chunk, register the function, and emit its make-op. `fname` is None for lambdas (anonymous, sentinel slot); Some(name) registers a store slot for a def. Order matches the historic def path: free-name propagation, then slot, then push. */
    pub(super) fn push_function(&mut self, params: Vec<String>, body: SSAChunk, defaults: u16, fname: Option<&str>, op: OpCode) {
        let param_slots: crate::util::fx::FxHashSet<String> = params.iter()
            .map(|p| s!(str types::param_base_name(p), "_0")).collect();
        for name in &body.names {
            if !param_slots.contains(name.as_str()) {
                self.chunk.push_name(name);
            }
        }
        let fi = self.chunk.functions.len() as u16;
        let name_slot = match fname {
            Some(f) => self.push_ssa_name(f, self.current_version(f) + 1),
            None => u16::MAX,
        };
        self.chunk.functions.push((params, body, defaults, name_slot));
        self.chunk.emit(op, fi);
    }

    /* Raw version-bump + StoreName; returns the slot. NOT `store_name`: deliberately skips `globals_decl` routing. */
    pub(super) fn emit_store_new(&mut self, name: &str) -> u16 {
        let ver = self.increment_version(name);
        let i = self.push_ssa_name(name, ver);
        self.chunk.emit(OpCode::StoreName, i);
        i
    }

    pub(super) fn with_fresh_chunk(&mut self, f: impl FnOnce(&mut Self)) -> SSAChunk {
        let saved_chunk = core::mem::take(&mut self.chunk);
        let saved_ver = self.ssa_versions.clone();
        let saved_globals = core::mem::take(&mut self.globals_decl);
        // Nested body owns its loops and block stack; isolate, then restore the enclosing ones.
        let saved_loops = (
            core::mem::take(&mut self.loop_starts),
            core::mem::take(&mut self.loop_breaks),
            core::mem::take(&mut self.loop_kinds),
            core::mem::take(&mut self.loop_cleanup_base),
            core::mem::replace(&mut self.cleanup_count, 0),
        );
        // Copy parent externs so nested def bodies can call imported natives; extras don't leak up.
        self.chunk.extern_table = saved_chunk.extern_table.clone();
        self.chunk.extern_index = saved_chunk.extern_index.clone();
        // Inherit source/path for consistent traceback file context.
        self.chunk.source = saved_chunk.source.clone();
        self.chunk.path = saved_chunk.path.clone();
        f(self);
        let body = core::mem::take(&mut self.chunk);
        self.chunk = saved_chunk;
        self.ssa_versions = saved_ver;
        self.globals_decl = saved_globals;
        (self.loop_starts, self.loop_breaks, self.loop_kinds,
         self.loop_cleanup_base, self.cleanup_count) = saved_loops;
        body
    }
}

// SSA join points: Phi emission at control-flow merges.

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub(super) fn enter_block(&mut self) {
        self.join_stack.push(JoinNode {
            backup: self.ssa_versions.clone(),
            then: None,
        });
    }

    pub(super) fn mid_block(&mut self) {
        let Some(j) = self.join_stack.last_mut() else { return };
        // Save then-branch versions before restoring else baseline.
        j.then = Some(self.ssa_versions.clone());
        let mut restored = j.backup.clone();
        for (name, &v) in &self.ssa_versions {
            let e = restored.entry(name.clone()).or_insert(0);
            *e = (*e).max(v);
        }
        self.ssa_versions = restored;
    }

    pub(super) fn commit_block(&mut self) {
        let Some(j) = self.join_stack.pop() else { return };
        let post = self.ssa_versions.clone();

        let (a, b) = match j.then {
            Some(t) => (t, post),
            None => (post, j.backup.clone()),
        };

        let mut divergent: Vec<&String> = a
            .keys()
            .chain(b.keys())
            .filter(|name| a.get(*name).unwrap_or(&0) != b.get(*name).unwrap_or(&0))
            .collect();

        // Sort for deterministic Phi order; dedup chain() duplicates from shared vars.
        divergent.sort();
        divergent.dedup();

        for name in divergent {
            let va = *a.get(name).unwrap_or(&0);
            let vb = *b.get(name).unwrap_or(&0);
            let mut ba = [0u8; 128];
            let mut bb = [0u8; 128];
            let mut bx = [0u8; 128];
            let ia = self.chunk.push_name(Self::ssa_name(name, va, &mut ba));
            let ib = self.chunk.push_name(Self::ssa_name(name, vb, &mut bb));
            let v = self.increment_version(name);
            let ix = self.chunk.push_name(Self::ssa_name(name, v, &mut bx));

            self.chunk.phi_sources.push((ia, ib));
            self.chunk.emit(OpCode::Phi, ix);
        }
    }
}

// Token-stream utilities: advance, peek, eat, diagnostics.

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    /* Advance without bracket_stack tracking; for recovery consumers with their own balance bookkeeping. */
    pub(super) fn advance_raw(&mut self) -> Token {
        let tok = self.tokens.next().unwrap_or(Token {
            kind: TokenType::Endmarker,
            line: 0, start: 0, end: 0,
        });
        self.last_line = tok.line;
        if tok.end > 0 { self.last_end = tok.end; }
        tok
    }

    pub(super) fn advance(&mut self) -> Token {
        let tok = self.tokens.next().unwrap_or(Token {
            kind: TokenType::Endmarker,
            line: 0, start: 0, end: 0,
        });
        self.last_line = tok.line;
        if tok.end > 0 { self.last_end = tok.end; }
        match tok.kind {
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace => {
                self.bracket_stack.push((tok.kind, tok.start, tok.end, self.errors.len()));
            }
            TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace => {
                let want_open = bracket_pair(tok.kind).0;
                /* Find matching opener; report unclosed brackets above it; handle orphan closers. */
                if let Some(idx) = self.bracket_stack.iter().rposition(|&(k, _, _, _)| k == want_open) {
                    while self.bracket_stack.len() > idx + 1 {
                        let (k, st, en, ov) = self.bracket_stack.pop().unwrap();
                        self.errors.truncate(ov);
                        self.errors.push(Diagnostic {
                            start: st, end: en,
                            msg: s!(str open_str(k), " was never closed"),
                        });
                    }
                    self.bracket_stack.pop();
                } else if let Some(&(top_k, _, _, _)) = self.bracket_stack.last() {
                    self.errors.push(Diagnostic {
                        start: tok.start, end: tok.end,
                        msg: s!(str close_str(tok.kind), " does not match ", str open_str(top_k), ", expected ", str match_close_str(top_k)),
                    });
                    self.bracket_stack.pop();
                } else {
                    self.errors.push(Diagnostic {
                        start: tok.start, end: tok.end,
                        msg: s!("unexpected ", str close_str(tok.kind), ", no matching opener"),
                    });
                }
            }
            _ => {}
        }
        tok
    }

    /* Non-syncing diagnostic at peek; used when flow must continue (e.g. missing class name). */
    pub(super) fn diag_at_peek(&mut self, msg: &str) {
        let n = self.source.len();
        let (start, end) = match self.tokens.peek() {
            Some(t) if t.line > self.last_line && self.last_end > 0 => (self.last_end, self.last_end),
            Some(t) => (t.start, t.end),
            None => (n, n),
        };
        self.errors.push(Diagnostic { start, end, msg: msg.to_string() });
    }

    /* Diagnostic at next token + panic-mode sync to next statement boundary. */
    pub(super) fn error(&mut self, msg: &str) {
        let n = self.source.len();
        let (start, end) = self.tokens.peek().map(|t| (t.start, t.end)).unwrap_or((n, n));
        self.error_at(start, end, msg);
    }

    /* Diagnostic at caller span + panic-mode sync; stops at statement boundary or bracket close. */
    pub(super) fn error_at(&mut self, start: usize, end: usize, msg: &str) {
        self.errors.push(Diagnostic { start, end, msg: msg.to_string() });
        let in_brackets = !self.bracket_stack.is_empty();
        let mut depth: i32 = 0;
        loop {
            let kind = self.tokens.peek().map(|t| t.kind);
            match kind {
                None | Some(TokenType::Newline | TokenType::Dedent | TokenType::Endmarker) => break,
                Some(TokenType::Comma) if in_brackets && depth == 0 => break,
                Some(TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace) => {
                    if in_brackets && depth == 0 { break; }
                    depth -= 1;
                    self.tokens.next();
                }
                Some(TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace) => {
                    depth += 1;
                    self.tokens.next();
                }
                _ => { self.tokens.next(); }
            }
        }
    }

    pub(super) fn at_end(&mut self) -> bool { self.peek().is_none() }

    pub(super) fn lexeme(&self, t: &Token) -> &'src str { &self.source[t.start..t.end] }

    /* Consume the next token and return its source text as a String. */
    pub(super) fn advance_text(&mut self) -> String {
        let t = self.advance();
        self.lexeme(&t).to_string()
    }

    /* Skips Newline/Nl/Comment; maps Endmarker->None; latches saw_newline for ternary detection. */
    pub(super) fn peek(&mut self) -> Option<TokenType> {
        loop {
            match self.tokens.peek().map(|t| t.kind) {
                Some(TokenType::Newline) => {
                    self.saw_newline = true;
                    self.tokens.next();
                }
                Some(TokenType::Nl | TokenType::Comment) => { self.tokens.next(); }
                Some(TokenType::Endmarker) | None => return None,
                Some(k) => return Some(k),
            }
        }
    }

    /* Like `peek` but stops at the logical-line boundary: a `Newline` yields `None` (unconsumed) so postfix trailers don't bind across statements. `Nl`/`Comment` (bracket-internal) still skip, so multiline `()`/`[]`/`{}` are unaffected. */
    pub(super) fn peek_same_line(&mut self) -> Option<TokenType> {
        loop {
            match self.tokens.peek().map(|t| t.kind) {
                Some(TokenType::Nl | TokenType::Comment) => { self.tokens.next(); }
                Some(TokenType::Newline) | Some(TokenType::Endmarker) | None => return None,
                Some(k) => return Some(k),
            }
        }
    }

    pub(super) fn patch(&mut self, pos: usize) {
        let target = self.chunk.instructions.len() as u16;
        // Error recovery can leave a stale placeholder; skip if it was never emitted.
        if let Some(ins) = self.chunk.instructions.get_mut(pos) { ins.operand = target; }
    }

    /* Patch a jump to an explicit target; bounds-safe so instruction overflow (`emit` freezes at MAX_INSTRUCTIONS) can't panic on a stale index. */
    pub(super) fn patch_to(&mut self, pos: usize, target: u16) {
        if let Some(ins) = self.chunk.instructions.get_mut(pos) { ins.operand = target; }
    }

    /* Consumes kind or emits diagnostic; for missing closers anchors at opener and drops cascade. */
    pub(super) fn eat(&mut self, kind: TokenType) {
        if matches!(self.peek(), Some(k) if k == kind) {
            self.advance();
            return;
        }
        if matches!(kind, TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace) {
            let peeked = self.tokens.peek().map(|t| t.kind);
            // Wrong closer: bail silently; advance() will report the mismatch.
            if matches!(peeked, Some(TokenType::Rpar | TokenType::Rsqb | TokenType::Rbrace)) && peeked != Some(kind)
            {
                return;
            }
            // Missing closer: anchor diagnostic at the opener, not at the next-line token.
            let want_open = bracket_pair(kind).0;
            if let Some(&(opener_kind, start, end, errors_at_open)) = self.bracket_stack.last()
                && opener_kind == want_open
            {
                self.errors.truncate(errors_at_open);
                self.errors.push(Diagnostic {
                    start, end,
                    msg: s!(str open_str(opener_kind), " was never closed"),
                });
                self.bracket_stack.pop();
                return;
            }
        }
        let label: alloc::string::String = match self.tokens.peek() {
            Some(t) if t.kind == TokenType::Endmarker => "EOF".to_string(),
            Some(t) if t.kind == TokenType::Newline || t.kind == TokenType::Nl => "newline".to_string(),
            Some(t) if t.kind == TokenType::Indent => "indent".to_string(),
            Some(t) if t.kind == TokenType::Dedent => "dedent".to_string(),
            Some(t) if t.start == t.end => t.kind.as_str().to_string(),
            Some(t) => {
                let mut s = alloc::string::String::with_capacity(t.end - t.start + 2);
                s.push('\''); s.push_str(&self.source[t.start..t.end]); s.push('\''); s
            }
            None => "EOF".to_string(),
        };
        let msg = s!("expected ", str kind.as_str(), ", got ", str &label);
        // Token on later line: anchor at end-of-prev-line (common case: missing `:` in header).
        if let Some(t) = self.tokens.peek()
            && t.line > self.last_line
            && self.last_end > 0
        {
            let p = self.last_end;
            self.error_at(p, p, &msg);
        } else {
            self.error(&msg);
        }
    }

    pub(super) fn eat_if(&mut self, kind: TokenType) -> bool {
        if matches!(self.peek(), Some(k) if k == kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    // Comma list with forward-progress guard.
    pub(super) fn comma_list(&mut self, is_end: impl Fn(TokenType) -> bool, mut elem: impl FnMut(&mut Self)) {
        while !matches!(self.peek(), Some(t) if is_end(t)) && self.peek().is_some() {
            let progress = self.last_end;
            elem(self);
            self.eat_if(TokenType::Comma);
            if self.last_end == progress { break; }
        }
    }
}

// Parser constructors and parse entry point.

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub fn new(source: &'src str, iter: I) -> Self {
        Self::with_resolver(source, iter, Box::new(NoopResolver))
    }

    /* Parser with explicit resolver; required for `from X import` support. */
    pub fn with_resolver(source: &'src str, iter: I, resolver: Box<dyn Resolver>) -> Self {
        Self::with_shared_cache(source, iter, resolver,
            alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new())))
    }

    /* Shared-cache constructor; sub-parsers inherit it so each spec parses once. */
    pub(crate) fn with_shared_cache(source: &'src str,iter: I,resolver: Box<dyn Resolver>, module_cache: ModuleCache) -> Self {
        let chunk = SSAChunk {
            source: alloc::sync::Arc::new(source.into()),
            ..Default::default()
        };
        Self {
            source,
            tokens: iter.peekable(),
            chunk,
            ssa_versions: HashMap::default(),
            globals_decl: crate::util::fx::FxHashSet::default(),
            join_stack: Vec::new(),
            loop_starts: Vec::new(),
            loop_breaks: Vec::new(),
            loop_kinds: Vec::new(),
            cleanup_count: 0,
            loop_cleanup_base: Vec::new(),
            saw_newline: false,
            in_fstring_expr: false,
            in_target_list: false,
            expr_depth: 0,
            last_line: 0,
            last_end: 0,
            bracket_stack: Vec::new(),
            errors: Vec::new(),
            resolver,
            module_cache,
        }
    }

    pub fn parse(mut self) -> (SSAChunk, Vec<Diagnostic>) {
        while !self.at_end() {
            while self.eat_if(TokenType::Semi) {}
            if self.at_end() { break; }

            let produced_value = self.stmt();
            // Pop expression-statement results; chunk's implicit ReturnValue expects empty stack.
            if produced_value { self.chunk.emit(OpCode::PopTop, 0); }
        }

        if self.chunk.overflow {
            let n = self.source.len();
            self.errors.push(Diagnostic {
                start: n, end: n,
                msg: "program too large: exceeded maximum instruction limit".to_string(),
            });
        }

        if !self.errors.is_empty() {
            // Clear bytecode state so `finalize_prev_slots` doesn't use stale phi sources.
            self.chunk.instructions.clear();
            self.chunk.constants.clear();
            self.chunk.names.clear();
            self.chunk.phi_sources.clear();
            self.chunk.phi_map.clear();
            self.chunk.functions.clear();
            self.chunk.classes.clear();
            self.chunk.name_index.clear();
            self.chunk.nonlocals.clear();
            self.chunk.stmt_pos.clear();
            self.loop_kinds.clear();
        }

        self.chunk.emit(OpCode::ReturnValue, 0);
        self.chunk.finalize_prev_slots();
        (self.chunk, self.errors)
    }
}
