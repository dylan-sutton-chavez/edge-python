/* 
Compile-time import: Native binds to extern_table (CallExtern); Code inlines MakeFunction+StoreName. 
*/

use crate::s;

use super::Parser;
use super::types::{Diagnostic, ImportEntry, ImportKind, NativeClassEntry, OpCode, SSAChunk, parse_string, ssa_strip};
use crate::lexer::{Token, TokenType, lex};
use crate::packages::{Resolved, binding_to_extern, parse_integrity};
use crate::util::sha256::{sha256, hex_encode};
use crate::util::hash::FxHashSet;

use alloc::{string::{String, ToString}, vec::Vec};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /* `import name [as alias]`: resolves and binds module as HeapObj::Module under alias. Compile-time only, no import opcodes reach the VM. */
    pub(super) fn import_stmt(&mut self) {
        self.advance(); // 'import'
        loop {
            let (spec, span) = self.read_module_spec();
            let alias = if self.eat_if(TokenType::As) {
                self.advance_text()
            } else {
                spec.split('.').next().unwrap_or(&spec).to_string()
            };
            self.resolve_and_bind_all(&spec, span, &alias);
            if !self.eat_if(TokenType::Comma) { break; }
        }
    }

    /* `from <spec> import names|*`: spec is URL, path, or bare name; `*` binds all exports. Names may be parenthesized for multi-line lists, trailing comma allowed. */
    pub(super) fn parse_from_stmt(&mut self) {
        self.advance(); // 'from'
        let (spec, spec_span) = self.read_module_spec();
        self.eat(TokenType::Import);

        if self.eat_if(TokenType::Star) {
            self.resolve_and_bind_star(&spec, spec_span);
            return;
        }

        let parens = self.eat_if(TokenType::Lpar);

        let mut names: Vec<(String, String)> = Vec::new();
        loop {
            // Peek flushes Nl/Comment inside parens and lets a trailing `,` end the list.
            if parens && matches!(self.peek(), Some(TokenType::Rpar)) { break; }
            let name = self.advance_text();
            let alias = if self.eat_if(TokenType::As) { self.advance_text() } else { name.clone() };
            names.push((name, alias));
            if !self.eat_if(TokenType::Comma) { break; }
        }

        if parens { self.eat(TokenType::Rpar); }

        self.resolve_and_bind_named(&spec, spec_span, names);
    }

    /* Reads a quoted or dotted spec; returns (spec, span) for diagnostics. */
    fn read_module_spec(&mut self) -> (String, (usize, usize)) {
        if matches!(self.peek(), Some(TokenType::String)) {
            let t = self.advance();
            let raw = self.lexeme(&t).to_string();
            let unquoted = parse_string(&raw);
            (unquoted, (t.start, t.end))
        } else {
            let first = self.advance();
            let mut name = self.lexeme(&first).to_string();
            let mut span = (first.start, first.end);
            while self.eat_if(TokenType::Dot) {
                let next = self.advance();
                name.push('.');
                name.push_str(self.lexeme(&next));
                span.1 = next.end;
            }
            (name, span)
        }
    }

    /* Verifies #sha256 fragment then resolves; parser re-hashes bytes as defence-in-depth. Returns clean URL. */
    fn resolve_with_integrity(&mut self, spec: &str) -> Result<(String, Resolved), String> {
        let (url, expected) = parse_integrity(spec)?;
        if let Some(hash) = expected {
            // Host verifies its side; parser re-hashes as defence-in-depth.
            let bytes = self.resolver.fetch_bytes(url, Some(hash))?;
            let computed = sha256(&bytes);
            if computed != hash {
                return Err(s!("integrity check failed for '", str url, "'\n expected sha256-", str &hex_encode(&hash), "\n got sha256-", str &hex_encode(&computed)));
            }
        }
        let resolved = self.resolver.resolve(url)?;
        Ok((url.to_string(), resolved))
    }

    /* Parses or returns cached SSAChunk. Only path/URL specs cached; bare names skipped to avoid cross-manifest collisions. */
    fn parse_or_get_cached(&mut self, spec: &str, src: &str, span: (usize, usize)) -> Option<alloc::rc::Rc<SSAChunk>> {
        let cache_safe = spec.contains('/') || spec.contains("://");
        if cache_safe
            && let Some(cached) = self.module_cache.borrow().iter().find(|(k, _)| k == spec).map(|(_, c)| c.clone())
        {
            return Some(cached);
        }
        let owned = src.to_string();
        let (tokens, lex_errs) = lex(&owned);
        let mut sub_parser = Parser::with_shared_cache(
            &owned, tokens.into_iter(),
            self.resolver.child(spec),
            self.module_cache.clone(),
        );
        // Set path so tracebacks show the module file, not '<module>'.
        sub_parser.chunk.path = alloc::sync::Arc::new(spec.to_string());
        for e in lex_errs {
            sub_parser.errors.push(Diagnostic {
                start: e.start, end: e.end, msg: e.msg.to_string(),
            });
        }
        let (sub, errs) = sub_parser.parse();
        if !errs.is_empty() {
            self.error_at(span.0, span.1,
                &s!("module '", str spec, "' parse error: ", str &errs[0].msg));
            return None;
        }
        let rc = alloc::rc::Rc::new(sub);
        if cache_safe {
            self.module_cache.borrow_mut().push((spec.to_string(), rc.clone()));
        }
        Some(rc)
    }

    /* Registers import deduped by spec; returns LoadModule operand index. */
    fn register_import(&mut self, spec: &str, kind: ImportKind) -> u16 {
        if let Some(i) = self.chunk.imports.iter().position(|e| e.spec == spec) {
            return i as u16;
        }
        let i = self.chunk.imports.len() as u16;
        self.chunk.imports.push(ImportEntry {
            spec: spec.to_string(),
            kind,
        });
        i
    }

    /* Collects public top-level names from StoreName/MakeFunction; used by import-star. */
    fn module_public_exports(sub: &SSAChunk) -> Vec<String> {
        let mut exports: Vec<String> = Vec::new();
        let mut seen: FxHashSet<String> = FxHashSet::default();
        for ins in &sub.instructions {
            let slot_idx = match ins.opcode {
                OpCode::StoreName => Some(ins.operand as usize),
                OpCode::MakeFunction | OpCode::MakeCoroutine => sub.functions
                    .get(ins.operand as usize)
                    .map(|f| f.3 as usize),
                _ => None,
            };
            let Some(s) = slot_idx else { continue };
            let Some(name) = sub.names.get(s) else { continue };
            let bare = ssa_strip(name).to_string();
            if bare.starts_with('_') { continue; }
            if seen.insert(bare.clone()) { exports.push(bare); }
        }
        exports
    }

    /* Build a Native ImportKind from resolved bindings/classes/consts. */
    fn native_import_kind(bindings: &[crate::packages::NativeBinding], classes: &[crate::packages::NativeClass], consts: &[crate::packages::NativeBinding]) -> ImportKind {
        ImportKind::Native {
            funcs: bindings.iter().map(binding_to_extern).collect(),
            classes: classes.iter().map(|c| NativeClassEntry {
                name: c.name.clone(),
                methods: c.methods.iter().map(binding_to_extern).collect(),
            }).collect(),
            consts: consts.iter().map(binding_to_extern).collect(),
        }
    }

    /* Emit `LoadModule + LoadAttr(name) + StoreName(alias)`: bind a module attribute under `alias`. */
    fn bind_module_attr(&mut self, import_idx: u16, name: &str, alias: &str) {
        self.chunk.emit(OpCode::LoadModule, import_idx);
        let attr_idx = self.chunk.push_name(name);
        self.chunk.emit(OpCode::LoadAttr, attr_idx);
        self.emit_store_new(alias);
    }

    /* Named import: registers module, emits LoadModule+LoadAttr+StoreName; Native also populates extern_table for functions. */
    fn resolve_and_bind_named(&mut self, spec: &str, span: (usize, usize), names: Vec<(String, String)>) {
        let (_url, resolved) = match self.resolve_with_integrity(spec) {
            Ok(r) => r,
            Err(msg) => { self.error_at(span.0, span.1, &msg); return; }
        };
        let url = match &resolved {
            Resolved::Code { canonical, .. } => canonical.clone(),
            Resolved::Native { canonical, .. } => canonical.clone(),
        };
        match resolved {
            Resolved::Native { bindings, classes, consts, .. } => {
                // Register module first so LoadModule can target it for class and const imports.
                let import_idx = self.register_import(&url, Self::native_import_kind(&bindings, &classes, &consts));
                // Classes and consts bind via LoadModule+LoadAttr (module attr); functions take the extern_table fast path.
                for (name, alias) in &names {
                    if classes.iter().any(|c| c.name == *name) || consts.iter().any(|c| c.name == *name) {
                        self.bind_module_attr(import_idx, name, alias);
                        continue;
                    }
                    let Some(b) = bindings.iter().find(|b| b.name == *name) else {
                        self.error_at(span.0, span.1,
                            &s!("module '", str &url, "' has no export '", str name, "'"));
                        continue;
                    };
                    let idx = self.chunk.extern_table.len() as u16;
                    self.chunk.extern_table.push(binding_to_extern(b));
                    self.chunk.extern_index.insert(alias.clone(), idx);
                }
            }
            Resolved::Code { src, canonical } => {
                let Some(sub) = self.parse_or_get_cached(&canonical, &src, span) else { return; };
                let exports = Self::module_public_exports(&sub);
                let import_idx = self.register_import(&canonical, ImportKind::Code(sub));
                for (name, alias) in &names {
                    if !exports.iter().any(|e| e == name) {
                        let _ = url;
                        self.error_at(span.0, span.1,
                            &s!("module '", str &canonical, "' has no export '", str name, "'"));
                        continue;
                    }
                    self.bind_module_attr(import_idx, name, alias);
                }
            }
        }
    }

    /* `import X`: registers module, emits LoadModule+StoreName; VM builds a singleton Val at init. */
    fn resolve_and_bind_all(&mut self, spec: &str, span: (usize, usize), alias: &str) {
        let (_url, resolved) = match self.resolve_with_integrity(spec) {
            Ok(r) => r,
            Err(msg) => { self.error_at(span.0, span.1, &msg); return; }
        };
        let import_idx = match resolved {
            Resolved::Native { bindings, classes, consts, canonical } => {
                self.register_import(&canonical, Self::native_import_kind(&bindings, &classes, &consts))
            }
            Resolved::Code { src, canonical } => {
                let Some(sub) = self.parse_or_get_cached(&canonical, &src, span) else { return; };
                self.register_import(&canonical, ImportKind::Code(sub))
            }
        };
        self.chunk.emit(OpCode::LoadModule, import_idx);
        self.emit_store_new(alias);
    }

    /* Star import: Native fills extern_index; Code scans top-level and emits LoadModule+LoadAttr+StoreName per export. */
    fn resolve_and_bind_star(&mut self, spec: &str, span: (usize, usize)) {
        let (_url, resolved) = match self.resolve_with_integrity(spec) {
            Ok(r) => r,
            Err(msg) => { self.error_at(span.0, span.1, &msg); return; }
        };
        match resolved {
            Resolved::Native { bindings, classes, consts, canonical } => {
                for b in &bindings {
                    let idx = self.chunk.extern_table.len() as u16;
                    self.chunk.extern_table.push(binding_to_extern(b));
                    self.chunk.extern_index.insert(b.name.clone(), idx);
                }
                let import_idx = self.register_import(&canonical, Self::native_import_kind(&bindings, &classes, &consts));
                // Star import binds each export via LoadModule+LoadAttr+StoreName under its name.
                for name in bindings.iter().map(|b| &b.name)
                    .chain(classes.iter().map(|c| &c.name))
                    .chain(consts.iter().map(|c| &c.name)) {
                    self.bind_module_attr(import_idx, name, name);
                }
            }
            Resolved::Code { src, canonical } => {
                let Some(sub) = self.parse_or_get_cached(&canonical, &src, span) else { return; };
                let exports = Self::module_public_exports(&sub);
                let import_idx = self.register_import(&canonical, ImportKind::Code(sub));
                for name in &exports {
                    self.bind_module_attr(import_idx, name, name);
                }
            }
        }
    }

}
