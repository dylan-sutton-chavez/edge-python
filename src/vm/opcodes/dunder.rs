use super::*;
use crate::alloc::string::ToString;

/* Single source of truth for opcode -> (forward, reflected) arithmetic dunder names. */
pub(crate) fn binary_dunder_names(op: OpCode) -> Option<(&'static str, &'static str)> {
    Some(match op {
        OpCode::Add => ("__add__", "__radd__"),
        OpCode::Sub => ("__sub__", "__rsub__"),
        OpCode::Mul => ("__mul__", "__rmul__"),
        OpCode::Div => ("__truediv__", "__rtruediv__"),
        OpCode::FloorDiv => ("__floordiv__", "__rfloordiv__"),
        OpCode::Mod => ("__mod__", "__rmod__"),
        OpCode::Pow => ("__pow__", "__rpow__"),
        OpCode::BitAnd => ("__and__", "__rand__"),
        OpCode::BitOr => ("__or__", "__ror__"),
        OpCode::BitXor => ("__xor__", "__rxor__"),
        OpCode::Shl => ("__lshift__", "__rlshift__"),
        OpCode::Shr => ("__rshift__", "__rrshift__"),
        _ => return None,
    })
}

/* Same for comparisons, (forward, reflected). `__eq__` reflects to itself. `<` reflects to `>` and vice-versa. */
pub(crate) fn compare_dunder_names(op: OpCode) -> Option<(&'static str, &'static str)> {
    Some(match op {
        OpCode::Eq => ("__eq__", "__eq__"),
        OpCode::NotEq => ("__ne__", "__ne__"),
        OpCode::Lt => ("__lt__", "__gt__"),
        OpCode::LtEq => ("__le__", "__ge__"),
        OpCode::Gt => ("__gt__", "__lt__"),
        OpCode::GtEq => ("__ge__", "__le__"),
        _ => return None,
    })
}

impl<'a> VM<'a> {
    /* `recv.<name>(*args)` probes the instance method and invokes it with `self` prepended. `Some(v)` on return, `None` on miss / `NotImplemented` (triggers reflected/fallback dispatch), `Err` only on a raised dunder. */
    pub(crate) fn try_call_dunder(&mut self, recv: Val, name: &str, args: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        // Built-in types route through their native handlers, and dunder dispatch only fires on user instances.
        if !recv.is_heap() { return Ok(None); }
        let HeapObj::Instance(cls_val, _) = self.heap.get(recv) else { return Ok(None); };
        let cls_val = *cls_val;

        // Special methods resolve on the type's MRO, bypassing the instance __dict__, like Python.
        let Some((func, class)) = self.lookup_class_member(cls_val, name) else { return Ok(None); };
        // Only plain functions bind as methods, so data attributes never dispatch implicitly.
        if !(func.is_heap() && matches!(self.heap.get(func), HeapObj::Func(..))) { return Ok(None); }

        // Mirror `__init__` dispatch, depth guard before pushing so a recursive blow-up leaves no half-built frame.
        if self.depth >= self.max_calls { return Err(cold_depth()); }

        self.pending.method_binding = Some((class, recv));
        self.push(func);
        self.push(recv);
        for &a in args { self.push(a); }
        let argc = (1 + args.len()) as u16;
        self.exec_call(argc, chunk, slots)?;

        let result = self.pop()?;
        if self.heap.is_not_implemented(result) { return Ok(None); }
        Ok(Some(result))
    }

    /* Class of an Instance, or `None` for built-in operands. Powers the subclass-first ordering rule. */
    fn instance_class(&self, v: Val) -> Option<Val> {
        if !v.is_heap() { return None; }
        match self.heap.get(v) { HeapObj::Instance(c, _) => Some(*c), _ => None }
    }

    /* Ordered forward/reflected dunder dispatch, reflected (`b.rname(a)`) runs first when `type(b)` strictly subclasses `type(a)` so overrides win. Returns the first non-None result. */
    fn dispatch_reflected(&mut self, a: Val, b: Val, lname: &str, rname: &str, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        let b_overrides = match (self.instance_class(a), self.instance_class(b)) {
            (Some(ac), Some(bc)) => ac.0 != bc.0 && self.heap.is_subclass(bc, ac),
            _ => false,
        };
        let calls: [(Val, &str, Val); 2] = if b_overrides {
            [(b, rname, a), (a, lname, b)]
        } else {
            [(a, lname, b), (b, rname, a)]
        };
        for (recv, name, arg) in calls {
            if let Some(r) = self.try_call_dunder(recv, name, &[arg], chunk, slots)? { return Ok(Some(r)); }
        }
        Ok(None)
    }

