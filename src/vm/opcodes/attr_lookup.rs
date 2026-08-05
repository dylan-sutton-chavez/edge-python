/*
Attribute resolution for `LoadAttr` / `CallMethod`. Built-in method bodies live in `vm/methods/`; this file owns `AttrLookup`, the resolver, and the `__getattr__` fallback.
*/

use super::*;
use crate::alloc::string::ToString;
use crate::s;

pub use crate::vm::methods::BuiltinMethodId;
use crate::vm::methods::lookup_method;

// `resolve_attr` result, every shape LoadAttr / CallMethod dispatches on.
pub(crate) enum AttrLookup {
    ModuleAttr(Val),
    ClassMember(Val),
    InstanceField(Val),
    // `class` is where `func` was found; the called frame needs it so `super()` knows where to resume.
    InstanceMethod { recv: Val, func: Val, class: Val },
    BuiltinMethod(BuiltinMethodId),
    // `e.args` on ExcInstance, caller picks: LoadAttr materialises the tuple, CallMethod errors.
    ExcArgs(Vec<Val>),
    // Property descriptor on an instance, `LoadAttr` invokes `getter(recv)`.
    PropertyGet { recv: Val, getter: Val },
    // `prop.setter` access, `LoadAttr` materialises a `PropertySetter` value bound to the source property.
    PropertySetterRef(Val),
    // `__name__` on a function, type, or class; `LoadAttr` materialises the str.
    Name(String),
}

impl<'a> VM<'a> {
    // The cached C3 linearization of `cls`, or `[cls]` when uncached (native classes, or an inconsistent hierarchy that `c3_merge` declined to cache).
    fn mro_of(&self, c: Val) -> alloc::vec::Vec<Val> {
        match self.mro_cache.get(&c.0) {
            Some(r) => (**r).clone(),
            None => alloc::vec![c],
        }
    }

    /* C3 merge of the bases' linearizations plus the bases list itself, the tail of `L[cls] = cls :: merge(...)`. `cls` is prepended by the caller (it isn't allocated yet at validation time). Errs on an inconsistent hierarchy, matching Python's `TypeError` at class creation. */
    pub(crate) fn c3_merge(&self, bases: &[Val]) -> Result<alloc::vec::Vec<Val>, VmErr> {
        let mut seqs: alloc::vec::Vec<alloc::vec::Vec<Val>> = bases.iter().map(|&b| self.mro_of(b)).collect();
        if !bases.is_empty() { seqs.push(bases.to_vec()); }
        let mut out = alloc::vec::Vec::new();
        loop {
            seqs.retain(|s| !s.is_empty());
            if seqs.is_empty() { break; }
            // A valid head appears in no sequence's tail; take the first such across sequences (C3 order).
            let mut head = None;
            for s in &seqs {
                let h = s[0];
                let in_tail = seqs.iter().any(|t| t.len() > 1 && t[1..].iter().any(|&x| x.0 == h.0));
                if !in_tail { head = Some(h); break; }
            }
            let Some(h) = head else {
                return Err(cold_type("Cannot create a consistent method resolution order (MRO) for bases"));
            };
            out.push(h);
            for s in &mut seqs { s.retain(|&x| x.0 != h.0); }
        }
        Ok(out)
    }

    /* Bind a resolved MRO member `mv` to `recv`: Property -> getter, staticmethod -> unbound, function -> descriptor-bound; plain data returned as-is. Guards is_heap before heap.get so a non-heap data member is never read as a pointer. */
    fn bind_member(&self, mv: Val, recv: Val, defining: Val) -> AttrLookup {
        if mv.is_heap() {
            match self.heap.get(mv) {
                HeapObj::Property(getter, _) => return AttrLookup::PropertyGet { recv, getter: *getter },
                HeapObj::StaticMethod(func) => return AttrLookup::ClassMember(*func),
                // Native-class methods take self as their first argument.
                HeapObj::Extern(_) => return AttrLookup::InstanceMethod { recv, func: mv, class: defining },
                HeapObj::ClassMethod(func) => {
                    // Bind the receiver's class, not the instance.
                    let cls = if recv.is_heap() && let HeapObj::Instance(c, _) = self.heap.get(recv) { *c } else { recv };
                    return AttrLookup::InstanceMethod { recv: cls, func: *func, class: defining };
                }
                HeapObj::Func(..) => return AttrLookup::InstanceMethod { recv, func: mv, class: defining },
                _ => {}
            }
        }
        AttrLookup::ClassMember(mv)
    }

