use alloc::{string::String, vec::Vec};

use super::super::VM;
use super::super::types::*;
use super::matches_exc_class;

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
        // Use the NaN-boxed bit pattern as identity. Truncate to fit `INT_MAX`.
        let id = ((o.0 as i64).wrapping_abs()) & Val::INT_MAX; // wrapping_abs since i64::MIN (e.g. -0.0 bits) would overflow plain abs
        self.push(Val::int(id));
        Ok(())
    }

    pub fn call_hash(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use core::hash::{Hash, Hasher};
        let o = self.pop()?;

        // instance dispatch, user `__hash__` wins, `__eq__` without `__hash__` makes the instance unhashable.
        if o.is_heap() && let HeapObj::Instance(cls, _) = self.heap.get(o) {
            let cls = *cls;
            let has_hash = self.lookup_class_member(cls, "__hash__").is_some();
            let has_eq = self.lookup_class_member(cls, "__eq__").is_some();
            if has_hash {
                let r = self.try_call_dunder(o, "__hash__", &[], chunk, slots)?
                    .ok_or_else(|| cold_type("__hash__ returned NotImplemented"))?;
                if !r.is_int() {
                    return Err(cold_type("__hash__ must return int"));
                }
                self.push(Val::int(r.as_int() & Val::INT_MAX));
                return Ok(());
            }
            if has_eq {
                return Err(cold_type("unhashable type: instance defines __eq__ without __hash__"));
            }
            // Default fallback, pointer identity, mirroring Python's `object.__hash__`.
        }

        // Python guarantees hash(n)==n for ints in range (hash(-1) is -2), bool hashes as 0/1.
        if o.is_int() { let n = o.as_int(); self.push(Val::int(if n == -1 { -2 } else { n })); return Ok(()); }
        if o.is_bool() { self.push(Val::int(o.as_bool() as i64)); return Ok(()); }

        let mut h = crate::util::hash::FxHasher::default();
        if o.is_float() {
            let f = o.as_float();
            // Integral floats hash as the equal int, same unification hash_depth uses for dict keys.
            if f as i64 as f64 == f {
                let n = f as i64;
                let v = self.int_to_val(Some((if n == -1 { -2 } else { n }) as i128))?;
                self.push(v);
                return Ok(());
            }
            f.to_bits().hash(&mut h);
        }
        else if o.is_none() { 0u64.hash(&mut h); }
        else if o.is_heap() {
            match self.heap.get(o) {
                HeapObj::Str(s) => s.hash(&mut h),
                HeapObj::Bytes(b) => b.hash(&mut h),
                HeapObj::Tuple(items) => { for v in items { v.0.hash(&mut h); } }
                HeapObj::List(_) | HeapObj::Dict(_) | HeapObj::Set(_) => { return Err(cold_type("unhashable type")); }
                _ => o.0.hash(&mut h),
            }
        }
        self.push(Val::int(h.finish() as i64 & Val::INT_MAX));
        Ok(())
    }

    /* Type-name based isinstance check. Accepts Type / NativeFn (builtin types) / user Class on the right, allows int<->bool aliasing and walks user inheritance via `is_subclass`. */
    pub fn call_isinstance(&mut self) -> Result<(), VmErr> {
        let (arg2, obj) = (self.pop()?, self.pop()?);
        // User instances keep the "object" label here, class membership is decided via obj_class below.
        let obj_ty = if obj.is_heap() && matches!(self.heap.get(obj), HeapObj::Instance(..)) {
            "object"
        } else {
            self.type_name(obj)
        };

        // For exception matching, when `obj` is a Type itself or an ExcInstance, compare names against the asserted type.
        let obj_type_name: Option<String> = if obj.is_heap() {
            match self.heap.get(obj) {
                HeapObj::Type(n) => Some(n.clone()),
                HeapObj::ExcInstance(n, _) => Some(n.clone()),
                _ => None,
            }
        } else { None };

        // User-class membership uses heap identity, not type names, so capture the instance's class up-front.
        let obj_class: Option<Val> = if obj.is_heap() {
            if let HeapObj::Instance(cls, _) = self.heap.get(obj) { Some(*cls) } else { None }
        } else { None };

        let check_one = |t: Val, heap: &HeapPool| -> Result<bool, VmErr> {
            if !t.is_heap() {
                return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types"));
            }
            let exc_match = |name: &str| -> bool {
                obj_type_name.as_deref()
                    .map(|n| matches_exc_class(n, name))
                    .unwrap_or(false)
            };
            match heap.get(t) {
                HeapObj::Type(name) => Ok(
                    name == "object" // everything is an object
                    || matches_exc_class(obj_ty, name)
                    || (obj_ty == "bool" && name == "int")
                    || exc_match(name)
                ),
                HeapObj::NativeFn(id) => {
                    let name = id.name();
                    if !matches!(name, "int"|"str"|"bytes"|"float"|"bool"|"list"|"tuple"|"dict"|"set") {
                        return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types"));
                    }
                    Ok(
                        name == obj_ty
                        || (obj_ty == "bool" && name == "int")
                        )
                }
                HeapObj::Class(..) => Ok(obj_class.is_some_and(|c| heap.is_subclass(c, t))),
                _ => Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types")),
            }
        };

        self.check_classinfo(arg2, check_one)
    }

    /* Shared `isinstance`/`issubclass` classinfo dispatch, a tuple matches if any member does, else a single check. `single` already emits the correct TypeError for non-heap / wrong-variant args. */
    fn check_classinfo<F>(&mut self, arg2: Val, single: F) -> Result<(), VmErr>
    where F: Fn(Val, &HeapPool) -> Result<bool, VmErr> {
        let result = if arg2.is_heap() && let HeapObj::Tuple(items) = self.heap.get(arg2) {
            // Propagate TypeError from a non-class member instead of silently ignoring it.
            let items: Vec<Val> = items.clone();
            let mut found = false;
            for t in items { if single(t, &self.heap)? { found = true; break; } }
            found
        } else {
            single(arg2, &self.heap)?
        };
        self.push(Val::bool(result));
        Ok(())
    }

    /* `issubclass(C, B)`, both are classes (B may be a tuple). Walks the exception hierarchy for built-ins and the inheritance chain for user classes. Unlike `isinstance`, arg 1 must itself be a class. */
    pub fn call_issubclass(&mut self) -> Result<(), VmErr> {
        let (arg2, sub) = (self.pop()?, self.pop()?);

        // arg 1 must be a built-in/exception `Type` or a user `Class`.
        let (sub_name, sub_class): (Option<String>, Option<Val>) = match sub.is_heap().then(|| self.heap.get(sub)) {
            Some(HeapObj::Type(n)) => (Some(n.clone()), None),
            Some(HeapObj::Class(..)) => (None, Some(sub)),
            _ => return Err(VmErr::Type("issubclass() arg 1 must be a class")),
        };

        let check_one = |t: Val, heap: &HeapPool| -> Result<bool, VmErr> {
            if !t.is_heap() {
                return Err(VmErr::Type("issubclass() arg 2 must be a class or tuple of classes"));
            }
            match heap.get(t) {
                HeapObj::Type(name2) => Ok(match &sub_name {
                    Some(name1) => matches_exc_class(name1, name2) || (name1 == "bool" && name2 == "int"),
                    None => false, // user class is never a subclass of a built-in type
                }),
                HeapObj::Class(..) => Ok(sub_class.is_some_and(|c| heap.is_subclass(c, t))),
                _ => Err(VmErr::Type("issubclass() arg 2 must be a class or tuple of classes")),
            }
        };

        self.check_classinfo(arg2, check_one)
    }
}
