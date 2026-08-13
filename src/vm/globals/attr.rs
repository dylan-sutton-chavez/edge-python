use crate::s;

use alloc::string::{String, ToString};

use super::super::VM;
use super::super::types::*;

impl<'a> VM<'a> {

    // `getattr(obj, name [, default])`.
    pub fn call_getattr(&mut self, op: u16) -> Result<(), VmErr> {
        if op != 2 && op != 3 {
            return Err(cold_type("getattr() takes 2 or 3 arguments"));
        }
        let default = if op == 3 { Some(self.pop()?) } else { None };
        let name = self.expect_str_arg("getattr() name must be a string")?;
        let obj = self.pop()?;

        // Instance attribute, instance dict first, then the user class chain (mirrors obj.name).
        let mut bind: Option<(Val, Val)> = None;
        if obj.is_heap() && let HeapObj::Instance(cls_val, attrs) = self.heap.get(obj) {
            let cls_val = *cls_val;
            let found = attrs.borrow().entries.iter()
                .find(|(k, _)| k.is_heap() && matches!(self.heap.get(*k), HeapObj::Str(s) if s.as_str() == name))
                .map(|(_, v)| *v);
            if let Some(v) = found { self.push(v); return Ok(()); }
            if let Some((mv, defining)) = self.lookup_class_member(cls_val, &name) {
                if mv.is_heap() && matches!(self.heap.get(mv), HeapObj::Func(..)) { bind = Some((mv, defining)); }
                else { self.push(mv); return Ok(()); }
            }
        }
        if let Some((func, defining)) = bind {
            let b = self.heap.alloc(HeapObj::BoundUserMethod(obj, func, defining))?;
            self.push(b); return Ok(());
        }

        // Class target, resolve a class attribute (incl. ones added via setattr / a decorator).
        if obj.is_heap() && matches!(self.heap.get(obj), HeapObj::Class(..))
            && let Some((v, _)) = self.lookup_class_member(obj, &name) {
            self.push(v);
            return Ok(());
        }
        let func_attr = if obj.is_heap() && let HeapObj::Func(_, _, _, attrs) = self.heap.get(obj) {
            attrs.borrow().iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
        } else { None };
        if let Some(v) = func_attr {
            self.push(v);
            return Ok(());
        }
        let bound_attr = if obj.is_heap() {
            match self.heap.get(obj) {
                HeapObj::BoundUserMethod(recv, func, _) => match name.as_str() {
                    "__self__" => Some(*recv),
                    "__func__" => Some(*func),
                    _ => None,
                },
                HeapObj::BoundMethod(recv, _) if name == "__self__" => Some(*recv),
                _ => None,
            }
        } else { None };
        if let Some(v) = bound_attr {
            self.push(v);
            return Ok(());
        }
        let ty = self.type_name(obj);
        if let Some(method_id) = crate::vm::methods::lookup_method(ty, &name) {
            let bound = self.heap.alloc(HeapObj::BoundMethod(obj, method_id))?;
            self.push(bound);
            return Ok(());
        }
        if let Some(d) = default {
            self.push(d);
            return Ok(());
        }
        Err(VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str &name, "'")))
    }

    // `hasattr(obj, name)`.
    pub fn call_hasattr(&mut self) -> Result<(), VmErr> {
        let name = self.expect_str_arg("hasattr() name must be a string")?;
        let obj = self.pop()?;
        // Instance attribute, instance dict or the user class chain.
        if obj.is_heap() && let HeapObj::Instance(cls_val, attrs) = self.heap.get(obj) {
            let cls_val = *cls_val;
            let in_dict = attrs.borrow().entries.iter()
                .any(|(k, _)| k.is_heap() && matches!(self.heap.get(*k), HeapObj::Str(s) if s.as_str() == name));
            if in_dict || self.lookup_class_member(cls_val, &name).is_some() {
                self.push(Val::bool(true)); return Ok(());
            }
        }
        let is_class_attr = obj.is_heap()
            && matches!(self.heap.get(obj), HeapObj::Class(..))
            && self.lookup_class_member(obj, &name).is_some();
        let is_func_attr = obj.is_heap()
            && matches!(self.heap.get(obj), HeapObj::Func(_, _, _, attrs) if attrs.borrow().iter().any(|(n, _)| *n == name));
        let is_bound_attr = obj.is_heap() && match self.heap.get(obj) {
            HeapObj::BoundUserMethod(..) => matches!(name.as_str(), "__self__" | "__func__"),
            HeapObj::BoundMethod(..) => name == "__self__",
            _ => false,
        };
        let ty = self.type_name(obj);
        let exists = is_class_attr || is_func_attr || is_bound_attr || crate::vm::methods::lookup_method(ty, &name).is_some();
        self.push(Val::bool(exists));
        Ok(())
    }

    /* `setattr(obj, name, value)`, mirrors `obj.name = value`. Instance-only because builtin types have no mutable attribute table. */
    pub fn call_setattr(&mut self) -> Result<(), VmErr> {
        let value = self.pop()?;
        let name = self.expect_str_arg("setattr() name must be a string")?;
        let obj = self.pop()?;
        // Class target, insert or replace in the mutable members store.
        if obj.is_heap() && let HeapObj::Class(_, _, members) = self.heap.get(obj) {
            set_member(members, &name, value);
            self.push(Val::none());
            return Ok(());
        }
        if obj.is_heap() && let HeapObj::Func(_, _, _, attrs) = self.heap.get(obj) {
            set_member(attrs, &name, value);
            self.push(Val::none());
            return Ok(());
        }
        if !obj.is_heap() || !matches!(self.heap.get(obj), HeapObj::Instance(..)) {
            return Err(cold_type("setattr() target must be an instance, class, or function"));
        }
        let key = self.heap.alloc(HeapObj::Str(name))?;
        if let HeapObj::Instance(_, attrs) = self.heap.get(obj) {
            attrs.borrow_mut().insert(key, value, &self.heap);
        }
        self.push(Val::none());
        Ok(())
    }

    /* `delattr(obj, name)`, remove an attribute from a user instance or class. */
    pub fn call_delattr(&mut self) -> Result<(), VmErr> {
        let name = self.expect_str_arg("delattr() name must be a string")?;
        let obj = self.pop()?;
        self.delete_attr_named(obj, &name)?;
        self.push(Val::none());
        Ok(())
    }

    /* `del obj.attr` opcode, pop the object, remove the named attribute in place. */
    pub fn exec_del_attr(&mut self, op: u16, chunk: &crate::parser::SSAChunk) -> Result<(), VmErr> {
        let obj = self.pop()?;
        let name = chunk.names.get(op as usize).ok_or(cold_runtime("DelAttr: bad name index"))?.clone();
        self.delete_attr_named(obj, &name)
    }

    /* Shared attribute removal for `delattr()` and `del obj.attr`, AttributeError when absent. */
    fn delete_attr_named(&mut self, obj: Val, name: &str) -> Result<(), VmErr> {
        if obj.is_heap() && matches!(self.heap.get(obj), HeapObj::Class(..)) {
            if let HeapObj::Class(_, _, members) = self.heap.get(obj) {
                members.borrow_mut().retain(|(n, _)| n != name);
            }
            return Ok(());
        }
        if obj.is_heap() && let HeapObj::Func(_, _, _, attrs) = self.heap.get(obj) {
            let attrs = attrs.clone();
            let had = attrs.borrow().iter().any(|(n, _)| n == name);
            if !had {
                return Err(VmErr::Attribute(s!("'function' object has no attribute '", str name, "'")));
            }
            attrs.borrow_mut().retain(|(n, _)| n != name);
            return Ok(());
        }
        if !obj.is_heap() || !matches!(self.heap.get(obj), HeapObj::Instance(..)) {
            return Err(cold_type("delattr() target must be an instance or class"));
        }
        // Strings <=128 bytes are interned, so re-alloc'ing yields the same Val key StoreAttr used.
        let key = self.heap.alloc(HeapObj::Str(name.to_string()))?;
        let existed = if let HeapObj::Instance(_, attrs) = self.heap.get(obj) {
            let had = attrs.borrow().entries.iter()
                .any(|(k, _)| k.is_heap() && matches!(self.heap.get(*k), HeapObj::Str(s) if s.as_str() == name));
            if had { attrs.borrow_mut().remove(&key, &self.heap); }
            had
        } else { false };
        // Deleting a missing attribute raises AttributeError, matching Python.
        if !existed {
            let ty = self.type_name(obj);
            return Err(VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str name, "'")));
        }
        Ok(())
    }

    // Returns v's String, or errors with `msg` when it isn't a heap string.
    pub(crate) fn str_of(&self, v: Val, msg: &'static str) -> Result<String, VmErr> {
        if v.is_heap() && let HeapObj::Str(s) = self.heap.get(v) { return Ok(s.clone()); }
        Err(cold_type(msg))
    }

    // Pops TOS and returns its String, or errors with `msg` if it isn't a heap string.
    fn expect_str_arg(&mut self, msg: &'static str) -> Result<String, VmErr> {
        let v = self.pop()?;
        self.str_of(v, msg)
    }

    /* `vars(obj)`, an Instance yields a copy of `__dict__`, a Module yields a dict from its attrs. No-arg form is unsupported, use `locals()`. */
    pub fn call_vars(&mut self) -> Result<(), VmErr> {
        use alloc::vec::Vec;
        let obj = self.pop()?;
        if !obj.is_heap() {
            return Err(cold_type("vars() requires an instance or module"));
        }
        // Two passes, drop the heap borrow before `alloc()`. Modules materialise names as `Vec<String>` first.
        enum Source { Instance(Vec<(Val, Val)>), Module(Vec<(String, Val)>) }
        let src = match self.heap.get(obj) {
            HeapObj::Instance(_, attrs) => Source::Instance(attrs.borrow().entries.clone()),
            HeapObj::Module(_, attrs) => Source::Module(attrs.clone()),
            _ => return Err(cold_type("vars() requires an instance or module")),
        };
        let entries: Vec<(Val, Val)> = match src {
            Source::Instance(e) => e,
            Source::Module(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (name, v) in items {
                    let key = self.heap.alloc(HeapObj::Str(name))?;
                    out.push((key, v));
                }
                out
            }
        };
        let mut dm = DictMap::with_capacity(entries.len());
        for (k, v) in entries { dm.insert(k, v, &self.heap); }
        self.alloc_and_push_dict(dm)
    }

    /* `globals()`, module-level bindings as a dict. User top-level names only (entry-chunk slots + module state), builtins live in a separate namespace, matching Python. Returned dict is a copy. */
    pub fn call_globals(&mut self, chunk: &crate::parser::SSAChunk, slots: &[Val]) -> Result<(), VmErr> {
        let mut out: crate::util::hash::FxHashMap<String, Val> = crate::util::hash::FxHashMap::default();
        // Inside a function, entry slots sit at the bottom of `live_slots`, at top-level, use `slots` as-is.
        let (entry_chunk, entry_slots): (&crate::parser::SSAChunk, &[Val]) =
            if core::ptr::eq(chunk as *const _, self.chunk as *const _) {
                (chunk, slots)
            } else {
                let n = self.chunk.names.len().min(self.live_slots.len());
                (self.chunk, &self.live_slots[..n])
            };
        for (i, name) in entry_chunk.names.iter().enumerate() {
            if name.starts_with('#') { continue; }
            let v = match entry_slots.get(i) {
                Some(v) if !v.is_undef() => *v,
                _ => continue,
            };
            let bare = crate::parser::ssa_strip(name).to_string();
            // User assignment overrides the builtin entry of the same name.
            out.insert(bare, v);
        }
        // Module state (user-mutated via `global` from inside functions) overrides entry-chunk snapshots.
        for (k, v) in self.module_state.iter() {
            out.insert(k.clone(), *v);
        }
        let mut dm = DictMap::with_capacity(out.len());
        for (k, v) in out {
            let key = self.heap.alloc(HeapObj::Str(k))?;
            dm.insert(key, v, &self.heap);
        }
        self.alloc_and_push_dict(dm)
    }

    /* `locals()`, frame bindings as a dict. Dedupes SSA versions (`x_0`, `x_1`, ...) to the highest live one. Filters synthetic `#`-slots and unrebound builtins (same Val as the global). */
    pub fn call_locals(&mut self, chunk: &crate::parser::SSAChunk, slots: &[Val]) -> Result<(), VmErr> {
        // Map bare-name -> (best version, val) so we keep only the latest.
        let mut latest: crate::util::hash::FxHashMap<String, (i64, Val)> = crate::util::hash::FxHashMap::default();
        for (i, name) in chunk.names.iter().enumerate() {
            let v = match slots.get(i) {
                Some(v) if !v.is_undef() => *v,
                _ => continue,
            };
            // Synthetic `#`-slots are matcher scratch, never user-visible.
            if name.starts_with('#') { continue; }
            // Strip SSA version suffix.
            let (bare, ver) = crate::parser::SsaName::parse_or_bare(name);
            let ver = ver as i64;
            // Skip unrebound builtins, same Val as the global means the user never assigned locally.
            if let Some(&gv) = self.globals.get(bare)
                && gv.0 == v.0 { continue; }
            let entry = latest.entry(bare.to_string()).or_insert((-1, Val::undef()));
            if ver > entry.0 { *entry = (ver, v); }
        }
        let mut dm = DictMap::with_capacity(latest.len());
        for (name, (_, v)) in latest {
            let key = self.heap.alloc(HeapObj::Str(name))?;
            dm.insert(key, v, &self.heap);
        }
        self.alloc_and_push_dict(dm)
    }
}