    // Member lookup along the C3 MRO; first hit wins. Falls back to a direct-then-DFS walk for uncached classes (native classes have no bases, so DFS = own members). Returns `(value, defining_class)` so callers building `BoundUserMethod` / `InstanceMethod` record where the method came from for `super()`.
    pub(crate) fn lookup_class_member(&self, cls: Val, name: &str) -> Option<(Val, Val)> {
        if !cls.is_heap() { return None; }
        let HeapObj::Class(_, bases, members) = self.heap.get(cls) else { return None; };
        if let Some(mro) = self.mro_cache.get(&cls.0) {
            for &c in mro.iter() {
                if let HeapObj::Class(_, _, m) = self.heap.get(c)
                    && let Some(&(_, v)) = m.borrow().iter().find(|(n, _)| n == name) {
                        return Some((v, c));
                    }
            }
            return None;
        }
        if let Some(&(_, v)) = members.borrow().iter().find(|(n, _)| n == name) { return Some((v, cls)); }
        for &b in bases {
            if let Some(found) = self.lookup_class_member(b, name) { return Some(found); }
        }
        None
    }

    /* `super()` lookup: walk `derived`'s C3 MRO strictly past `after`, so a diamond resolves to the next class in the instance's linearization (not just `after`'s own bases). Falls back to a DFS over `after`'s bases when `derived` has no cached MRO. */
    pub(crate) fn lookup_class_member_after(&self, derived: Val, after: Val, name: &str) -> Option<(Val, Val)> {
        if let Some(mro) = self.mro_cache.get(&derived.0) {
            let mut past = false;
            for &c in mro.iter() {
                if past
                    && let HeapObj::Class(_, _, m) = self.heap.get(c)
                    && let Some(&(_, v)) = m.borrow().iter().find(|(n, _)| n == name) {
                        return Some((v, c));
                    }
                if c.0 == after.0 { past = true; }
            }
            return None;
        }
        // Fallback: search strictly above `after` via its own bases.
        if !after.is_heap() { return None; }
        let HeapObj::Class(_, bases, _) = self.heap.get(after) else { return None; };
        for &b in bases {
            if let Some(found) = self.lookup_class_member(b, name) { return Some(found); }
        }
        None
    }