    /* Binary arithmetic dunder dispatch with Python's subclass-first ordering, if `type(b)` is a strict subclass of `type(a)` the reflected op runs first so overrides win. */
    pub(crate) fn try_binary_dunder(&mut self, op: OpCode, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        if self.instance_class(a).is_none() && self.instance_class(b).is_none() { return Ok(None); }
        let Some((lname, rname)) = binary_dunder_names(op) else { return Ok(None); };
        self.dispatch_reflected(a, b, lname, rname, chunk, slots)
    }

    /* Comparison dunder dispatch. `__eq__` reflects to itself. `__ne__` falls back to `not __eq__`. `<` reflects to `>` and vice-versa. */
    pub(crate) fn try_compare_dunder(&mut self, op: OpCode, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        if self.instance_class(a).is_none() && self.instance_class(b).is_none() { return Ok(None); }
        let Some((lname, rname)) = compare_dunder_names(op) else { return Ok(None); };

        let Some(r) = self.dispatch_reflected(a, b, lname, rname, chunk, slots)? else {
            // `!=` falls back to negated `__eq__` when `__ne__` is absent.
            if matches!(op, OpCode::NotEq)
                && let Some(eq) = self.try_compare_dunder(OpCode::Eq, a, b, chunk, slots)? {
                return Ok(Some(Val::bool(!self.truthy(eq))));
            }
            return Ok(None);
        };
        Ok(Some(r))
    }

    /* Python `bool()` semantics, try `__bool__`, then `__len__` (0 = False), else default True for instances. Pass-through for built-in types. */
    pub(crate) fn truthy_op(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if !v.is_heap() || !matches!(self.heap.get(v), HeapObj::Instance(..)) {
            return Ok(self.truthy(v));
        }
        if let Some(r) = self.try_call_dunder(v, "__bool__", &[], chunk, slots)? {
            if !matches!(r, x if x.is_bool()) {
                return Err(cold_type("__bool__ should return bool"));
            }
            return Ok(r.as_bool());
        }
        if let Some(r) = self.try_call_dunder(v, "__len__", &[], chunk, slots)? {
            return self.len_to_bool(r);
        }
        Ok(true)
    }

