use super::*;

impl<'a> VM<'a> {

    /* StoreName: single SSA slot write after register coalescing. */
    pub(crate) fn handle_store(&mut self, operand: u16, slots: &mut [Val]) -> Result<(), VmErr> {
        let v = self.pop()?;
        // Malformed bytecode can carry an out-of-range slot; drop the write rather than panic.
        if let Some(s) = slots.get_mut(operand as usize) { *s = v; }
        Ok(())
    }

    /* Container constructors: list / tuple / dict / set / slice / string. */
    pub(crate) fn handle_build(&mut self, op: OpCode, operand: u16) -> Result<(), VmErr> {
        match op {
            OpCode::BuildList => {
                let v = self.pop_n(operand as usize)?;
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(v))))?;
                self.push(val);
            }
            OpCode::BuildTuple => {
                let v = self.pop_n(operand as usize)?;
                let val = self.heap.alloc(HeapObj::Tuple(v))?;
                self.push(val);
            }
            OpCode::BuildDict => {
                let flat = self.pop_n(operand as usize * 2)?;
                for pair in flat.chunks(2) { self.require_hashable(pair[0])?; }
                let dm = DictMap::from_pairs(flat.chunks(2).map(|c| (c[0], c[1])).collect(), &self.heap);
                let val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?;
                self.push(val);
            }
            OpCode::BuildString => {
                let parts = self.pop_n(operand as usize)?;
                let s: String = parts.iter().map(|v| self.display(*v)).collect();
                let val = self.heap.alloc(HeapObj::Str(s))?;
                self.push(val);
            }
            OpCode::BuildSet => self.build_set(operand)?,
            OpCode::BuildSlice => self.build_slice(operand)?,
            _ => return Err(cold_runtime("non-build opcode in handle_build")),
        }
        Ok(())
    }

    /* Unpacking and `{value!s:spec}` formatting. Indexed get/store/del are dispatched directly from the hot loop, never here. */
    pub(crate) fn handle_container(&mut self, op: OpCode, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        match op {
            OpCode::UnpackSequence => self.exec_unpack_seq(operand as usize)?,
            OpCode::UnpackEx => self.unpack_ex(operand)?,
            OpCode::FormatValue => {
                /* Operand layout: bit 0 has_spec, bits 1..=2 conversion (0 none, 1 !r, 2 !s, 3 !a). See parser/literals.rs. */
                let has_spec = (operand & 1) != 0;
                let conv = (operand >> 1) & 0b11;
                let spec_val = if has_spec { Some(self.pop()?) } else { None };
                let v = self.pop()?;

                // Conversion flags consult the dunder-aware helpers so `f"{x!s}"` honours `__str__`.
                // Charge each result's length; a big Str is one heap object the object quota misses.
                let converted = match conv {
                    1 => { let s = self.repr_op(v, chunk, slots)?; self.charge_steps(s.len())?; self.heap.alloc(HeapObj::Str(s))? }
                    2 => { let s = self.display_op(v, chunk, slots)?; self.charge_steps(s.len())?; self.heap.alloc(HeapObj::Str(s))? }
                    3 => {
                        let raw = super::format::display_inline(v, &self.heap);
                        self.charge_steps(raw.len())?;
                        self.heap.alloc(HeapObj::Str(raw.escape_default().collect::<String>()))?
                    }
                    _ => v,
                };

                let result = match spec_val {
                    Some(sv) => {
                        // `try_get`: a non-heap spec value is a TypeError, not a bad-index heap access.
                        let spec = match self.heap.try_get(sv) {
                            Some(HeapObj::Str(s)) => s.clone(),
                            _ => return Err(cold_type("format spec must be a string")),
                        };
                        // Instance `__format__(spec)` runs through `format_op`; built-ins fall through to the spec engine.
                        self.format_op(converted, &spec, chunk, slots)?
                    }
                    None => {
                        if conv != 0 && let HeapObj::Str(s) = self.heap.get(converted) {
                            s.clone()
                        } else {
                            self.display_op(converted, chunk, slots)?
                        }
                    }
                };
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val);
            }
            _ => return Err(cold_runtime("non-container opcode in handle_container")),
        }
        Ok(())
    }

    /* Append/add to the comprehension accumulator at the top of the stack. */
    pub(crate) fn handle_comprehension(&mut self, op: OpCode) -> Result<(), VmErr> {
        let (kind, value, key) = match op {
            OpCode::ListAppend => ("list", self.pop()?, None),
            OpCode::SetAdd => ("set", self.pop()?, None),
            OpCode::MapAdd => { let v = self.pop()?; let k = self.pop()?; ("dict", v, Some(k)) }
            _ => return Err(cold_runtime("non-comprehension opcode in handle_comprehension")),
        };
        let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
        let corrupt = || VmErr::Runtime(match kind {
            "list" => "list accumulator corrupted",
            "set" => "set accumulator corrupted",
            _ => "dict accumulator corrupted",
        });
        if !acc.is_heap() { return Err(corrupt()); }
        match (kind, self.heap.get(acc)) {
            ("list", HeapObj::List(rc)) => { rc.borrow_mut().push(value); }
            ("set", HeapObj::Set(rc))  => { rc.borrow_mut().insert(value, &self.heap); }
            ("dict", HeapObj::Dict(rc)) => { rc.borrow_mut().insert(key.unwrap(), value, &self.heap); }
            _ => return Err(corrupt()),
        }
        Ok(())
    }

    /* Merge the source on top of the stack into the container below it: `{**m}`, `{*s}`, `[*it]`. */
    pub(crate) fn handle_spread_merge(&mut self, op: OpCode) -> Result<(), VmErr> {
        let src = self.pop()?;
        let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
        if !acc.is_heap() { return Err(cold_runtime("spread accumulator corrupted")); }
        match op {
            OpCode::DictUpdate => {
                // `**` requires a mapping; later keys overwrite earlier ones.
                let pairs: Vec<(Val, Val)> = match self.heap.try_get(src) {
                    Some(HeapObj::Dict(rc)) => rc.borrow().iter().collect(),
                    _ => return Err(cold_type("argument after ** must be a mapping")),
                };
                if let HeapObj::Dict(rc) = self.heap.get(acc) {
                    let mut m = rc.borrow_mut();
                    for (k, v) in pairs { m.insert(k, v, &self.heap); }
                }
            }
            OpCode::SetUpdate => {
                let items = self.iter_to_vec_for_spread(src)?;
                for it in items {
                    self.require_hashable(it)?;
                    match self.heap.get(acc) {
                        HeapObj::Set(rc) => { rc.borrow_mut().insert(it, &self.heap); }
                        _ => return Err(cold_runtime("spread accumulator corrupted")),
                    }
                }
            }
            OpCode::ListExtend => {
                let items = self.iter_to_vec_for_spread(src)?;
                match self.heap.get(acc) {
                    HeapObj::List(rc) => rc.borrow_mut().extend(items),
                    _ => return Err(cold_runtime("spread accumulator corrupted")),
                }
            }
            _ => return Err(cold_runtime("non-spread opcode in handle_spread_merge")),
        }
        Ok(())
    }

    /* Yield: keep the value on the stack and flag the executor to suspend. */
    pub(crate) fn handle_yield(&mut self) -> Result<(), VmErr> {
        let v = self.pop()?;
        self.push(v);
        self.yielded = true;
        Ok(())
    }

    /* Side-effecting / impure ops: assert, del, global/nonlocal, import, type alias, raise, await. */
    pub(crate) fn handle_side(&mut self, op: OpCode, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        match op {
            OpCode::Assert => {
                let v = self.pop()?;
                if !self.truthy_op(v, chunk, slots)? {
                    // Bare `assert` raises a catchable AssertionError with empty args.
                    let inst = self.heap.alloc(HeapObj::ExcInstance("AssertionError".into(), Vec::new()))?;
                    self.pending.exc_val = Some(inst);
                    return Err(VmErr::Raised("AssertionError".into()));
                }
            }
            OpCode::Del => {
                let slot = operand as usize;
                // Deleting an already-unbound name raises NameError, matching CPython.
                match slots.get_mut(slot) {
                    Some(s) if !s.is_undef() => *s = Val::undef(),
                    _ => {
                        let name = chunk.names.get(slot).map(|n| ssa_strip(n)).unwrap_or_default();
                        return Err(VmErr::Name(name.into()));
                    }
                }
                // At module scope, drop it from module_state too so later reads see the deletion.
                if core::ptr::eq(chunk, self.chunk) && let Some(n) = chunk.names.get(slot) {
                    self.module_state.remove(ssa_strip(n));
                }
                // Unbind the shared closure cell too, so closures over this name see the deletion.
                let cell = self.call_stack.last()
                    .and_then(|f| f.cells.iter().find(|(s, _)| *s == slot).map(|&(_, c)| c));
                if let Some(cell) = cell && cell.is_heap() && let HeapObj::List(rc) = self.heap.get(cell) {
                    rc.borrow_mut()[0] = Val::undef(); // cells are 1-element boxes
                }
            }
            OpCode::Global | OpCode::Nonlocal => self.mark_impure(),
            OpCode::Raise | OpCode::RaiseFrom => {
                self.mark_impure();
                // Bare `raise` (operand 1): re-raise the exception currently being handled.
                if op == OpCode::Raise && operand == 1 {
                    let Some(exc) = self.handling_exc else {
                        return Err(VmErr::Runtime("No active exception to re-raise"));
                    };
                    let name = self.exc_type_name(exc);
                    self.pending.exc_val = Some(exc);
                    return Err(VmErr::Raised(name));
                }
                // RaiseFrom emits both `expr` then `from expr`, the topmost value is the cause, but the exception to raise is the LHS.
                if op == OpCode::RaiseFrom { let _cause = self.pop()?; }
                let exc = self.pop()?;
                // Stash the Val for `except as e` binding; non-Exc values use `display()`.
                self.pending.exc_val = None;
                // Extract owned (class name, first arg) so display() can run after the heap borrow ends.
                let info: Option<(alloc::string::String, Option<Val>)> = if exc.is_heap() {
                    match self.heap.get(exc) {
                        HeapObj::ExcInstance(n, args) => {
                            self.pending.exc_val = Some(exc);
                            Some((n.clone(), args.first().copied()))
                        }
                        HeapObj::Type(n) => {
                            // Bare `raise X`: build empty ExcInstance so `e.args` is `()`.
                            let n = n.clone();
                            let inst = self.heap.alloc(
                                HeapObj::ExcInstance(n.clone(), Vec::new()))?;
                            self.pending.exc_val = Some(inst);
                            Some((n, None))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                // Append the first arg so an uncaught traceback reads "Class: message".
                let msg = match info {
                    Some((n, Some(arg))) => { let detail = self.display(arg); crate::s!(str &n, ": ", str &detail) }
                    Some((n, None)) => n,
                    // Non-exception value (str, int, ...): raises TypeError, catchable by `except Exception`.
                    None => crate::s!("TypeError: exceptions must derive from BaseException"),
                };
                return Err(VmErr::Raised(msg));
            }
            OpCode::Await => {
                // Coroutine: park on it (single-driver) so the top loop runs it to completion, even across suspension; sync values pass through.
                let val = self.pop()?;
                if val.is_heap() && matches!(self.heap.get(val), HeapObj::Coroutine(..)) {
                    self.await_coroutine(val)?;
                } else {
                    self.push(val);
                }
            }
            _ => return Err(cold_runtime("non-side opcode in handle_side")),
        }
        Ok(())
    }
}