    // `obj.<name>` resolution shared by `handle_load_attr` and `exec_call_method`.
    pub(crate) fn resolve_attr(&self, obj: Val, name: &str) -> Result<AttrLookup, VmErr> {
        let bare = crate::parser::ssa_strip(name);

        // Module attr: linear scan; the table is sized for around 30 entries.
        if obj.is_heap()
            && let HeapObj::Module(mod_name, attrs) = self.heap.get(obj) {
                if let Some((_, v)) = attrs.iter().find(|(n, _)| n == bare) {
                    return Ok(AttrLookup::ModuleAttr(*v));
                }
                return Err(VmErr::Attribute(s!("module '", str mod_name, "' has no attribute '", str bare, "'")));
            }

        // ExcInstance attr: only `e.args` is defined.
        if obj.is_heap()
            && let HeapObj::ExcInstance(_, args) = self.heap.get(obj) {
                if bare == "args" { return Ok(AttrLookup::ExcArgs(args.clone())); }
                let ty = self.type_name(obj);
                return Err(VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str bare, "'")));
            }

        // Bound methods expose their receiver; user methods also their function. Builtin bound methods have no `__func__`, like Python.
        if obj.is_heap() {
            match self.heap.get(obj) {
                HeapObj::BoundUserMethod(recv, func, _) => match bare {
                    "__self__" => return Ok(AttrLookup::ClassMember(*recv)),
                    "__func__" => return Ok(AttrLookup::ClassMember(*func)),
                    _ => {}
                },
                HeapObj::BoundMethod(recv, _) if bare == "__self__" => {
                    return Ok(AttrLookup::ClassMember(*recv));
                }
                _ => {}
            }
        }

        // Function attributes; a stored attr wins over the derived `__name__`.
        if obj.is_heap()
            && let HeapObj::Func(_, _, _, attrs) = self.heap.get(obj)
            && let Some(v) = attrs.borrow().iter().find(|(n, _)| n == bare).map(|(_, v)| *v)
        {
            return Ok(AttrLookup::ClassMember(v));
        }

        // `__name__` on callables and types resolves to their declared name.
        if obj.is_heap() && bare == "__name__" {
            let resolved = match self.heap.get(obj) {
                HeapObj::Func(fi, ..) => self.function_names.get(*fi).cloned(),
                HeapObj::Type(n) => Some(n.clone()),
                HeapObj::Class(n, _, _) => Some(n.clone()),
                _ => None,
            };
            if let Some(n) = resolved { return Ok(AttrLookup::Name(n)); }
        }

        // Class attr: `MyClass.method` returns the unbound function (no `self` prepended).
        if obj.is_heap()
            && let HeapObj::Class(cls_name, _, _) = self.heap.get(obj) {
                if let Some((v, defining)) = self.lookup_class_member(obj, bare) {
                    // `staticmethod` accessed on the class itself unwraps to the plain function.
                    if v.is_heap() && let HeapObj::StaticMethod(func) = self.heap.get(v) {
                        return Ok(AttrLookup::ClassMember(*func));
                    }
                    // `classmethod` binds the accessed class, derived included.
                    if v.is_heap() && let HeapObj::ClassMethod(func) = self.heap.get(v) {
                        return Ok(AttrLookup::InstanceMethod { recv: obj, func: *func, class: defining });
                    }
                    return Ok(AttrLookup::ClassMember(v));
                }
                let cls_name = cls_name.clone();
                return Err(VmErr::Attribute(s!("type object '", str &cls_name, "' has no attribute '", str bare, "'")));
            }

        // Instance attribute lookup: check `__dict__` first, then the class chain (direct + bases).
        if obj.is_heap()
            && let HeapObj::Instance(cls_val, attrs) = self.heap.get(obj) {
                let cls_val = *cls_val;
                let found = attrs.borrow().entries.iter()
                    .find(|(k, _)| k.is_heap() && matches!(self.heap.get(*k), HeapObj::Str(s) if s == name))
                    .map(|(_, v)| *v);
                if let Some(v) = found { return Ok(AttrLookup::InstanceField(v)); }
                if let Some((mv, defining)) = self.lookup_class_member(cls_val, bare) {
                    return Ok(self.bind_member(mv, obj, defining));
                }
                let ty = self.type_name(obj);
                return Err(VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str name, "'")));
            }

        // `super().<name>`: search strictly above the proxy's stored class; methods bind to the proxy's `recv`.
        if obj.is_heap()
            && let HeapObj::Super(cls_val, recv) = self.heap.get(obj) {
                let (cls_val, recv) = (*cls_val, *recv);
                // C3 super: walk the *instance type*'s MRO past the defining class, not just the defining class's bases.
                let derived = match self.heap.get(recv) {
                    HeapObj::Instance(c, _) => *c,
                    _ => cls_val,
                };
                if let Some((mv, defining)) = self.lookup_class_member_after(derived, cls_val, name) {
                    return Ok(self.bind_member(mv, recv, defining));
                }
                return Err(VmErr::Attribute(s!("'super' object has no attribute '", str name, "'")));
            }

        // `prop.setter` produces a callable that re-builds the property with a new setter (powers `@x.setter`).
        if obj.is_heap()
            && matches!(self.heap.get(obj), HeapObj::Property(..))
            && bare == "setter" {
                return Ok(AttrLookup::PropertySetterRef(obj));
            }

        // Builtin classmethods accessed on the type object (e.g. dict.fromkeys, bytes.fromhex, int.from_bytes): resolve under the type's own name rather than "type".
        if obj.is_heap()
            && let HeapObj::Type(n) = self.heap.get(obj)
            && matches!(bare, "fromkeys" | "fromhex" | "from_bytes") {
                let n = n.clone();
                if let Some(id) = lookup_method(&n, bare) { return Ok(AttrLookup::BuiltinMethod(id)); }
            }

        // Builtin type method.
        let ty = self.type_name(obj);
        lookup_method(ty, name)
            .map(AttrLookup::BuiltinMethod)
            .ok_or_else(|| VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str name, "'")))
    }

    /* instance fallback via `__getattr__(name)`. Called by `LoadAttr` / `CallMethod` after the normal lookup raises `AttributeError`. */
    pub(crate) fn try_getattr_fallback(&mut self, obj: Val, name: &str, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        if !obj.is_heap() || !matches!(self.heap.get(obj), HeapObj::Instance(..)) { return Ok(None); }
        let bare = crate::parser::ssa_strip(name);
        let name_val = self.heap.alloc(HeapObj::Str(bare.to_string()))?;
        self.try_call_dunder(obj, "__getattr__", &[name_val], chunk, slots)
    }

    pub(crate) fn handle_load_attr(&mut self, name_idx: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Borrow, don't clone: `chunk` outlives every `&mut self` call below.
        let name = chunk.names.get(name_idx as usize).ok_or(VmErr::Runtime("LoadAttr: bad name index"))?;
        let obj = self.pop()?;
        let lookup = match self.resolve_attr(obj, name) {
            Ok(l) => l,
            Err(VmErr::Attribute(msg)) => {
                if let Some(v) = self.try_getattr_fallback(obj, name, chunk, slots)? {
                    self.push(v);
                    return Ok(());
                }
                return Err(VmErr::Attribute(msg));
            }
            Err(other) => return Err(other),
        };
        match lookup {
            AttrLookup::ModuleAttr(v)
            | AttrLookup::ClassMember(v)
            | AttrLookup::InstanceField(v) => {
                self.push(v);
                Ok(())
            }
            AttrLookup::InstanceMethod { recv, func, class } => {
                let bound = self.heap.alloc(HeapObj::BoundUserMethod(recv, func, class))?;
                self.push(bound);
                Ok(())
            }
            AttrLookup::BuiltinMethod(id) => {
                let bound = self.heap.alloc(HeapObj::BoundMethod(obj, id))?;
                self.push(bound);
                Ok(())
            }
            AttrLookup::ExcArgs(args) => {
                let v = self.heap.alloc(HeapObj::Tuple(args))?;
                self.push(v);
                Ok(())
            }
            AttrLookup::PropertyGet { recv, getter } => {
                // Inline getter call: matches `BoundUserMethod` dispatch (push func, push self, call).
                if self.depth >= self.max_calls { return Err(cold_depth()); }
                self.push(getter);
                self.push(recv);
                self.exec_call(1, chunk, slots)
            }
            AttrLookup::PropertySetterRef(prop) => {
                let v = self.heap.alloc(HeapObj::PropertySetter(prop))?;
                self.push(v);
                Ok(())
            }
            AttrLookup::Name(s) => {
                let v = self.heap.alloc(HeapObj::Str(s))?;
                self.push(v);
                Ok(())
            }
        }
    }
}
