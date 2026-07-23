use crate::s;
use super::*;
use super::super::ParamKind;

use crate::alloc::string::ToString;

// Builtin conversion-type name -> its constructor; None for exception/other types.
fn constructor_native(name: &str) -> Option<super::super::types::NativeFnId> {
    use super::super::types::NativeFnId::*;
    Some(match name {
        "int" => Int, "float" => Float, "str" => Str, "bytes" => Bytes,
        "bool" => Bool, "list" => List, "tuple" => Tuple, "dict" => Dict,
        "set" => Set, "frozenset" => FrozenSet, "range" => Range, "type" => Type,
        "slice" => Slice,
        _ => return None,
    })
}

// Effects/mutation/scheduling/import/reflection/nondeterminism. Over-marking only forgoes memoisation; under-marking drops effects.
fn native_is_impure(id: super::super::types::NativeFnId) -> bool {
    use super::super::types::NativeFnId::*;
    matches!(id,
        Print | Input | Receive | Sleep // I/O + scheduler
        | SetAttr | DelAttr // mutation
        | GetAttr | HasAttr // attr access can run getters
        | Run | ImportModule // arbitrary execution / import
        | Cancel | WithTimeout | Gather // async effects
        | Globals | Locals | Vars | Frame | Super // reflection of mutable state
        | Id // heap-slot nondeterminism
    )
}

// Fused builtin opcode -> its id (plain-`pos+kw` operand ops); powers the shared arity guard.
fn fused_native(op: OpCode) -> Option<super::super::types::NativeFnId> {
    use super::super::types::NativeFnId as F;
    Some(match op {
        OpCode::CallLen => F::Len, OpCode::CallAbs => F::Abs, OpCode::CallStr => F::Str, OpCode::CallInt => F::Int,
        OpCode::CallType => F::Type, OpCode::CallFloat => F::Float, OpCode::CallBool => F::Bool, OpCode::CallRound => F::Round,
        OpCode::CallSum => F::Sum, OpCode::CallZip => F::Zip, OpCode::CallList => F::List, OpCode::CallTuple => F::Tuple,
        OpCode::CallSet => F::Set, OpCode::CallInput => F::Input, OpCode::CallIsInstance => F::IsInstance, OpCode::CallChr => F::Chr,
        OpCode::CallOrd => F::Ord, OpCode::CallAll => F::All, OpCode::CallAny => F::Any, OpCode::CallBin => F::Bin,
        OpCode::CallOct => F::Oct, OpCode::CallHex => F::Hex, OpCode::CallDivmod => F::Divmod, OpCode::CallPow => F::Pow,
        OpCode::CallRepr => F::Repr, OpCode::CallReversed => F::Reversed, OpCode::CallCallable => F::Callable,
        OpCode::CallId => F::Id, OpCode::CallHash => F::Hash,
        _ => return None,
    })
}

impl<'a> VM<'a> {
    /* Dispatch every function-shaped opcode (Call, MakeFunction, builtins). */
    pub(crate) fn handle_function(&mut self, op: OpCode, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Fused builtins skip dispatch_native's arity check; validate fixed-arity ones so a wrong count is a clean TypeError.
        if let Some(id) = fused_native(op)
            && let Some(n) = id.arity()
            && operand != n {
            return Err(cold_type("wrong number of arguments to builtin"));
        }
        match op {
            OpCode::Call => {
                let r = self.exec_call(operand, chunk, slots);
                // Restore the enclosing call's spread delta.
                if let Some((p, k)) = self.pending.delta_save.pop() {
                    self.pending.pos_delta = p;
                    self.pending.kw_delta = k;
                }
                r
            }
            OpCode::MakeFunction | OpCode::MakeCoroutine => self.exec_make_function(op, operand, chunk, slots),
            OpCode::CallLen => self.call_len(chunk, slots),
            OpCode::CallAbs => self.call_abs(),
            OpCode::CallStr => self.call_str(operand, chunk, slots),
            OpCode::CallInt => self.call_int(operand, chunk, slots),
            OpCode::CallFloat => self.call_float(operand),
            OpCode::CallBool => self.call_bool(operand, chunk, slots),
            OpCode::CallType => self.call_type(),
            OpCode::CallChr => self.call_chr(),
            OpCode::CallOrd => self.call_ord(),
            OpCode::CallSorted => self.call_sorted(false, chunk, slots),
            OpCode::CallList => self.call_list(operand, chunk, slots),
            OpCode::CallTuple => self.call_tuple(operand, chunk, slots),
            OpCode::CallEnumerate => self.call_enumerate(operand),
            OpCode::CallIsInstance => self.call_isinstance(),
            OpCode::CallRange => self.call_range(operand),
            OpCode::CallRound => self.call_round(operand),
            OpCode::CallMin => self.call_min(operand, chunk, slots),
            OpCode::CallMax => self.call_max(operand, chunk, slots),
            OpCode::CallSum => self.call_sum(operand),
            OpCode::CallZip => self.call_zip(operand),
            OpCode::CallDict => self.call_dict(operand),
            OpCode::CallSet => self.call_set(operand),
            OpCode::CallPrint => { self.mark_impure(); self.call_print(operand, chunk, slots) }
            OpCode::CallInput => { self.mark_impure(); self.call_input() }
            OpCode::CallAll => self.call_all(operand),
            OpCode::CallAny => self.call_any(operand),
            OpCode::CallBin => self.call_bin(),
            OpCode::CallOct => self.call_oct(),
            OpCode::CallHex => self.call_hex(),
            OpCode::CallDivmod => self.call_divmod(),
            OpCode::CallPow => self.call_pow(operand),
            OpCode::CallRepr => self.call_repr(chunk, slots),
            OpCode::CallReversed => self.call_reversed(),
            OpCode::CallCallable => self.call_callable(),
            OpCode::CallId => self.call_id(),
            OpCode::CallHash => self.call_hash(chunk, slots),
            OpCode::CallExtern => self.call_extern(operand, chunk),
            _ => Err(cold_runtime("non-function opcode in handle_function")),
        }
    }