    /* Step an iterator candidate once, flagged builtin-iterator lists drain the front, user instances dispatch `__next__`. `None` means no iterator protocol on the receiver. */
    pub(crate) fn iter_next_proto(&mut self, iter: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Val>, VmErr> {
        if iter.is_heap()
            && let HeapObj::List(rc) = self.heap.get(iter) {
            let rc = rc.clone();
            if self.is_iter_list(&rc) {
                let mut v = rc.borrow_mut();
                if v.is_empty() { return Err(VmErr::Raised(crate::s!("StopIteration"))); }
                let item = v.remove(0);
                drop(v);
                return Ok(Some(item));
            }
        }
        self.try_call_dunder(iter, "__next__", &[], chunk, slots)
    }

    /* `in` operator prefers the container's `__contains__`. For built-in sequences with an instance item, iterate using `__eq__` so user equality is honoured. */
    pub(crate) fn contains_op(&mut self, container: Val, item: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if let Some(r) = self.try_call_dunder(container, "__contains__", &[item], chunk, slots)? {
            return Ok(self.truthy(r));
        }

        let item_is_instance = item.is_heap() && matches!(self.heap.get(item), HeapObj::Instance(..));

        // Built-in sequence container + instance item, walk and compare with `__eq__` so user equality wins over pointer eq.
        if item_is_instance && container.is_heap() {
            let items: Option<Vec<Val>> = match self.heap.get(container) {
                HeapObj::List(v) => Some(v.borrow().clone()),
                HeapObj::Tuple(v) => Some(v.clone()),
                HeapObj::Set(s) => Some(s.borrow().iter().copied().collect()),
                HeapObj::FrozenSet(s) => Some(s.iter().copied().collect()),
                _ => None,
            };
            if let Some(items) = items {
                for x in items {
                    if self.eq_op(item, x, chunk, slots)? { return Ok(true); }
                }
                return Ok(false);
            }
        }

        // User instance container with `__iter__` walks via the iterator protocol, comparing items with `__eq__`.
        if container.is_heap() && matches!(self.heap.get(container), HeapObj::Instance(..))
            && let Some(iter) = self.try_call_dunder(container, "__iter__", &[], chunk, slots)? {
            loop {
                self.charge_step()?;
                match self.iter_next_proto(iter, chunk, slots) {
                    Ok(Some(v)) => {
                        if self.eq_op(item, v, chunk, slots)? { return Ok(true); }
                    }
                    Ok(None) => return Ok(false),
                    Err(VmErr::Raised(ref m)) if m == "StopIteration" || m.starts_with("StopIteration:") => return Ok(false),
                    Err(e) => return Err(e),
                }
            }
        }

        self.contains(container, item)
    }

    /* `==` with dunder dispatch and pointer-eq fallback, used wherever `contains_op` walks a sequence. */
    pub(crate) fn eq_op(&mut self, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if let Some(r) = self.try_compare_dunder(OpCode::Eq, a, b, chunk, slots)? { return Ok(self.truthy(r)); }
        Ok(eq_vals_with_heap(a, b, &self.heap))
    }

    /* Drive a user instance's `__iter__` result to a Vec, stepping a user `__next__` or draining a builtin-iterator list. Treats a missing `__iter__` as "no protocol" by returning `None`. Used by `list(custom)`, `tuple(custom)`, etc. */
    pub(crate) fn iter_to_vec_op(&mut self, obj: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Option<Vec<Val>>, VmErr> {
        if !obj.is_heap() || !matches!(self.heap.get(obj), HeapObj::Instance(..)) { return Ok(None); }
        let Some(iter) = self.try_call_dunder(obj, "__iter__", &[], chunk, slots)? else { return Ok(None); };
        let mut out = Vec::new();
        loop {
            self.charge_step()?;
            match self.iter_next_proto(iter, chunk, slots) {
                Ok(Some(v)) => out.push(v),
                Ok(None) => return Ok(Some(out)),
                Err(VmErr::Raised(ref m)) if m == "StopIteration" || m.starts_with("StopIteration:") => return Ok(Some(out)),
                Err(e) => return Err(e),
            }
        }
    }

    /* `str(v)` semantics, instance `__str__` wins, then `__repr__`, else the built-in display. */
    pub(crate) fn display_op(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<String, VmErr> {
        if v.is_heap() && matches!(self.heap.get(v), HeapObj::Instance(..)) {
            if let Some(r) = self.try_call_dunder(v, "__str__", &[], chunk, slots)? {
                return self.require_str(r, "__str__");
            }
            if let Some(r) = self.try_call_dunder(v, "__repr__", &[], chunk, slots)? {
                return self.require_str(r, "__repr__");
            }
        }
        // Containers render their elements with repr, dispatching user __repr__ on instances.
        let s = if self.is_container_val(v) {
            self.repr_deep(v, chunk, slots, &mut Vec::new())?
        } else {
            self.display(v)
        };
        // Render is O(size). Charge it so reprinting growing data can't outrun the budget.
        self.charge_steps(s.len())?;
        Ok(s)
    }

    /* `repr(v)` semantics, instance `__repr__` wins, otherwise the built-in repr (which adds quotes for strings, etc.). */
    pub(crate) fn repr_op(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<String, VmErr> {
        if v.is_heap() && matches!(self.heap.get(v), HeapObj::Instance(..))
            && let Some(r) = self.try_call_dunder(v, "__repr__", &[], chunk, slots)? {
            return self.require_str(r, "__repr__");
        }
        let s = if self.is_container_val(v) {
            self.repr_deep(v, chunk, slots, &mut Vec::new())?
        } else {
            self.repr(v)
        };
        self.charge_steps(s.len())?;
        Ok(s)
    }

    fn is_container_val(&self, v: Val) -> bool {
        v.is_heap() && matches!(
            self.heap.get(v),
            HeapObj::List(_) | HeapObj::Tuple(_) | HeapObj::Dict(_) | HeapObj::Set(_) | HeapObj::FrozenSet(_)
        )
    }

    /* Container-aware repr dispatches `__repr__` on nested instances. Elements always use repr, with `seen` tracking heap ids for cycle detection. */
    pub(crate) fn repr_deep(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val], seen: &mut Vec<u32>) -> Result<String, VmErr> {
        const DEEP_MAX: usize = 100;
        if !v.is_heap() { return Ok(self.repr(v)); }
        if !self.is_container_val(v) {
            if matches!(self.heap.get(v), HeapObj::Instance(..))
                && let Some(r) = self.try_call_dunder(v, "__repr__", &[], chunk, slots)? {
                return self.require_str(r, "__repr__");
            }
            return Ok(self.repr(v));
        }
        let id = v.as_heap();
        if seen.contains(&id) {
            return Ok(match self.heap.get(v) {
                HeapObj::Dict(_) | HeapObj::Set(_) => "{...}".into(),
                HeapObj::Tuple(_) => "(...)".into(),
                HeapObj::FrozenSet(_) => "frozenset({...})".into(),
                _ => "[...]".into(),
            });
        }
        if seen.len() > DEEP_MAX { return Ok("...".into()); }
        seen.push(id);
        let body = self.repr_container_body(v, chunk, slots, seen);
        seen.pop();
        body
    }

    /* Builds the bracketed body for a container `v` (caller has pushed `v` to `seen`). */
    fn repr_container_body(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val], seen: &mut Vec<u32>) -> Result<String, VmErr> {
        // Clone element handles first so a dunder call (which may GC/mutate) can't dangle a borrow.
        match self.heap.get(v) {
            HeapObj::List(rc) => {
                let items = rc.borrow().clone();
                let mut out = String::from("[");
                self.join_reprs(&mut out, &items, chunk, slots, seen)?;
                out.push(']');
                Ok(out)
            }
            HeapObj::Tuple(t) => {
                let items = t.clone();
                let mut out = String::from("(");
                if items.len() == 1 {
                    let r = self.repr_deep(items[0], chunk, slots, seen)?;
                    out.push_str(&r);
                    out.push(',');
                } else {
                    self.join_reprs(&mut out, &items, chunk, slots, seen)?;
                }
                out.push(')');
                Ok(out)
            }
            HeapObj::Set(s) => {
                let items: Vec<Val> = s.borrow().iter().copied().collect();
                if items.is_empty() { return Ok("set()".into()); }
                let mut out = String::from("{");
                self.join_reprs(&mut out, &items, chunk, slots, seen)?;
                out.push('}');
                Ok(out)
            }
            HeapObj::FrozenSet(s) => {
                let items: Vec<Val> = s.iter().copied().collect();
                if items.is_empty() { return Ok("frozenset()".into()); }
                let mut out = String::from("frozenset({");
                self.join_reprs(&mut out, &items, chunk, slots, seen)?;
                out.push_str("})");
                Ok(out)
            }
            HeapObj::Dict(d) => {
                let entries: Vec<(Val, Val)> = d.borrow().iter().collect();
                let mut out = String::from("{");
                for (i, (k, val)) in entries.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    let kr = self.repr_deep(*k, chunk, slots, seen)?;
                    out.push_str(&kr);
                    out.push_str(": ");
                    let vr = self.repr_deep(*val, chunk, slots, seen)?;
                    out.push_str(&vr);
                }
                out.push('}');
                Ok(out)
            }
            _ => Ok(self.repr(v)),
        }
    }

    fn join_reprs(&mut self, out: &mut String, items: &[Val], chunk: &SSAChunk, slots: &mut [Val], seen: &mut Vec<u32>) -> Result<(), VmErr> {
        for (i, e) in items.iter().enumerate() {
            if i > 0 { out.push_str(", "); }
            let r = self.repr_deep(*e, chunk, slots, seen)?;
            out.push_str(&r);
        }
        Ok(())
    }

    fn require_str(&self, v: Val, name: &str) -> Result<String, VmErr> {
        if v.is_heap() && let HeapObj::Str(s) = self.heap.get(v) { return Ok(s.clone()); }
        Err(VmErr::TypeMsg(crate::s!("'", str name, "' did not return a string")))
    }

    /* `format(v, spec)` dispatch, instance `__format__(spec)` wins, otherwise the built-in spec engine runs. Empty spec on an instance still goes through `__format__` so user formatting can opt in. */
    pub(crate) fn format_op(&mut self, v: Val, spec: &str, chunk: &SSAChunk, slots: &mut [Val]) -> Result<String, VmErr> {
        if v.is_heap() && matches!(self.heap.get(v), HeapObj::Instance(..)) {
            let spec_val = self.heap.alloc(HeapObj::Str(spec.to_string()))?;
            if let Some(r) = self.try_call_dunder(v, "__format__", &[spec_val], chunk, slots)? {
                return self.require_str(r, "__format__");
            }
        }
        crate::vm::format_spec::format_value(v, spec, &self.heap).map_err(crate::vm::format_spec::fmt_err)
    }

    /* Coerce a `__len__` / `__length_hint__` return value to bool semantics and reject negatives. */
    fn len_to_bool(&self, v: Val) -> Result<bool, VmErr> {
        let n = if v.is_int() { v.as_int() as i128 }
        else if let Some(i) = crate::vm::types::as_i128(v, &self.heap) { i }
        else { return Err(cold_type("__len__ must return int")); };
        if n < 0 { return Err(cold_value("__len__() should return >= 0")); }
        Ok(n != 0)
    }
}
