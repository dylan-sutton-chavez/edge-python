
use super::super::VM;
use super::super::types::*;
use super::matches_exc_class;

/* CPython's int hash, identity in i64 range with -1 mapped to -2, wider values reduce mod 2^61-1. */
fn py_int_hash(i: i128) -> i128 {
    match i64::try_from(i) {
        Ok(n) => (if n == -1 { -2 } else { n }) as i128,
        Err(_) => i.rem_euclid((1i128 << 61) - 1),
    }
}

impl<'a> VM<'a> {

    /* `property(fget)` / `property(fget, fset)`, captures the descriptor pair the class chain hands to `LoadAttr` / `StoreAttr`. The `@x.setter` decorator builds the second form via `PropertySetter`. */
    pub fn call_property(&mut self, argc: u16) -> Result<(), VmErr> {
        let args = self.pop_n(argc as usize)?;
        let (getter, setter) = match args.as_slice() {
            [g] => (*g, Val::none()),
            [g, s] => (*g, *s),
            _ => return Err(cold_type("property() takes 1 or 2 arguments")),
        };
        let prop = self.heap.alloc(HeapObj::Property(getter, setter))?;
        self.push(prop);
        Ok(())
    }

    // `staticmethod(func)` wraps a function so the class chain returns it unbound, with no `self`.
    pub fn call_staticmethod(&mut self, argc: u16) -> Result<(), VmErr> {
        let args = self.pop_n(argc as usize)?;
        let [func] = args.as_slice() else {
            return Err(cold_type("staticmethod() takes exactly one argument"));
        };
        let wrapped = self.heap.alloc(HeapObj::StaticMethod(*func))?;
        self.push(wrapped);
        Ok(())
    }

    // `classmethod(func)` wraps a function so attribute lookup binds the class.
    pub fn call_classmethod(&mut self, argc: u16) -> Result<(), VmErr> {
        let args = self.pop_n(argc as usize)?;
        let [func] = args.as_slice() else {
            return Err(cold_type("classmethod() takes exactly one argument"));
        };
        let wrapped = self.heap.alloc(HeapObj::ClassMethod(*func))?;
        self.push(wrapped);
        Ok(())
    }

    // `super()` zero-arg reads the running method's `(class, self)` off the top frame and returns a Super proxy.
    pub fn call_super(&mut self) -> Result<(), VmErr> {
        let binding = self.call_stack.last()
            .and_then(|f| f.current_class.zip(f.current_self));
        let Some((class, recv)) = binding else {
            return Err(VmErr::Runtime("super() must be called inside a method"));
        };
        let proxy = self.heap.alloc(HeapObj::Super(class, recv))?;
        self.push(proxy);
        Ok(())
    }

    pub fn call_repr(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.repr_op(o, chunk, slots)?;
        self.alloc_and_push_str(s)
    }

    pub fn call_callable(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let result = if o.is_heap() {
            match self.heap.get(o) {
                HeapObj::Func(..) | HeapObj::BoundMethod(..)
                | HeapObj::Type(_) | HeapObj::NativeFn(_)
                | HeapObj::Class(..) | HeapObj::BoundUserMethod(..)
                | HeapObj::Extern(_) => true,
                // instance is callable iff its class chain defines `__call__`.
                HeapObj::Instance(cls, _) => self.lookup_class_member(*cls, "__call__").is_some(),
                _ => false,
            }
        } else { false };
        self.push(Val::bool(result));
        Ok(())
    }

    pub fn call_id(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        // The full NaN-boxed bits, the ones `is` compares, so `id` and `is` always agree.
        let id = self.int_to_val(Some(o.0 as i128))?;
        self.push(id);
        Ok(())
    }

    pub fn call_hash(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let o = self.pop()?;

        // instance dispatch, user `__hash__` wins, `__eq__` without `__hash__` makes the instance unhashable.
        if o.is_heap() && let HeapObj::Instance(cls, _) = self.heap.get(o) {
            let cls = *cls;
            let hash_fn = self.lookup_class_member(cls, "__hash__").map(|(f, _)| f);
            let has_eq = self.lookup_class_member(cls, "__eq__").is_some();
            // `__hash__ = object.__hash__` keeps the identity hash below.
            let inherited = hash_fn.is_some_and(|f| f.is_heap() && matches!(self.heap.get(f), HeapObj::BoundMethod(_, id) if id.name() == "__hash__"));
            if hash_fn.is_some() && !inherited {
                let r = self.try_call_dunder(o, "__hash__", &[], chunk, slots)?
                    .ok_or_else(|| cold_type("__hash__ returned NotImplemented"))?;
                if !r.is_int() {
                    return Err(cold_type("__hash__ must return int"));
                }
                self.push(Val::int(r.as_int() & Val::INT_MAX));
                return Ok(());
            }
            if hash_fn.is_none() && has_eq {
                return Err(cold_type("unhashable type: instance defines __eq__ without __hash__"));
            }
            // Default fallback, pointer identity, mirroring Python's `object.__hash__`.
        }

        // Python guarantees hash(n)==n for ints in range (hash(-1) is -2), bool hashes as 0/1.
        if o.is_int() { let n = o.as_int(); self.push(Val::int(if n == -1 { -2 } else { n })); return Ok(()); }
        if o.is_bool() { self.push(Val::int(o.as_bool() as i64)); return Ok(()); }

        // Wide ints hash by value through the same rule, the i64 branch keeps hash(n)==n.
        let wide = if o.is_heap() {
            if let HeapObj::LongInt(i) = self.heap.get(o) { Some(i.get()) } else { None }
        } else { None };
        if let Some(i) = wide {
            let v = self.int_to_val(Some(py_int_hash(i)))?;
            self.push(v);
            return Ok(());
        }

        // Integral floats hash as the equal int or LongInt, same unification hash_depth uses for dict keys.
        if o.is_float() && let Some(i) = float_exact_i128(o.as_float()) {
            let v = self.int_to_val(Some(py_int_hash(i)))?;
            self.push(v);
            return Ok(());
        }
        // The rest hashes by content, the hash a dict key probes with, so equal tuples and frozensets agree.
        if o.is_heap() { self.require_hashable(o)?; }
        // A tuple holding a user-hashed item hashes through that item, as its dict key would.
        let rich_tuple = matches!(self.heap.try_get(o), Some(HeapObj::Tuple(_))) && crate::vm::eq::is_rich_key(o, &self.heap);
        let h = if rich_tuple { self.with_roots([o], |vm| vm.key_hash(o, chunk, slots))? }
            else { crate::vm::eq::hash_val_with_heap(o, &self.heap) };
        self.push(Val::int(h as i64 & Val::INT_MAX));
        Ok(())
    }