    pub(crate) fn exec_bound_method(&mut self, recv: Val, id: super::methods::BuiltinMethodId, pos: &[Val], kw: &[Val]) -> Result<(), VmErr> {
        super::methods::dispatch_method(self, id, recv, pos, kw)
    }

    fn exec_make_function(&mut self, opcode: OpCode, operand: u16, chunk: &SSAChunk, slots: &[Val]) -> Result<(), VmErr> {
        let chunk_ptr = chunk as *const _;
        let global = self.fn_index.iter()
            .find(|(p, _)| *p == chunk_ptr)
            .and_then(|(_, v)| v.get(operand as usize).copied())
            .ok_or(cold_runtime("MakeFunction: unknown function index"))? as usize;

        if opcode == OpCode::MakeCoroutine {
            if self.is_async.len() <= global { self.is_async.resize(global + 1, false); }
            self.is_async[global] = true;
        }

        let n_defaults = self.functions[global].2 as usize;
        let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };

        let (params, body, _, _) = self.functions[global];
        let param_names: crate::util::fx::FxHashSet<String> = params.iter().map(|p| s!(str crate::modules::parser::types::param_base_name(p), "_0")).collect();
        let mut captures: Vec<(usize, Val)> = Vec::new();
        // Capture once per canonical slot, skipping formal params. Linear scan over `chunk.names` beats a HashMap at typical body sizes (<30) and avoids a per-call monomorphisation.
        let mut seen_canonical: crate::util::fx::FxHashSet<usize> = crate::util::fx::FxHashSet::default();
        // Root the in-progress cells: each is reachable only via this local Vec until the Func is allocated, so a GC during the loop's own allocs could otherwise sweep them.
        let roots_base = self.temp_roots.len();
        for (bi, bname) in body.names.iter().enumerate() {
            if param_names.contains(bname.as_str()) { continue; }
            let canon = body.alias_groups.get(bi)
                .and_then(|g| g.first().copied())
                .unwrap_or(bi as u16) as usize;
            if !seen_canonical.insert(canon) { continue; }
            if let Some((si, _)) = chunk.names.iter().enumerate().find(|(_, n)| n.as_str() == bname.as_str())
                && let Some(&v) = slots.get(si)
                && !v.is_undef() {
                    // Capture a shared cell, not the raw value, so sibling closures over the same variable see each other's nonlocal writes. Key the registry by the parent slot `si` (stable across siblings), not the callee's `canon` (which differs per closure body).
                    let cell = self.frame_cell_for(si, v)?;
                    self.temp_roots.push(cell);
                    captures.push((canon, cell));
                }
        }

        let val = self.heap.alloc(HeapObj::Func(global, defaults, captures))?;
        self.temp_roots.truncate(roots_base);

        // Entry-chunk top-level defs go into `globals` so forward refs resolve at call time. Module-level defs stay in the module's bindings (via `fn_module[fi]`) to keep cross-module helpers with the same name isolated.
        if core::ptr::eq(chunk, self.chunk) {
            let name_idx = self.functions[global].3 as usize;
            if name_idx < chunk.names.len() {
                let bare = ssa_strip(&chunk.names[name_idx]).to_string();
                self.globals.insert(bare, val);
            }
        }

        self.push(val);
        Ok(())
    }

    // Closure cell: a 1-element heap list used as a shared mutable box. Sibling closures over the same enclosing variable capture the same cell, so a `nonlocal` write through one is visible in the others.
    fn make_cell(&mut self, v: Val) -> Result<Val, VmErr> {
        self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(vec![v]))))
    }
    fn cell_get(&self, cell: Val) -> Val {
        if cell.is_heap()
            && let HeapObj::List(rc) = self.heap.get(cell)
            && let Some(&v) = rc.borrow().first() {
                return v;
            }
        cell
    }
    // Reuse the current frame's cell for parent slot `si` (so sibling closures share) or create one seeded with `v`.
    fn frame_cell_for(&mut self, si: usize, v: Val) -> Result<Val, VmErr> {
        if let Some(frame) = self.call_stack.last()
            && let Some(&(_, cell)) = frame.cells.iter().find(|(s, _)| *s == si) {
                return Ok(cell);
            }
        let cell = self.make_cell(v)?;
        if let Some(frame) = self.call_stack.last_mut() { frame.cells.push((si, cell)); }
        Ok(cell)
    }

    /* `Call` orchestrator. Only user `Func` callees build a fresh `fn_slots` and run the body inline; every other callee kind short-circuits in `try_dispatch_non_func_callable`. */
    pub(crate) fn exec_call(&mut self, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Taken so nested native calls see false.
        let call_safe = core::mem::take(&mut self.pending_exec_safe);
        let (positional, kw_flat, _num_pos, num_kw) = self.parse_call_args(operand)?;

        if self.depth >= self.max_calls { return Err(cold_depth()); }
        // Charge each call so wide recursion is op-budget bounded.
        self.charge_step()?;

        let callee = self.pop()?;
        if !callee.is_heap() { return Err(cold_type("object is not callable")); }

        // Snapshot defaults/captures once, both are tiny (<10), and cloning beats the 3+ heap re-reads later phases would do. Back-prop still uses `get_mut` since it writes. Probing Func first skips the 9-shape non-func walk on the hottest (user function) path.
        let (fi, defaults, captures) = match self.heap.get(callee) {
            HeapObj::Func(i, d, c) => (*i, d.clone(), c.clone()),
            _ => {
                // Bound methods re-dispatch as tail calls.
                if matches!(self.heap.get(callee), HeapObj::BoundUserMethod(..)) {
                    self.pending_exec_safe = call_safe;
                }
                let dispatched = self.try_dispatch_non_func_callable(callee, &positional, &kw_flat, num_kw, chunk, slots);
                self.pending_exec_safe = false;
                if dispatched? {
                    return Ok(());
                }
                return Err(cold_type("object is not callable"));
            }
        };

        // Pure-call memoisation. Disabled under impure outer frames (stale-view risk) or kwargs (cache key only spans positionals).
        let outer_impure = self.observed_impure.last().copied().unwrap_or(false);
        if num_kw == 0 && !outer_impure
            && let Some(cached) = self.templates.lookup(fi, &positional, &self.heap) {
                self.push(cached);
                return Ok(());
        }

        self.depth += 1;
        let (_params, body, _, _) = self.functions[fi];
        // Reuse a pooled buffer: clear + bulk copy beats a fresh alloc per call.
        let mut fn_slots = self.slot_pool.pop().unwrap_or_default();
        fn_slots.clear();
        fn_slots.extend_from_slice(&self.slot_templates[fi]);

        self.bind_function_args(fi, &defaults, &captures, &positional, &kw_flat, &mut fn_slots)?;

        if self.needs_caller_slots[fi] {
            self.apply_caller_slot_propagation(fi, &captures, chunk, slots, &mut fn_slots);
        }

        self.bind_self_reference(fi, callee, &mut fn_slots);

        // Generator/coroutine: return a suspended Coroutine instead of running. Both flags are O(1).
        let is_async_fn = self.is_async.get(fi).copied().unwrap_or(false);
        if is_async_fn || body.is_generator {
            let coro = self.heap.alloc(HeapObj::Coroutine(0, fn_slots, Vec::new(), BodyRef::Fn(fi), Vec::new(), Vec::new(), Vec::new()))?;
            self.push(coro);
            self.depth -= 1;
            return Ok(());
        }

        // Snapshot caller-visible depths so we can split the helper's stack/iter/exception contributions out if it suspends mid-body via a yielding builtin.
        let stack_base = self.stack.len();
        let iter_base = self.iter_stack.len();
        let exc_base = self.exception_stack.len();
        let yields_before = self.yields.len();
        self.pending_exec_safe = call_safe;
        let (callee_impure, exec_result) = self.run_body_with_frame(fi, body, chunk, &mut fn_slots, slots);
        self.depth -= 1;

        self.back_propagate_nonlocals(fi, body, callee, chunk, slots, &fn_slots);

        let result = exec_result?;
        if callee_impure { self.mark_impure(); }

        if self.yielded {
            // Sync helper suspended mid-execution (e.g. `sleep(0)` from inside a sync fn called by an async coro). Stage its frame on the VM-level buffer; `resume_coroutine` drains it onto the enclosing coro so the helper is re-entered from the right ip. Without this, the outer's resume_ip would skip past the unfinished helper and the next StoreName would underflow. A nested sync call inside this helper would already have pushed its own frame first, so the buffer ends up innermost-last.
            let helper_resume_ip = self.resume_ip;
            self.resume_ip = 0;
            let helper_stack_delta = if self.stack.len() > stack_base { self.stack.split_off(stack_base) } else { Vec::new() };
            let helper_iter_delta: Vec<IterFrame> = if self.iter_stack.len() > iter_base { self.iter_stack.drain(iter_base..).collect() } else { Vec::new() };
            let helper_exc_delta: Vec<ExceptionFrame> = if self.exception_stack.len() > exc_base {
                self.exception_stack.drain(exc_base..)
                    .map(|mut f| {
                        f.stack_depth = f.stack_depth.saturating_sub(stack_base);
                        f.iter_depth = f.iter_depth.saturating_sub(iter_base);
                        f
                    })
                    .collect()
            } else { Vec::new() };
            self.pending_sync_frames.push(SyncFrame {
                ip: helper_resume_ip, fi, slots: fn_slots,
                stack_delta: helper_stack_delta, iter_delta: helper_iter_delta, exception_delta: helper_exc_delta,
            });
            return Ok(());
        }

        if self.yields.len() > yields_before {
            let fn_yields = self.yields.split_off(yields_before);
            let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(fn_yields))))?;
            self.push(val);
        } else {
            if num_kw == 0 && body.is_pure && !callee_impure {
                self.templates.record(fi, &positional, result, &self.heap);
            }
            self.push(result);
        }
        // Recycle the frame buffer; error/suspend paths above just drop theirs.
        if self.slot_pool.len() < 64 { self.slot_pool.push(fn_slots); }
        Ok(())
    }

    /* Decode `operand` (lo=positional, hi=kw pairs) + star-spread deltas, then pop into positional/kw buffers. kw is flat alternating name/value as the parser emits. */
    pub(crate) fn parse_call_args(&mut self, operand: u16) -> Result<(Vec<Val>, Vec<Val>, usize, usize), VmErr> {
        let raw = operand as usize;

        let base_pos = (raw & 0xFF) as i32;
        let base_kw = ((raw >> 8) & 0xFF) as i32;
        let num_pos = (base_pos + self.pending.pos_delta).max(0) as usize;
        let num_kw = (base_kw + self.pending.kw_delta ).max(0) as usize;
        self.pending.pos_delta = 0;
        self.pending.kw_delta = 0;

        let total_items = num_pos + 2 * num_kw;
        // Bulk-copy the stack tail: one memcpy instead of per-item pops plus a reverse.
        let at = self.stack.len().checked_sub(total_items).ok_or(cold_runtime("stack underflow"))?;
        let kw_flat: Vec<Val> = self.stack[at + num_pos..].to_vec();
        let mut positional = self.stack.split_off(at);
        positional.truncate(num_pos);
        Ok((positional, kw_flat, num_pos, num_kw))
    }

    /* Pack a flat `[name, val, name, val, ...]` slice into a heap dict for the trailing kwargs slot. `None` when there are no kwargs so the FFI layer can serialize handle 0 on the wire. */
    pub(crate) fn pack_kw_dict(heap: &mut super::super::types::HeapPool, kw_flat: &[Val]) -> Result<Option<Val>, VmErr> {
        if kw_flat.is_empty() { return Ok(None); }
        let dm = super::super::types::DictMap::from_pairs(kw_flat.chunks_exact(2).map(|p| (p[0], p[1])).collect(), heap);
        Ok(Some(heap.alloc(super::super::types::HeapObj::Dict(Rc::new(RefCell::new(dm))))?))
    }

    /* list.sort(): parse key/reverse kwargs and sort in place. Intercepted from both call paths since it needs chunk/slots for __lt__. */
    pub(crate) fn exec_sort(&mut self, recv: Val, positional: &[Val], kw_flat: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if !positional.is_empty() {
            return Err(cold_type("list.sort() takes no positional arguments"));
        }
        let mut sort_key: Option<Val> = None;
        let mut sort_reverse = false;
        for pair in kw_flat.chunks(2) {
            match self.kw_name(pair[0]) {
                Some("key") => sort_key = Some(pair[1]),
                Some("reverse") => sort_reverse = self.truthy(pair[1]),
                _ => return Err(cold_type("list.sort() got unexpected keyword argument")),
            }
        }
        self.call_list_sort_keyed(recv, sort_key, sort_reverse, chunk, slots)
    }

    /* Dispatch non-Func callees. Returns Ok(true) when handled here; Ok(false) means the caller falls through to the Func path. */
    fn try_dispatch_non_func_callable(&mut self, callee: Val, positional: &[Val], kw_flat: &[Val], num_kw: usize, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if let HeapObj::BoundMethod(recv, id) = self.heap.get(callee) {
            let recv = *recv;
            let id = *id;
            if id.name() == "sort" {
                self.exec_sort(recv, positional, kw_flat, chunk, slots)?;
                return Ok(true);
            }
            self.exec_bound_method(recv, id, positional, kw_flat)?;
            return Ok(true);
        }

        if let HeapObj::NativeFn(id) = self.heap.get(callee) {
            let id = *id;
            // First-class builtins (e.g. `apply(print, x)`) bypass the CallPrint/CallInput opcodes; mark here so a pure wrapper around them isn't memoised.
            if native_is_impure(id) { self.mark_impure(); }
            self.dispatch_native(id, positional, kw_flat, chunk, slots)?;
            return Ok(true);
        }

        if let HeapObj::Extern(extern_fn) = self.heap.get(callee) {
            let func = extern_fn.func.clone();
            let pure = extern_fn.pure;
            if !pure { self.mark_impure(); }
            let kwargs = Self::pack_kw_dict(&mut self.heap, kw_flat)?;
            // Park on host-deferral like `call_extern` (e.g. `time.sleep` via module attr).
            match func(&mut self.heap, positional, kwargs) {
                Ok(result) => self.push(result),
                Err(VmErr::HostCallDeferred) => self.park_host_call(),
                Err(e) => return Err(e),
            }
            return Ok(true);
        }

        if let HeapObj::Type(name) = self.heap.get(callee) {
            let name = name.clone();
            if let Some(id) = constructor_native(&name) {
                self.dispatch_native(id, positional, kw_flat, chunk, slots)?; // int/set/list/... construct
                return Ok(true);
            }
            // `object()`: unique featureless instance.
            if name == "object" {
                if !positional.is_empty() || !kw_flat.is_empty() {
                    return Err(cold_type("object() takes no arguments"));
                }
                let inst = self.heap.alloc(HeapObj::Instance(callee, Rc::new(RefCell::new(DictMap::new()))))?;
                self.push(inst);
                return Ok(true);
            }
            // Other Type objects are exception classes: build an ExcInstance for `raise X("msg")`.
            if !kw_flat.is_empty() {
                return Err(cold_type("exception class takes no keyword arguments"));
            }
            let exc = self.heap.alloc(HeapObj::ExcInstance(name, positional.to_vec()))?;
            self.push(exc);
            return Ok(true);
        }

        // Calling a class: create an instance and run `__init__` if defined (walks bases).
        if let HeapObj::Class(..) = self.heap.get(callee) {
            // The recursive `exec_call` below only encodes positional count, kwargs would silently disappear before reaching `__init__`, so reject them here.
            if !kw_flat.is_empty() {
                return Err(cold_type("class constructor takes no keyword arguments"));
            }
            let instance = self.heap.alloc(HeapObj::Instance(callee, Rc::new(RefCell::new(DictMap::new()))))?;
            if let Some((init_fn, defining)) = self.lookup_class_member(callee, "__init__") {
                // Fail-fast before pushing, the inner check fires only after parse_call_args pops.
                if self.depth >= self.max_calls { return Err(cold_depth()); }
                self.pending.method_binding = Some((defining, instance));
                self.push(init_fn);
                self.push(instance);
                for a in positional { self.push(*a); }
                let argc = (1 + positional.len()) as u16;
                self.exec_call(argc, chunk, slots)?;
                // Discard `__init__` return value.
                self.pop()?;
            }
            self.push(instance);
            return Ok(true);
        }

        // Bound user method: prepend `self` to the arg list and re-dispatch.
        if let HeapObj::BoundUserMethod(recv, func, class) = self.heap.get(callee) {
            // Same as Class branch: depth check before mutating the stack.
            if self.depth >= self.max_calls { return Err(cold_depth()); }
            let (recv, func, class) = (*recv, *func, *class);
            self.pending.method_binding = Some((class, recv));
            self.push(func);
            self.push(recv);
            for a in positional { self.push(*a); }
            // Re-push keyword pairs too, else the re-dispatch underflows the stack.
            for k in kw_flat { self.push(*k); }
            let argc = (positional.len() + 1) as u16;
            let encoded = ((num_kw as u16) << 8) | argc;
            self.exec_call(encoded, chunk, slots)?;
            return Ok(true);
        }

        // `prop.setter(fn)` returns a new `Property` carrying the original getter plus the supplied setter.
        if let HeapObj::PropertySetter(prop_val) = self.heap.get(callee) {
            if positional.len() != 1 || !kw_flat.is_empty() {
                return Err(cold_type("property.setter takes exactly 1 argument"));
            }
            let prop_val = *prop_val;
            let getter = match self.heap.get(prop_val) {
                HeapObj::Property(g, _) => *g,
                _ => return Err(cold_runtime("PropertySetter wraps a non-Property value")),
            };
            let new_setter = positional[0];
            let new_prop = self.heap.alloc(HeapObj::Property(getter, new_setter))?;
            self.push(new_prop);
            return Ok(true);
        }

        // Instance with `__call__`, bind and dispatch through `BoundUserMethod`-style flow.
        if let HeapObj::Instance(..) = self.heap.get(callee)
            && let Some((func, class)) = self.lookup_class_member(
                match self.heap.get(callee) { HeapObj::Instance(c, _) => *c, _ => unreachable!() },
                "__call__")
        {
            if self.depth >= self.max_calls { return Err(cold_depth()); }
            self.pending.method_binding = Some((class, callee));
            self.push(func);
            self.push(callee);
            for a in positional { self.push(*a); }
            for k in kw_flat { self.push(*k); }
            let argc = (positional.len() + 1) as u16;
            let encoded = ((num_kw as u16) << 8) | argc;
            self.exec_call(encoded, chunk, slots)?;
            return Ok(true);
        }

        if let HeapObj::Coroutine(..) = self.heap.get(callee) {
            // Plain `async def` (no `yield`): drive to completion via the scheduler (await semantics). Async *generators* fall through to step-wise resume like sync generators.
            let drive_async = matches!(self.heap.get(callee),
                HeapObj::Coroutine(_, _, _, super::super::types::BodyRef::Fn(fi), ..)
                    if self.is_async.get(*fi).copied().unwrap_or(false) && !self.functions[*fi].1.is_generator);
            if drive_async {
                self.await_coroutine(callee)?;
                return Ok(true);
            }
            // Generator stepping (ForIter calls here per `next`): resume one step; the inner yield must NOT propagate to the caller.
            let result = self.resume_coroutine(callee)?;
            if self.yielded { self.yielded = false; }
            self.push(result);
            return Ok(true);
        }

        Ok(false)
    }

    /* Bind formal params from positional/kw buffers, then fill remaining undef slots with defaults and captures. `defaults`/`captures` are pre-snapshotted by `exec_call`. */
    fn bind_function_args(&mut self, fi: usize, defaults: &[Val], captures: &[(usize, Val)], positional: &[Val], kw_flat: &[Val], fn_slots: &mut [Val]) -> Result<(), VmErr> {
        // Index by position to avoid an iterator borrow on `param_slots` across `heap.alloc`.
        let n_params = self.param_slots[fi].len();
        // Without a `*args` sink, positionals past the normal params are an error.
        let has_star = self.param_slots[fi].iter().any(|(k, _)| matches!(k, ParamKind::Star));
        let normal_count = self.param_slots[fi].iter().filter(|(k, _)| matches!(k, ParamKind::Normal)).count();
        if !has_star && positional.len() > normal_count {
            return Err(cold_type("too many positional arguments"));
        }
        let mut pos_idx = 0usize;
        for i in 0..n_params {
            let (kind, slot) = self.param_slots[fi][i];
            match kind {
                ParamKind::DoubleStar => {
                    // **kwargs gets only keys not bound to a named param.
                    let params = &self.functions[fi].0;
                    let pairs: Vec<(Val, Val)> = kw_flat.chunks_exact(2)
                        .filter(|p| match self.heap.try_get(p[0]) {
                            Some(HeapObj::Str(s)) => !params.iter().any(|pp| !pp.starts_with('*') && crate::modules::parser::types::param_base_name(pp) == s.as_str()),
                            _ => true,
                        })
                        .map(|p| (p[0], p[1])).collect();
                    let dm = DictMap::from_pairs(pairs, &self.heap);
                    let dict_val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?;
                    if slot < fn_slots.len() { fn_slots[slot] = dict_val; }
                }
                ParamKind::Star => {
                    // *args binds to an immutable tuple.
                    let rest: Vec<Val> = positional[pos_idx..].to_vec();
                    pos_idx = positional.len();
                    let tuple_val = self.heap.alloc(HeapObj::Tuple(rest))?;
                    if slot < fn_slots.len() { fn_slots[slot] = tuple_val; }
                }
                ParamKind::Normal => {
                    if pos_idx >= positional.len() { continue; }
                    if slot < fn_slots.len() { fn_slots[slot] = positional[pos_idx]; }
                    pos_idx += 1;
                }
                // KwOnly slots are NOT consumed positionally; they bind only via kwargs.
                ParamKind::KwOnly => {}
            }
        }

        // Kwargs binding (rare path, not optimised).
        if !kw_flat.is_empty() {
            let params = &self.functions[fi].0;
            let body_map = &self.body_maps[fi];
            for pair in kw_flat.chunks_exact(2) {
                // Malformed `**`/kwarg bytecode can leave a non-string in the name slot; guard the heap access.
                let key = match self.heap.try_get(pair[0]) {
                    Some(HeapObj::Str(s)) => s.clone(),
                    _ => return Err(cold_runtime("malformed kwarg on stack")),
                };
                // Star/double-star params are not keyword targets; a kwarg whose name matches `*a`/`**k` goes to **kwargs.
                if params.iter().any(|p| !p.starts_with('*') && crate::modules::parser::types::param_base_name(p) == key.as_str()) {
                    let pname = s!(str &key, "_0");
                    if let Some(&s) = body_map.get(pname.as_str()) {
                        fn_slots[s] = pair[1];
                    }
                }
            }
        }

        // Defaults: only fill slots still undef after binding.
        if !defaults.is_empty() {
            let ds = &self.default_slots[fi];
            for (di, &dv) in defaults.iter().enumerate() {
                if let Some(&(slot, _)) = ds.get(di)
                    && slot < fn_slots.len() && fn_slots[slot].is_undef() {
                        fn_slots[slot] = dv;
                    }
            }
        }

        // Closure captures: same rule as defaults, only fill if undef. Each capture is a shared cell; read its current value into the slot.
        for &(bi, cell) in captures {
            if bi < fn_slots.len() && fn_slots[bi].is_undef() {
                fn_slots[bi] = self.cell_get(cell);
            }
        }

        Ok(())
    }

    /* Push caller slots into body slots. Same scope: late-binding, overwrite freely. Different scope: skip capture-filled slots (fixes stacked-decorator clobber). */
    fn apply_caller_slot_propagation(&mut self, fi: usize, captures: &[(usize, Val)], chunk: &SSAChunk, slots: &[Val], fn_slots: &mut [Val]) {
        let info = self.propagation_map(fi, chunk);
        let captured_set: crate::util::fx::FxHashSet<usize> = if info.same_scope {
            crate::util::fx::FxHashSet::default()
        } else {
            captures.iter().map(|(s, _)| *s).collect()
        };
        // Undef/captured filters stay per-call; the name matching is cached.
        for &(si, bs) in info.pairs.iter() {
            let bs = bs as usize;
            if let Some(&v) = slots.get(si as usize)
                && !v.is_undef()
                && !captured_set.contains(&bs)
            {
                fn_slots[bs] = v;
            }
        }

        // Bare-name fallback: body refs `<base>_0` but caller may store a higher SSA version. Layer 1 (caller's latest live version) runs off the cached candidates; misses fall to the shared slow layers.
        for (bare, bs, versions) in info.free.iter() {
            let bs = *bs as usize;
            if captured_set.contains(&bs) { continue; }
            let mut latest_ver: i64 = -1;
            let mut latest_v: Val = Val::undef();
            for &(v, si) in versions.iter() {
                let si = si as usize;
                if si < slots.len() && !slots[si].is_undef() && v > latest_ver {
                    latest_ver = v;
                    latest_v = slots[si];
                }
            }
            if !latest_v.is_undef() {
                fn_slots[bs] = latest_v;
            } else if let Some(v) = self.resolve_free_name_fallback(fi, bare) {
                fn_slots[bs] = v;
            }
        }
    }

    /* Build or fetch the static propagation info for (chunk, fi). Chunks are borrowed for the VM's lifetime, so the pointer key is stable. */
    fn propagation_map(&mut self, fi: usize, chunk: &SSAChunk) -> super::super::PropagationMap {
        let key = (chunk as *const SSAChunk, fi);
        if let Some(m) = self.propagation_maps.get(&key) { return m.clone(); }
        let body_map = &self.body_maps[fi];
        let param_bm = &self.is_param_slot[fi];
        let pairs: Vec<(u32, u32)> = chunk.names.iter().enumerate()
            .filter_map(|(si, name)| {
                let &bs = body_map.get(name.as_str())?;
                if param_bm.get(bs).copied().unwrap_or(false) { return None; }
                Some((si as u32, bs as u32))
            })
            .collect();
        // Same-scope also requires same module, keeps top-level imports (`parent_fi == None`) isolated.
        let caller_fi = self.body_to_fi.get(&(chunk as *const _)).copied();
        let callee_parent_fi = self.function_parents.get(fi).and_then(|x| *x);
        let caller_module = caller_fi.and_then(|cf| self.fn_module.get(cf).and_then(|m| m.as_deref()));
        let callee_module = self.fn_module.get(fi).and_then(|m| m.as_deref());
        let same_scope = caller_fi == callee_parent_fi && caller_module == callee_module;
        // Free loads with their caller-chunk (version, slot) candidates resolved once.
        let name_index = self.chunk_name_versions.get(&(chunk as *const _));
        let free: Vec<super::super::FreeLoadEntry> = self.body_free_loads[fi].iter()
            .map(|(bare, bs)| {
                let versions = name_index
                    .and_then(|idx| idx.get(bare.as_str()))
                    .map(|v| v.iter().map(|&(ver, si)| (ver, si as u32)).collect())
                    .unwrap_or_default();
                (bare.clone(), *bs as u32, versions)
            })
            .collect();
        let map: super::super::PropagationMap = alloc::rc::Rc::new(super::super::PropInfo { same_scope, pairs, free });
        self.propagation_maps.insert(key, map.clone());
        map
    }

    /* Slow layers for a bare free-load name after the caller-slot layer missed: callee module attrs -> entry globals -> entry module state. First hit wins. Centralised so the order is auditable. */
    fn resolve_free_name_fallback(&self, fi: usize, bare: &str) -> Option<Val> {
        // Layer 2: callee's module attrs, keeps `a.helper` and `b.helper` isolated.
        if let Some(Some(spec)) = self.fn_module.get(fi).cloned()
            && let Some(mod_val) = self.module_table.get(&spec).copied()
            && mod_val.is_heap()
            && let HeapObj::Module(_, attrs) = self.heap.get(mod_val)
            && let Some((_, v)) = attrs.iter().find(|(n, _)| n == bare)
        {
            return Some(*v);
        }
        // Layer 3: globals, catches forward-ref mutual recursion in the entry chunk.
        if let Some(&v) = self.globals.get(bare) {
            return Some(v);
        }
        // Layer 4: entry-module bindings; classes and plain vars never reach `globals`.
        if self.fn_module.get(fi).is_none_or(|m| m.is_none())
            && let Some(&v) = self.module_state.get(bare)
            && !v.is_undef()
        {
            return Some(v);
        }
        None
    }

    /* Bind the function's own name slot to `callee` so recursive calls skip the global lookup. No-op for lambdas or when an earlier phase already filled the slot. */
    fn bind_self_reference(&self, fi: usize, callee: Val, fn_slots: &mut [Val]) {
        if let Some(slot) = self.self_ref_slot.get(fi).copied().flatten()
            && slot < fn_slots.len()
            && fn_slots[slot].is_undef()
        {
            fn_slots[slot] = callee;
        }
    }

    /* Run the body with caller slots pinned in `live_slots` (GC roots) and a CallFrame on `call_stack` (traceback). Frame popped on success only; the dispatch catch clears it on swallowed exceptions. Returns `(callee_impure, exec_result)`. */
    fn run_body_with_frame(&mut self, fi: usize, body: &SSAChunk, chunk: &SSAChunk, fn_slots: &mut [Val], slots: &[Val]) -> (bool, Result<Val, VmErr>) {
        // GC roots come from `active_slots` (every live exec frame); `live_slots` only feeds `globals()`, which reads the entry chunk's slots at the bottom. Copy just that frame.
        let snap = self.live_slots.len();
        if snap == 0 && core::ptr::eq(chunk, self.chunk) {
            self.live_slots.extend_from_slice(slots);
        }

        // Frame snapshots caller's source/path so render doesn't borrow live chunk pointers.
        let call_byte_pos = self.pending.call_byte_pos.take().unwrap_or(0);
        // Method-call paths set `method_binding` immediately before invoking `exec_call`; plain function calls leave it `None`.
        let (current_class, current_self) = match self.pending.method_binding.take() {
            Some((c, s)) => (Some(c), Some(s)),
            None => (None, None),
        };
        self.call_stack.push(super::super::types::CallFrame {
            fi,
            call_byte_pos,
            caller_source: chunk.source.clone(),
            caller_path: chunk.path.clone(),
            current_class,
            current_self,
            cells: Vec::new(),
        });

        self.observed_impure.push(false);
        let exec_result = self.exec(body, fn_slots);
        let callee_impure = self.observed_impure.pop().unwrap_or(true);
        self.live_slots.truncate(snap);
        if exec_result.is_ok() {
            self.call_stack.pop();
        }
        (callee_impure, exec_result)
    }

    /* Back-propagate `nonlocal` writes to the caller's slots and sync the callee Func's capture entries so the next call sees the new value. No-op if no `nonlocal`. */
    fn back_propagate_nonlocals(&mut self, fi: usize, body: &SSAChunk, callee: Val, chunk: &SSAChunk, slots: &mut [Val], fn_slots: &[Val]) {
        if self.nonlocal_tables[fi].is_empty() { return; }
        // Snapshot to release borrows on self before the `heap.get_mut` writes.
        let nl_pairs: Vec<(usize, usize)> = self.nonlocal_tables[fi].clone();
        let name_index = self.chunk_name_versions.get(&(chunk as *const _));
        for (canon_body, _) in nl_pairs {
            if let Some(&val) = fn_slots.get(canon_body) {
                if val.is_undef() { continue; }
                for base in &body.nonlocals {
                    if let Some(idx) = name_index && let Some(versions) = idx.get(base.as_str())
                    {
                        for &(_, si) in versions {
                            if si < slots.len() { slots[si] = val; }
                        }
                    }
                    // Write into the shared cell so sibling closures over this variable observe the nonlocal write. Access `self.heap` directly (not via &mut self helpers) so it stays disjoint from the `name_index` borrow above.
                    let cell = if let HeapObj::Func(_, _, caps) = self.heap.get(callee) {
                        caps.iter().find(|(ci, _)| *ci == canon_body).map(|(_, c)| *c)
                    } else { None };
                    match cell {
                        Some(c) => if let HeapObj::List(rc) = self.heap.get(c) {
                            let mut b = rc.borrow_mut();
                            if b.is_empty() { b.push(val); } else { b[0] = val; }
                        },
                        // Nonlocal target not captured at MakeFunction (rare): attach a fresh cell so the next call sees it.
                        None => if let Ok(c) = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(vec![val]))))
                            && let HeapObj::Func(_, _, caps) = self.heap.get_mut(callee) {
                                caps.push((canon_body, c));
                            },
                    }
                }
            }
        }
    }


    /* CallExtern: operand packs `(extern_idx<<8)|(kw<<4)|pos`. Pop kw `name,val` pairs then `pos` positional vals, pack pairs into a heap dict via `pack_kw_dict` and hand it off as the explicit `Option<Val>` kwargs slot. Pure externs leave the impurity flag alone, bodies whose only side-effects are pure externs stay memoizable. */
    pub(crate) fn call_extern(&mut self, operand: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let extern_idx = (operand >> 8) as usize;
        let kw = ((operand >> 4) & 0xF) as usize;
        let pos = (operand & 0xF) as usize;
        let extern_fn = chunk.extern_table.get(extern_idx).ok_or(cold_runtime("CallExtern: extern index out of bounds"))?;
        let func = extern_fn.func.clone(); // Arc clone, refcount bump only
        let pure = extern_fn.pure;
        let kw_flat = if kw > 0 { self.pop_n(kw * 2)? } else { Vec::new() };
        let positional = self.pop_n(pos)?;
        let kwargs = Self::pack_kw_dict(&mut self.heap, &kw_flat)?;
        if !pure { self.mark_impure(); }
        match func(&mut self.heap, &positional, kwargs) {
            Ok(result) => { self.push(result); Ok(()) }
            Err(VmErr::HostCallDeferred) => { self.park_host_call(); Ok(()) }
            Err(e) => Err(e),
        }
    }

    /* Park a native that deferred to the host: `None` placeholder (overwritten by `set_host_result_by_id`), correlation id, yield. */
    fn park_host_call(&mut self) {
        self.push(Val::none());
        self.pending.host_call_id = self.next_host_call_id;
        self.next_host_call_id += 1;
        self.pending.host_call_request = true;
        self.yielded = true;
    }

    pub(crate) fn dispatch_native(&mut self, id: super::super::types::NativeFnId, positional: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use super::super::types::NativeFnId::*;

        // `sorted()` is the only builtin taking kwargs (`key=`, `reverse=`); extract them before the no-kw guard.
        let mut sort_key: Option<Val> = None;
        let mut sort_reverse = false;
        let leftover_storage: Vec<Val>;
        let kw_remaining: &[Val] = if id == Sorted {
            let mut leftover: Vec<Val> = Vec::new();
            for chunk_pair in kw.chunks(2) {
                let (name_v, val_v) = (chunk_pair[0], chunk_pair[1]);
                match self.kw_name(name_v) {
                    Some("key") => sort_key = Some(val_v),
                    Some("reverse") => sort_reverse = self.truthy(val_v),
                    _ => { leftover.push(name_v); leftover.push(val_v); }
                }
            }
            leftover_storage = leftover;
            &leftover_storage
        } else { kw };

        if !kw_remaining.is_empty() {
            return Err(cold_type("native function takes no keyword arguments"));
        }
        let argc = positional.len() as u16;

        // Pre-validate fixed arity to keep the stack clean on error.
        if let Some(n) = id.arity()
            && argc != n { return Err(cold_type("wrong number of arguments to builtin")); }

        for &v in positional { self.push(v); }

        match id {
            // Variadic
            Print => {
                // CallPrint is statement-shaped (no trailing Pop); when reached via Call the parser emits Pop, so push None to keep the stack balanced.
                self.call_print(argc, chunk, slots)?;
                self.push(Val::none());
                Ok(())
            }
            Range => self.call_range(argc),
            Round => self.call_round(argc),
            Min => self.call_min(argc, chunk, slots),
            Max => self.call_max(argc, chunk, slots),
            Sum => self.call_sum(argc),
            Zip => self.call_zip(argc),
            Dict => self.call_dict(argc),
            Set => self.call_set(argc),
            Pow => self.call_pow(argc),
            All => self.call_all(argc),
            Any => self.call_any(argc),
            GetAttr => self.call_getattr(argc),
            Format => self.call_format(argc, chunk, slots),
            // 0/1/2-arg
            Input => self.call_input(),
            Len => self.call_len(chunk, slots),
            Abs => self.call_abs(),
            Str => self.call_str(argc, chunk, slots),
            Int => self.call_int(argc, chunk, slots),
            Float => self.call_float(argc),
            Bool => self.call_bool(argc, chunk, slots),
            Type => self.call_type(),
            Chr => self.call_chr(),
            Ord => self.call_ord(),
            Sorted => self.call_sorted_with_key(sort_key, sort_reverse, chunk, slots),
            Enumerate => self.call_enumerate(argc),
            List => self.call_list(argc, chunk, slots),
            Tuple => self.call_tuple(argc, chunk, slots),
            Bin => self.call_bin(),
            Oct => self.call_oct(),
            Hex => self.call_hex(),
            Repr => self.call_repr(chunk, slots),
            Reversed => self.call_reversed(),
            Callable => self.call_callable(),
            Id => self.call_id(),
            Hash => self.call_hash(chunk, slots),
            Divmod => self.call_divmod(),
            IsInstance => self.call_isinstance(),
            IsSubclass => self.call_issubclass(),
            HasAttr => self.call_hasattr(),
            Next => self.call_next(argc, chunk, slots),
            Run => self.call_run(argc),
            Sleep => self.call_sleep(),
            Frame => self.call_frame(),
            Receive => self.call_receive(),
            Map => self.call_map(argc, chunk, slots),
            Filter => self.call_filter(chunk, slots),
            Iter => self.call_iter(argc, chunk, slots),
            Bytes => self.call_bytes(argc),
            Slice => self.call_slice(argc),
            Vars => self.call_vars(),
            SetAttr => self.call_setattr(),
            DelAttr => self.call_delattr(),
            ImportModule => self.call_import_module(),
            Gather => self.call_gather(argc),
            WithTimeout => self.call_with_timeout(),
            Cancel => self.call_cancel(),
            BytesFromHex => self.call_bytes_fromhex(),
            IntFromBytes => self.call_int_from_bytes(),
            IntToBytes => self.call_int_to_bytes(),
            FrozenSet => self.call_frozenset(argc),
            Globals => self.call_globals(chunk, slots),
            Locals => self.call_locals(chunk, slots),
            Super => self.call_super(),
            Property => self.call_property(argc),
            StaticMethod => self.call_staticmethod(argc),
        }
    }
}