    /* Type-name based isinstance check. Accepts Type / NativeFn (builtin types) / user Class on the right, allows int<->bool aliasing and walks user inheritance via `is_subclass`. */
    pub fn call_isinstance(&mut self) -> Result<(), VmErr> {
        let (arg2, obj) = (self.pop()?, self.pop()?);
        let result = self.check_classinfo(arg2, |t| self.is_instance_of(obj, t))?;
        self.push(Val::bool(result));
        Ok(())
    }

    /* `isinstance(obj, t)` for one class, builtins by type name and user classes along the bases. */
    fn is_instance_of(&self, obj: Val, t: Val) -> Result<bool, VmErr> {
        let bad = || VmErr::Type("isinstance() arg 2 must be a type or tuple of types");
        // A user instance is named "object" so a class named like a builtin never matches.
        let (obj_ty, obj_class) = match self.heap.try_get(obj) {
            Some(&HeapObj::Instance(cls, _)) => ("object", Some(cls)),
            _ => (self.type_name(obj), None),
        };
        // An exception instance or a type object also matches through the exception tree by its own name.
        let exc_name = match self.heap.try_get(obj) { Some(HeapObj::Type(n) | HeapObj::ExcInstance(n, _)) => Some(n.as_str()), _ => None };
        match self.heap.try_get(t).ok_or_else(bad)? {
            HeapObj::Type(name) => Ok(name == "object" || matches_exc_class(obj_ty, name) || (obj_ty == "bool" && name == "int")
                || exc_name.is_some_and(|n| matches_exc_class(n, name))
                || obj_class.and_then(|c| self.exc_base(c)).is_some_and(|b| matches_exc_class(b, name))),
            HeapObj::NativeFn(id) if matches!(id.name(), "int" | "str" | "bytes" | "float" | "bool" | "list" | "tuple" | "dict" | "set") =>
                Ok(id.name() == obj_ty || (obj_ty == "bool" && id.name() == "int")),
            HeapObj::Class(..) => Ok(obj_class.is_some_and(|c| self.heap.is_subclass(c, t))),
            _ => Err(bad()),
        }
    }

    /* Shared `isinstance`/`issubclass` classinfo dispatch, a tuple or union matches if any member does, else a single check. */
    fn check_classinfo(&self, arg2: Val, single: impl Fn(Val) -> Result<bool, VmErr>) -> Result<bool, VmErr> {
        // `int | str` checks like the tuple of its members.
        let arg2 = match self.heap.try_get(arg2) { Some(&HeapObj::Union(args)) => args, _ => arg2 };
        if let Some(HeapObj::Tuple(items)) = self.heap.try_get(arg2) {
            // A non-class member raises instead of being skipped.
            for &t in items { if single(t)? { return Ok(true); } }
            return Ok(false);
        }
        single(arg2)
    }

    /* `issubclass(C, B)`, both are classes (B may be a tuple). Walks the exception hierarchy for built-ins and the inheritance chain for user classes. Unlike `isinstance`, arg 1 must itself be a class. */
    pub fn call_issubclass(&mut self) -> Result<(), VmErr> {
        let (arg2, sub) = (self.pop()?, self.pop()?);
        // arg 1 must be a built-in/exception `Type` or a user `Class`.
        let sub_name = match self.heap.try_get(sub) {
            Some(HeapObj::Type(n)) => Some(n.as_str()),
            Some(HeapObj::Class(..)) => None,
            _ => return Err(VmErr::Type("issubclass() arg 1 must be a class")),
        };
        let result = self.check_classinfo(arg2, |t| match self.heap.try_get(t) {
            // A user class reaches a builtin type only through an exception base.
            Some(HeapObj::Type(name)) => Ok(sub_name.or_else(|| self.exc_base(sub)).is_some_and(|n| matches_exc_class(n, name) || (n == "bool" && name == "int"))),
            Some(HeapObj::Class(..)) => Ok(sub_name.is_none() && self.heap.is_subclass(sub, t)),
            _ => Err(VmErr::Type("issubclass() arg 2 must be a class or tuple of classes")),
        })?;
        self.push(Val::bool(result));
        Ok(())
    }
}
