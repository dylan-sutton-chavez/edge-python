use crate::s;
use super::*;
use super::super::ParamKind;
use crate::parser::fused_native;

use crate::alloc::string::ToString;

// Builtin conversion-type name -> its constructor, None for exception/other types.
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

// Effects/mutation/scheduling/import/reflection/nondeterminism. Over-marking only forgoes memoisation, under-marking drops effects.
fn native_is_impure(id: super::super::types::NativeFnId) -> bool {
    use super::super::types::NativeFnId::*;
    matches!(id,
        Print | Input | Receive | SendMsg | Sleep // I/O + scheduler
        | SetAttr | DelAttr // mutation
        | GetAttr | HasAttr // attr access can run getters
        | Run | ImportModule // arbitrary execution / import
        | Cancel | WithTimeout | Gather // async effects
        | Globals | Locals | Vars | Super // reflection of mutable state
        | Id // heap-slot nondeterminism
    )
}

/* Builtins that iterate an argument, where a user `__iter__` may stand in. */
#[inline]
fn iterates(id: super::super::types::NativeFnId) -> bool {
    use super::super::types::NativeFnId::*;
    matches!(id, Sum | Sorted | Set | FrozenSet | Enumerate | Any | All | Bytes | Dict | Min | Max | Iter | Map | Filter | Zip)
}

/* A count outside a builtin arity, ranged ones name the bound they miss. */
#[cold]
fn arity_err(id: super::super::types::NativeFnId, n: u16) -> VmErr {
    let (lo, hi) = id.arity();
    if lo == hi { return cold_type("wrong number of arguments to builtin"); }
    let (word, bound) = if n < lo { ("least", lo) } else { ("most", hi) };
    VmErr::TypeMsg(crate::s!(str id.name(), " expected at ", str word, " ", int bound, " argument", str if bound == 1 { "" } else { "s" }, ", got ", int n))
}

impl<'a> VM<'a> {
    /* Dispatch every function-shaped opcode (Call, MakeFunction, builtins). */
    pub(crate) fn handle_function(&mut self, op: OpCode, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Only a fused builtin carries these flags, `Call` and the make-ops use the bits as counts.
        if operand & (crate::parser::SPREAD_ARGS | crate::parser::KEYWORDS) != 0
            && !matches!(op, OpCode::Call | OpCode::CallSpread | OpCode::CallExtern | OpCode::MakeFunction | OpCode::MakeCoroutine) {
            let (pos, kw) = ((operand & 0xFF) as usize, ((operand >> 8) & 0x3F) as usize);
            let (pos, kw) = if operand & crate::parser::SPREAD_ARGS != 0 { self.close_spread(pos, kw) } else { (pos, kw) };
            // A packed op stays fused while its counts fit, the rest run as a plain call.
            let packed = matches!(op, OpCode::CallPrint | OpCode::CallDict | OpCode::CallMin | OpCode::CallMax | OpCode::CallEnumerate);
            let fits = pos <= 0xFF && fused_native(op).is_some_and(|id| id.takes(pos as u16));
            if fits && packed && kw <= 0x3F { return self.handle_function(op, crate::parser::pack_call(pos as u16, kw as u16), chunk, slots); }
            if fits && op == OpCode::CallRange && kw == 0 { return self.handle_function(op, pos as u16, chunk, slots); }
            return self.call_spread_builtin(op, pos, kw, chunk, slots);
        }
        // A module-scope rebind of a builtin name must win over call sites fused before it existed. Plain fused operands are a bare count, so counts past one byte fall back to the native (they cannot round-trip through `exec_call`'s packing).
        let packed_operand = matches!(op, OpCode::CallPrint | OpCode::CallDict | OpCode::CallMin | OpCode::CallMax | OpCode::CallEnumerate);
        if self.builtins_rebound
            && (packed_operand || operand <= 0xFF)
            && let Some(name) = fused_native(op).map(|id| id.name())
            && let Some(&bound) = self.module_state.get(name)
            && !bound.is_undef()
            && !(bound.is_heap() && matches!(self.heap.get(bound), HeapObj::NativeFn(id) if id.name() == name))
        {
            return self.call_rebound(bound, (operand & 0xFF) as usize, ((operand >> 8) & 0xFF) as usize, chunk, slots);
        }
        // Fused builtins skip dispatch_native, the parser checked their counts and user iterables lift here.
        if let Some(id) = fused_native(op) {
            if iterates(id) {
                let (pos, kw) = if packed_operand { (operand & 0xFF, operand >> 8) } else { (operand, 0) };
                self.lift_builtin_args(id, pos as usize, kw as usize, chunk, slots)?;
            }
            return self.run_native(id, operand, chunk, slots);
        }
        match op {
            OpCode::Call => self.exec_call(operand, chunk, slots),
            OpCode::CallSpread => {
                let (pos, kw) = self.close_spread((operand & 0xFF) as usize, ((operand >> 8) & 0xFF) as usize);
                self.exec_call_n(pos, kw, chunk, slots)
            }
            OpCode::MakeFunction | OpCode::MakeCoroutine => self.exec_make_function(op, operand, chunk, slots),
            OpCode::CallExtern => self.call_extern(operand, chunk),
            _ => Err(cold_runtime("non-function opcode in handle_function")),
        }
    }

    /* A builtin method call, `sort`, `str.format` and user iterables run user code so they take the frame here. */
    pub(crate) fn exec_bound_method(&mut self, recv: Val, id: crate::vm::methods::BuiltinMethodId, pos: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use crate::vm::methods::MethodKind;
        match id.kind() {
            MethodKind::Sort => self.exec_sort(recv, pos, kw, chunk, slots),
            MethodKind::Format if kw.is_empty() => crate::vm::methods::string::format(self, recv, pos, chunk, slots),
            MethodKind::Search if id.ty() == "list" && kw.is_empty() => {
                crate::vm::methods::method_frame(self, id, pos.len())?;
                self.with_roots(pos.iter().copied().chain([recv]), |vm| {
                    crate::vm::methods::list::search(vm, recv, id.name(), pos, |vm, a, b| vm.member_eq(a, b, chunk, slots))
                })
            }
            MethodKind::Iterates if pos.iter().any(|&a| matches!(self.heap.try_get(a), Some(HeapObj::Instance(..)))) => {
                // Each lifted list stays rooted while later arguments run their `__iter__`.
                self.with_roots(pos.iter().copied().chain([recv]), |vm| {
                    let mut args = pos.to_vec();
                    for a in &mut args {
                        if let Some(l) = vm.lift_iterable(*a, chunk, slots)? { vm.temp_roots.push(l); *a = l; }
                    }
                    vm.builtin_method(recv, id, &args, kw, chunk, slots)
                })
            }
            _ => self.builtin_method(recv, id, pos, kw, chunk, slots),
        }
    }

    /* A dict or set method meeting user keys runs in the VM, every other one through its table entry. */
    #[inline]
    fn builtin_method(&mut self, recv: Val, id: crate::vm::methods::BuiltinMethodId, pos: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // `dict.fromkeys` reaches here with the type as its receiver.
        let table = recv.is_heap() && matches!(self.heap.get(recv), HeapObj::Dict(_) | HeapObj::Set(_) | HeapObj::Type(_));
        if table && self.keyed_method(id, recv, pos, kw, chunk, slots)? { return Ok(()); }
        crate::vm::methods::dispatch_method(self, id, recv, pos, kw)
    }

    /* `str.lower(s)`, an unbound builtin method takes a receiver of its own type as the first argument. */
    pub(crate) fn exec_unbound_method(&mut self, id: crate::vm::methods::BuiltinMethodId, args: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let Some((&recv, rest)) = args.split_first() else {
            return Err(VmErr::TypeMsg(crate::s!("unbound method ", str id.ty(), ".", str id.name(), "() needs an argument")));
        };
        // An exception instance takes the `BaseException` methods its builtin base gives it.
        let fits = self.type_name(recv) == id.ty()
            || (id.ty() == "BaseException" && matches!(self.heap.try_get(recv), Some(&HeapObj::Instance(c, _)) if self.exc_base(c).is_some()));
        if !fits {
            return Err(VmErr::TypeMsg(crate::s!("descriptor '", str id.name(), "' for '", str id.ty(), "' objects doesn't apply to a '", str self.type_name(recv), "' object")));
        }
        self.exec_bound_method(recv, id, rest, kw, chunk, slots)
    }

    /* Lifts the user iterables among `pos` positional args under `kw` keyword pairs, wherever builtin `id` iterates. */
    fn lift_builtin_args(&mut self, id: super::super::types::NativeFnId, pos: usize, kw: usize, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use super::super::types::NativeFnId::*;
        let (from, to) = match id {
            Sum | Sorted | Set | FrozenSet | Enumerate | Any | All | Bytes | Dict => (0, 1),
            Min | Max | Iter if pos == 1 => (0, 1),
            Map => (1, pos),
            Filter => (1, 2),
            Zip => (0, pos),
            _ => return Ok(()),
        };
        let base = self.stack.len().saturating_sub(pos + 2 * kw);
        for i in from..to.min(pos) {
            if let Some(&v) = self.stack.get(base + i)
                && let Some(l) = self.lift_iterable(v, chunk, slots)? { self.stack[base + i] = l; }
        }
        Ok(())
    }

    fn exec_make_function(&mut self, opcode: OpCode, operand: u16, chunk: &SSAChunk, slots: &[Val]) -> Result<(), VmErr> {
        let chunk_ptr = chunk as *const _;
        let global = self.fn_index.iter()
            .find(|(p, _)| *p == chunk_ptr)
            .and_then(|(_, v)| v.get(operand as usize).copied())
            .ok_or_else(|| cold_runtime("MakeFunction: unknown function index"))? as usize;

        if opcode == OpCode::MakeCoroutine {
            if self.is_async.len() <= global { self.is_async.resize(global + 1, false); }
            self.is_async[global] = true;
        }

        let n_defaults = self.functions[global].2 as usize;
        let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };

        let (params, body, _, _) = self.functions[global];
        let param_names: crate::util::hash::FxHashSet<String> = params.iter().map(|p| s!(str crate::parser::types::param_base_name(p), "_0")).collect();
        let mut captures: Vec<(usize, Val)> = Vec::new();
        // Cells only make sense for variables of an enclosing FUNCTION scope. Module and class bodies are late-bound, their names resolve live at call time, never freeze.
        let defined_in_fn = self.body_to_fi.contains_key(&chunk_ptr);
        let parent_locals = if defined_in_fn { Some(self.chunk_locals(chunk)) } else { None };
        // Capture once per canonical slot, skipping formal params. Linear scan over `chunk.names` beats a HashMap at typical body sizes (<30) and avoids a per-call monomorphisation.
        let mut seen_canonical: crate::util::hash::FxHashSet<usize> = crate::util::hash::FxHashSet::default();
        // Root the in-progress cells, each is reachable only via this local Vec until the Func is allocated, so a GC during the loop's own allocs could otherwise sweep them.
        let roots_base = self.temp_roots.len();
        if let Some(locals) = parent_locals {
            for (bi, bname) in body.names.iter().enumerate() {
                if param_names.contains(bname.as_str()) { continue; }
                let canon = body.alias_groups.get(bi)
                    .and_then(|g| g.first().copied())
                    .unwrap_or(bi as u16) as usize;
                if !seen_canonical.insert(canon) { continue; }
                // Only variables bound by an enclosing FUNCTION scope become cells, module names stay late-bound. A not-yet-assigned local captures an undef-seeded cell the parent's store fills later.
                if !locals.contains(ssa_strip(bname)) && !self.lexical_ancestor_binds(chunk_ptr, ssa_strip(bname)) { continue; }
                if let Some((si, _)) = chunk.names.iter().enumerate().find(|(_, n)| n.as_str() == bname.as_str()) {
                    let psi = chunk.alias_groups.get(si)
                        .and_then(|g| g.first().copied())
                        .unwrap_or(si as u16) as usize;
                    let v = slots.get(psi).copied().unwrap_or(Val::undef());
                    // Capture a shared cell, not the raw value, so sibling closures over the same variable see each other's nonlocal writes. Key the registry by the canonical parent slot `psi` (stable across siblings and SSA versions), not the callee's `canon` (which differs per closure body).
                    let cell = self.frame_cell_for(psi, v)?;
                    self.temp_roots.push(cell);
                    captures.push((canon, cell));
                }
            }
        }

        let val = self.heap.alloc(HeapObj::Func(global, defaults, captures, Rc::new(RefCell::new(Vec::new()))))?;
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

    /* True when a function scope strictly above `chunk` binds `bare`, pass-through frees capture, module names do not. */
    fn lexical_ancestor_binds(&mut self, chunk_ptr: *const SSAChunk, bare: &str) -> bool {
        let mut anc = self.body_to_fi.get(&chunk_ptr).and_then(|&fi| self.function_parents.get(fi).copied().flatten());
        while let Some(afi) = anc {
            let abody = &self.functions[afi].1;
            if self.chunk_locals(abody).contains(bare) { return true; }
            anc = self.function_parents.get(afi).copied().flatten();
        }
        false
    }

    /* Bare names a chunk binds itself, StoreName/Phi targets plus formal params. Cached per chunk pointer. */
    fn chunk_locals(&mut self, chunk: &SSAChunk) -> alloc::rc::Rc<crate::util::hash::FxHashSet<String>> {
        let key = chunk as *const SSAChunk;
        if let Some(s) = self.chunk_local_binds.get(&key) { return s.clone(); }
        let mut set: crate::util::hash::FxHashSet<String> = crate::util::hash::FxHashSet::default();
        for ins in &chunk.instructions {
            if matches!(ins.opcode, OpCode::StoreName | OpCode::Phi)
                && let Some(n) = chunk.names.get(ins.operand as usize)
            {
                set.insert(ssa_strip(n).to_string());
            }
        }
        if let Some(&fi) = self.body_to_fi.get(&key) {
            for p in &self.functions[fi].0 {
                set.insert(crate::parser::types::param_base_name(p).to_string());
            }
        }
        let rc = alloc::rc::Rc::new(set);
        self.chunk_local_binds.insert(key, rc.clone());
        rc
    }

    // Closure cell, a 1-element heap list used as a shared mutable box. Sibling closures over the same enclosing variable capture the same cell, so a `nonlocal` write through one is visible in the others.
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

    /* Calls a rebound builtin with the args a fused site stacked, the callee slotted under them. */
    fn call_rebound(&mut self, callee: Val, pos: usize, kw: usize, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let at = self.stack.len().checked_sub(pos + 2 * kw).ok_or_else(|| cold_runtime("stack underflow"))?;
        self.stack.insert(at, callee);
        self.exec_call_n(pos, kw, chunk, slots)
    }

    /* A fused builtin given `*` or keywords it cannot count runs as a plain call through its binding. */
    fn call_spread_builtin(&mut self, op: OpCode, pos: usize, kw: usize, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let name = fused_native(op).ok_or_else(|| cold_runtime("spread on an unknown fused call"))?.name();
        self.register_builtin(name);
        let callee = self.module_state.get(name).copied().filter(|v| !v.is_undef()).or_else(|| self.global(name)).ok_or_else(|| VmErr::Name(name.into()))?;
        self.call_rebound(callee, pos, kw, chunk, slots)
    }

    /* The counts a call gains from its spreads, reopening the frame its first spread saved. */
    fn close_spread(&mut self, pos: usize, kw: usize) -> (usize, usize) {
        let counts = ((pos as i32 + self.pending.pos_delta).max(0) as usize, (kw as i32 + self.pending.kw_delta).max(0) as usize);
        let (p, k) = self.pending.delta_save.pop().unwrap_or((0, 0));
        self.pending.pos_delta = p;
        self.pending.kw_delta = k;
        counts
    }

    /* The entry binds `bare` once and no class or module body binds it. */
    fn bound_once(&self, bare: &str) -> bool {
        let bound = |names: &crate::vm::NameVersionIndex| names.get(bare).map_or(0, |v| v.iter().filter(|(ver, _)| *ver >= 1).count());
        self.chunk_name_versions.get(&(self.chunk as *const SSAChunk)).is_some_and(|names| bound(names) == 1)
            && self.chunk_name_versions.iter().all(|(&chunk, names)| core::ptr::eq(chunk, self.chunk) || self.body_to_fi.contains_key(&chunk) || bound(names) == 0)
    }

    /* Each free name is an unbound builtin, or bound once to an immutable value or fixed function. */
    fn memo_reads_fixed(&self, fi: usize, visiting: &mut Vec<usize>) -> bool {
        if visiting.contains(&fi) { return true; }
        let own = self.self_ref_slot[fi];
        self.body_free_loads[fi].iter().filter(|(_, slot, _)| Some(*slot) != own).all(|(name, _, _)| match self.module_state.get(name.as_str()) {
            None => self.builtins.contains_key(name.as_str()),
            Some(&v) => self.bound_once(name) && (cache::deeply_immutable(v, &self.heap, 0) || matches!(self.heap.try_get(v),
                Some(HeapObj::Func(f, defaults, captures, attrs)) if captures.is_empty() && attrs.borrow().is_empty()
                    && defaults.iter().all(|&d| cache::deeply_immutable(d, &self.heap, 0))
                    && self.memo_ok[*f] && self.functions[*f].1.is_pure
                    && { visiting.push(fi); self.memo_reads_fixed(*f, visiting) })),
        })
    }

    /* Caches `result` on the second run of its key, or turns `fi` off when it never can. */
    fn memo_keep(&mut self, fi: usize, callee: Val, args: &[Val], defaults: &[Val], owner: Val, result: Val) {
        let immutable = |v: &Val| !v.is_heap() || cache::deeply_immutable(*v, &self.heap, 0);
        if !immutable(&result) || !args.iter().chain(defaults).all(immutable) {
            // A body that sees a mutable value before any entry likely always does.
            if !self.templates.holds(fi) { self.memo_ok[fi] = false; }
            return;
        }
        let Some(h) = self.templates.admit(fi, args, owner, &self.heap) else { return };
        // A function with attributes never memoizes, and reads found fixed stay so until the tables clear.
        let keep = matches!(self.heap.get(callee), HeapObj::Func(.., attrs) if attrs.borrow().is_empty())
            && (self.templates.holds(fi) || self.memo_reads_fixed(fi, &mut Vec::new()));
        if keep { self.templates.insert(fi, args, owner, result, h); } else { self.memo_ok[fi] = false; }
    }

    /* Calls `callee`, with `recv` first when bound, the arguments laid out as a plain call leaves them. */
    pub(crate) fn call_with(&mut self, callee: Val, recv: Option<Val>, positional: &[Val], kw_flat: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        self.push(callee);
        self.stack.extend(recv);
        self.stack.extend_from_slice(positional);
        self.stack.extend_from_slice(kw_flat);
        self.exec_call_n(positional.len() + recv.is_some() as usize, kw_flat.len() / 2, chunk, slots)
    }

    pub(crate) fn exec_call(&mut self, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        self.exec_call_n((operand & 0xFF) as usize, ((operand >> 8) & 0xFF) as usize, chunk, slots)
    }

    /* `Call` orchestrator. Only user `Func` callees build a fresh `fn_slots` and run the body inline, every other callee kind short-circuits in `try_dispatch_non_func_callable`. */
    pub(crate) fn exec_call_n(&mut self, num_pos: usize, num_kw: usize, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Taken so nested native calls see false.
        let call_safe = core::mem::take(&mut self.pending_exec_safe);
        let (positional, kw_flat) = self.take_args(num_pos, num_kw)?;

        if self.depth >= self.max_calls { return Err(cold_depth()); }
        // Charge each call so wide recursion is op-budget bounded.
        self.charge_step()?;

        let callee = self.pop()?;
        if !callee.is_heap() { return Err(cold_type("object is not callable")); }

        // Snapshot defaults/captures once, both are tiny (<10), and cloning beats the 3+ heap re-reads later phases would do. Back-prop still uses `get_mut` since it writes. Probing Func first skips the 9-shape non-func walk on the hottest (user function) path.
        let (fi, defaults, captures) = match self.heap.get(callee) {
            HeapObj::Func(i, d, c, _) => (*i, d.clone(), c.clone()),
            _ => {
                // Bound methods re-dispatch as tail calls.
                if matches!(self.heap.get(callee), HeapObj::BoundUserMethod(..)) {
                    self.pending_exec_safe = call_safe;
                }
                let dispatched = self.try_dispatch_non_func_callable(callee, &positional, &kw_flat, chunk, slots);
                self.pending_exec_safe = false;
                if dispatched? {
                    return Ok(());
                }
                return Err(cold_type("object is not callable"));
            }
        };

        // Kwargs and closures reach past the key, and a function with defaults keys on itself.
        let memo_ok = num_kw == 0 && captures.is_empty() && self.memo_ok.get(fi).copied().unwrap_or(false);
        let owner = if defaults.is_empty() { Val::none() } else { callee };
        if memo_ok && let Some(cached) = self.templates.lookup(fi, &positional, owner, &self.heap) {
            self.push(cached);
            return Ok(());
        }

        self.depth += 1;
        let (_params, body, _, _) = self.functions[fi];
        // Reuse a pooled buffer, clear + bulk copy beats a fresh alloc per call.
        let mut fn_slots = self.slot_pool.pop().unwrap_or_default();
        fn_slots.clear();
        fn_slots.extend_from_slice(&self.slot_templates[fi]);

        self.bind_function_args(fi, &defaults, &captures, &positional, &kw_flat, &mut fn_slots)?;

        if self.needs_caller_slots[fi] {
            self.apply_caller_slot_propagation(fi, &captures, chunk, slots, &mut fn_slots);
        }

        self.bind_self_reference(fi, callee, &mut fn_slots);

        // Generator/coroutine, return a suspended Coroutine instead of running. Both flags are O(1).
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
        if callee_impure {
            self.mark_impure();
            if self.globals_written { self.reload_globals(chunk, slots); }
        }

        if self.yielded {
            // Sync helper suspended mid-execution (e.g. `sleep(0)` from inside a sync fn called by an async coro). Stage its frame on the VM-level buffer. `resume_coroutine` drains it onto the enclosing coro so the helper is re-entered from the right ip. Without this, the outer's resume_ip would skip past the unfinished helper and the next StoreName would underflow. A nested sync call inside this helper would already have pushed its own frame first, so the buffer ends up innermost-last.
            let helper_resume_ip = self.resume_ip;
            self.resume_ip = 0;
            let (stack_delta, iter_delta, exception_delta) = self.split_frames(stack_base, iter_base, exc_base);
            self.pending_sync_frames.push(SyncFrame { ip: helper_resume_ip, fi, func: callee, slots: fn_slots, stack_delta, iter_delta, exception_delta });
            return Ok(());
        }

        if self.yields.len() > yields_before {
            let fn_yields = self.yields.split_off(yields_before);
            let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(fn_yields))))?;
            self.push(val);
        } else {
            if memo_ok && body.is_pure && !callee_impure { self.memo_keep(fi, callee, &positional, &defaults, owner, result); }
            self.push(result);
        }
        // Recycle the frame buffer, error/suspend paths above just drop theirs.
        if self.slot_pool.len() < 64 { self.slot_pool.push(fn_slots); }
        Ok(())
    }

    /* Pops the positional args and the flat name and value keyword pairs a call counts. */
    pub(crate) fn parse_call_args(&mut self, operand: u16) -> Result<(Vec<Val>, Vec<Val>), VmErr> {
        self.take_args((operand & 0xFF) as usize, ((operand >> 8) & 0xFF) as usize)
    }

    /* Pops `num_pos` positionals and `num_kw` name/value pairs. */
    pub(crate) fn take_args(&mut self, num_pos: usize, num_kw: usize) -> Result<(Vec<Val>, Vec<Val>), VmErr> {
        let total_items = num_pos + 2 * num_kw;
        // Bulk-copy the stack tail, one memcpy instead of per-item pops plus a reverse.
        let at = self.stack.len().checked_sub(total_items).ok_or_else(|| cold_runtime("stack underflow"))?;
        let kw_flat: Vec<Val> = self.stack[at + num_pos..].to_vec();
        let mut positional = self.stack.split_off(at);
        positional.truncate(num_pos);
        Ok((positional, kw_flat))
    }

    /* Pack a flat `[name, val, name, val, ...]` slice into a heap dict for the trailing kwargs slot. `None` when there are no kwargs so the FFI layer can serialize handle 0 on the wire. */
    pub(crate) fn pack_kw_dict(heap: &mut super::super::types::HeapPool, kw_flat: &[Val]) -> Result<Option<Val>, VmErr> {
        if kw_flat.is_empty() { return Ok(None); }
        let dm = super::super::types::DictMap::from_pairs(kw_flat.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect(), heap);
        Ok(Some(heap.alloc(super::super::types::HeapObj::Dict(Rc::new(RefCell::new(dm))))?))
    }

    /* list.sort() parses key/reverse kwargs and sorts in place. Intercepted from both call paths since it needs chunk/slots for __lt__. */
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

    /* Dispatch non-Func callees. Returns Ok(true) when handled here, Ok(false) means the caller falls through to the Func path. */
    fn try_dispatch_non_func_callable(&mut self, callee: Val, positional: &[Val], kw_flat: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        match self.heap.try_get(callee) {
            // `list[int](xs)` builds what its origin builds.
            Some(&HeapObj::GenericAlias(origin, _)) => return self.try_dispatch_non_func_callable(origin, positional, kw_flat, chunk, slots),
            Some(&HeapObj::BoundMethod(recv, id)) if recv.is_undef() => self.exec_unbound_method(id, positional, kw_flat, chunk, slots)?,
            Some(&HeapObj::BoundMethod(recv, id)) => self.exec_bound_method(recv, id, positional, kw_flat, chunk, slots)?,
            Some(&HeapObj::NativeFn(id)) => {
                // First-class builtins (e.g. `apply(print, x)`) bypass the CallPrint/CallInput opcodes. Mark here so a pure wrapper around them isn't memoised.
                if native_is_impure(id) { self.mark_impure(); }
                self.dispatch_native(id, positional, kw_flat, chunk, slots)?;
            }
            // Park on host-deferral like `call_extern` (e.g. `time.sleep` via module attr).
            Some(HeapObj::Extern(extern_fn)) => {
                let (func, pure) = (extern_fn.func.clone(), extern_fn.pure);
                self.invoke_extern(&func, pure, positional, kw_flat)?;
            }
            Some(HeapObj::Type(name)) => {
                let name = name.clone();
                if let Some(id) = constructor_native(&name) {
                    self.dispatch_native(id, positional, kw_flat, chunk, slots)?; // int/set/list/... construct
                } else if name == "object" {
                    // `object()` builds a unique featureless instance.
                    if !positional.is_empty() || !kw_flat.is_empty() { return Err(cold_type("object() takes no arguments")); }
                    let inst = self.heap.alloc(HeapObj::Instance(callee, Rc::new(RefCell::new(DictMap::new()))))?;
                    self.push(inst);
                } else {
                    // Other Type objects are exception classes, build an ExcInstance for `raise X("msg")`.
                    if !kw_flat.is_empty() { return Err(cold_type("exception class takes no keyword arguments")); }
                    let exc = self.heap.alloc(HeapObj::ExcInstance(name, positional.to_vec()))?;
                    self.push(exc);
                }
            }
            // Calling a class, create an instance and run `__init__` if defined (walks bases).
            Some(HeapObj::Class(..)) => {
                let init = self.lookup_class_member(callee, "__init__");
                if init.is_none() && !kw_flat.is_empty() {
                    return Err(cold_type("class constructor takes no keyword arguments"));
                }
                let instance = self.heap.alloc(HeapObj::Instance(callee, Rc::new(RefCell::new(DictMap::new()))))?;
                // An exception keeps its constructor arguments as `args`, a later `__init__` may reset them.
                if self.exc_base(callee).is_some() { self.set_exc_args(instance, positional.to_vec())?; }
                if let Some((init_fn, defining)) = init {
                    // Fail-fast before pushing, the inner check fires only after parse_call_args pops.
                    if self.depth >= self.max_calls { return Err(cold_depth()); }
                    self.pending.method_binding = Some((defining, instance));
                    // Keywords reach `__init__` as they reach any function, its return value is dropped.
                    self.call_with(init_fn, Some(instance), positional, kw_flat, chunk, slots)?;
                    self.pop()?;
                }
                self.push(instance);
            }
            // Bound user method, prepend `self` to the arg list and re-dispatch.
            Some(&HeapObj::BoundUserMethod(recv, func, class)) => {
                // Same as the Class branch, depth check before mutating the stack.
                if self.depth >= self.max_calls { return Err(cold_depth()); }
                self.pending.method_binding = Some((class, recv));
                self.call_with(func, Some(recv), positional, kw_flat, chunk, slots)?;
            }
            // `prop.setter(fn)` returns a new `Property` carrying the original getter plus the supplied setter.
            Some(&HeapObj::PropertySetter(prop_val)) => {
                if positional.len() != 1 || !kw_flat.is_empty() {
                    return Err(cold_type("property.setter takes exactly 1 argument"));
                }
                let Some(&HeapObj::Property(getter, _)) = self.heap.try_get(prop_val) else {
                    return Err(cold_runtime("PropertySetter wraps a non-Property value"));
                };
                let new_prop = self.heap.alloc(HeapObj::Property(getter, positional[0]))?;
                self.push(new_prop);
            }
            // Instance with `__call__`, bind and dispatch through `BoundUserMethod`-style flow.
            Some(&HeapObj::Instance(cls, _)) => {
                let Some((func, class)) = self.lookup_class_member(cls, "__call__") else { return Ok(false) };
                if self.depth >= self.max_calls { return Err(cold_depth()); }
                self.pending.method_binding = Some((class, callee));
                self.call_with(func, Some(callee), positional, kw_flat, chunk, slots)?;
            }
            Some(HeapObj::Coroutine(_, _, _, body, ..)) => {
                // Plain `async def` (no `yield`) drives to completion via the scheduler (await semantics). Async *generators* fall through to step-wise resume like sync generators.
                let drive_async = matches!(*body, super::super::types::BodyRef::Fn(fi)
                    if self.is_async.get(fi).copied().unwrap_or(false) && !self.functions[fi].1.is_generator);
                if drive_async {
                    self.await_coroutine(callee)?;
                } else {
                    // Generator stepping (ForIter calls here per `next`) resumes one step, the inner yield must NOT propagate to the caller.
                    let result = self.resume_coroutine(callee)?;
                    self.yielded = false;
                    self.push(result);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
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
                    let pairs: Vec<(Val, Val)> = kw_flat.as_chunks::<2>().0.iter()
                        .filter(|p| match self.heap.try_get(p[0]) {
                            Some(HeapObj::Str(s)) => !params.iter().any(|pp| !pp.starts_with('*') && crate::parser::types::param_base_name(pp) == s.as_str()),
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
                // KwOnly slots are NOT consumed positionally, they bind only via kwargs.
                ParamKind::KwOnly => {}
            }
        }

        // Kwargs binding (rare path, not optimised).
        if !kw_flat.is_empty() {
            let params = &self.functions[fi].0;
            let body_map = &self.body_maps[fi];
            let has_double_star = self.param_slots[fi].iter().any(|(k, _)| matches!(k, ParamKind::DoubleStar));
            for pair in kw_flat.as_chunks::<2>().0 {
                // Malformed `**`/kwarg bytecode can leave a non-string in the name slot, so guard the heap access.
                let key = match self.heap.try_get(pair[0]) {
                    Some(HeapObj::Str(s)) => s.clone(),
                    _ => return Err(cold_runtime("malformed kwarg on stack")),
                };
                // Star/double-star params are not keyword targets. A kwarg whose name matches `*a`/`**k` goes to **kwargs.
                if params.iter().any(|p| !p.starts_with('*') && crate::parser::types::param_base_name(p) == key.as_str()) {
                    let pname = s!(str &key, "_0");
                    if let Some(&s) = body_map.get(pname.as_str()) {
                        if !fn_slots[s].is_undef() { return Err(VmErr::TypeMsg(s!("got multiple values for argument '", str &key, "'"))); }
                        fn_slots[s] = pair[1];
                    }
                } else if !has_double_star {
                    return Err(VmErr::TypeMsg(s!("got an unexpected keyword argument '", str &key, "'")));
                }
            }
        }

        // Defaults only fill slots still undef after binding.
        if !defaults.is_empty() {
            let ds = &self.default_slots[fi];
            for (di, &dv) in defaults.iter().enumerate() {
                if let Some(&(slot, _)) = ds.get(di)
                    && slot < fn_slots.len() && fn_slots[slot].is_undef() {
                        fn_slots[slot] = dv;
                    }
            }
        }

        // A parameter left without a positional, a keyword or a default is a missing argument.
        if positional.len() < n_params {
            for (i, &(kind, slot)) in self.param_slots[fi].iter().enumerate() {
                if matches!(kind, ParamKind::Normal | ParamKind::KwOnly) && fn_slots.get(slot).is_some_and(|v| v.is_undef()) {
                    let name = crate::parser::types::param_base_name(&self.functions[fi].0[i]);
                    return Err(VmErr::TypeMsg(s!("missing required argument '", str name, "'")));
                }
            }
        }

        // Closure captures follow the same rule as defaults, only fill if undef. Each capture is a shared cell, read its current value into the slot.
        for &(bi, cell) in captures {
            if bi < fn_slots.len() && fn_slots[bi].is_undef() {
                fn_slots[bi] = self.cell_get(cell);
            }
        }

        Ok(())
    }

    /* Push caller slots into body slots. Same scope means late-binding, overwrite freely. Different scope skips capture-filled slots (fixes stacked-decorator clobber). */
    fn apply_caller_slot_propagation(&mut self, fi: usize, captures: &[(usize, Val)], chunk: &SSAChunk, slots: &[Val], fn_slots: &mut [Val]) {
        let info = self.propagation_map(fi, chunk);
        let captured_set: crate::util::hash::FxHashSet<usize> = if info.same_scope {
            crate::util::hash::FxHashSet::default()
        } else {
            captures.iter().map(|(s, _)| *s).collect()
        };
        // Undef/captured filters stay per-call, the name matching is cached.
        for &(si, bs) in info.pairs.iter() {
            let bs = bs as usize;
            if let Some(&v) = slots.get(si as usize)
                && !v.is_undef()
                && !captured_set.contains(&bs)
            {
                fn_slots[bs] = v;
            }
        }

        // Free names resolve lexically, exact-version hit in the caller frame (live parent local), then the module layers (late-bound), then the latest-version net where a lexical source is plausible. Entry-chunk slots are excluded from the net since `global`/`del` writes only reach `module_state`, so those slots go stale.
        let caller_is_module = !self.body_to_fi.contains_key(&(chunk as *const SSAChunk));
        let caller_is_entry = core::ptr::eq(chunk, self.chunk);
        let parent_is_fn = self.function_parents.get(fi).is_some_and(|p| p.is_some());
        for (bare, bs, ref_ver, versions) in info.free.iter() {
            let bs = *bs as usize;
            if captured_set.contains(&bs) { continue; }
            if !caller_is_module
                && let Some(&(_, si)) = versions.iter().find(|&&(v, _)| v == *ref_ver)
                && let Some(&v) = slots.get(si as usize)
                && !v.is_undef()
            {
                fn_slots[bs] = v;
                continue;
            }
            if let Some(v) = self.resolve_free_name_fallback(fi, bare) {
                fn_slots[bs] = v;
                continue;
            }
            if caller_is_entry || (!info.same_scope && !parent_is_fn) { continue; }
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
            }
        }
    }

    /* After a `global` store a function caller re-reads its globals, back at the entry it resets. */
    fn reload_globals(&mut self, chunk: &SSAChunk, slots: &mut [Val]) {
        if core::ptr::eq(chunk, self.chunk) { self.globals_written = false; return; }
        let ptr = chunk as *const SSAChunk;
        let Some(&cfi) = self.body_to_fi.get(&ptr) else { return };
        if self.fn_module[cfi].is_some() { return; }
        // Taken out for the loop, so no name is cloned to satisfy the borrow.
        let loads = core::mem::take(&mut self.body_free_loads[cfi]);
        for (bare, slot, _) in &loads {
            let Some(&v) = self.module_state.get(bare.as_str()) else { continue };
            // A name an enclosing function binds is its captured local, not the global.
            if !self.lexical_ancestor_binds(ptr, bare) && let Some(s) = slots.get_mut(*slot) { *s = v; }
        }
        self.body_free_loads[cfi] = loads;
    }

    /* Build or fetch the static propagation info for (chunk, fi). Chunks are borrowed for the VM's lifetime, so the pointer key is stable. */
    fn propagation_map(&mut self, fi: usize, chunk: &SSAChunk) -> super::super::PropagationMap {
        let key = (chunk as *const SSAChunk, fi);
        if let Some(m) = self.propagation_maps.get(&key) { return m.clone(); }
        // Same-scope also requires same module, keeps top-level imports (`parent_fi == None`) isolated.
        let caller_fi = self.body_to_fi.get(&(chunk as *const _)).copied();
        let callee_parent_fi = self.function_parents.get(fi).and_then(|x| *x);
        let caller_module = caller_fi.and_then(|cf| self.fn_module.get(cf).and_then(|m| m.as_deref()));
        let callee_module = self.fn_module.get(fi).and_then(|m| m.as_deref());
        let same_scope = caller_fi == callee_parent_fi && caller_module == callee_module;
        // Another top-level function lends only the globals it reads itself, never one of its locals.
        let lent: &[(String, usize, i64)] = match caller_fi {
            Some(cf) if !same_scope && caller_module == callee_module && self.function_parents[cf].is_none() => &self.body_free_loads[cf],
            _ => &[],
        };
        let canon = |si: usize| chunk.alias_groups.get(si).and_then(|g| g.first().copied()).unwrap_or(si as u16) as u32;
        let lends = |si: u32| same_scope || lent.iter().any(|&(_, s, _)| s == si as usize);
        let body_map = &self.body_maps[fi];
        let param_bm = &self.is_param_slot[fi];
        let mut pairs: Vec<(u32, u32)> = body_map.iter()
            .filter_map(|(name, &bs)| {
                let si = chunk.slot_of(name)? as usize;
                if param_bm.get(bs).copied().unwrap_or(false) || !lends(canon(si)) { return None; }
                Some((si as u32, bs as u32))
            })
            .collect();
        // Caller slot order, so a later version still wins a shared body slot.
        pairs.sort_unstable();
        // Free loads with their caller-chunk (version, slot) candidates resolved once. Candidate slots canonicalise because operand rewriting stores values at the version chain's root.
        let name_index = self.chunk_name_versions.get(&(chunk as *const _));
        let free: Vec<super::super::FreeLoadEntry> = self.body_free_loads[fi].iter()
            .map(|(bare, bs, ref_ver)| {
                let versions = name_index
                    .and_then(|idx| idx.get(bare.as_str()))
                    .map(|v| v.iter().map(|&(ver, si)| (ver, canon(si))).filter(|&(_, si)| lends(si)).collect())
                    .unwrap_or_default();
                (bare.clone(), *bs as u32, *ref_ver, versions)
            })
            .collect();
        let map: super::super::PropagationMap = alloc::rc::Rc::new(super::super::PropInfo { same_scope, pairs, free });
        self.propagation_maps.insert(key, map.clone());
        map
    }

    /* Slow layers for a bare free-load name after the caller-slot layer missed, callee module attrs -> entry module state -> globals. First hit wins. Centralised so the order is auditable. */
    pub(crate) fn resolve_free_name_fallback(&self, fi: usize, bare: &str) -> Option<Val> {
        // Layer 2 checks the callee's module attrs, keeping `a.helper` and `b.helper` isolated.
        if let Some(Some(spec)) = self.fn_module.get(fi).cloned()
            && let Some(mod_val) = self.module_table.get(&spec).copied()
            && mod_val.is_heap()
            && let HeapObj::Module(_, attrs) = self.heap.get(mod_val)
            && let Some((_, v)) = attrs.iter().find(|(n, _)| n == bare)
        {
            return Some(*v);
        }
        // Layer 3 checks entry-module bindings, live-mirrored on every store. Beats `globals` so rebinding a def'd name is seen.
        if self.fn_module.get(fi).is_none_or(|m| m.is_none())
            && let Some(&v) = self.module_state.get(bare)
            && !v.is_undef()
        {
            return Some(v);
        }
        // Layer 4 checks globals, catching forward-ref mutual recursion in the entry chunk.
        self.global(bare)
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

    /* Run the body with caller slots pinned in `live_slots` (GC roots) and a CallFrame on `call_stack` (traceback). Frame popped on success only, the dispatch catch clears it on swallowed exceptions. Returns `(callee_impure, exec_result)`. */
    fn run_body_with_frame(&mut self, fi: usize, body: &SSAChunk, chunk: &SSAChunk, fn_slots: &mut [Val], slots: &[Val]) -> (bool, Result<Val, VmErr>) {
        // GC roots come from `active_slots` (every live exec frame), `live_slots` only feeds `globals()`, which reads the entry chunk's slots at the bottom. Copy just that frame.
        let snap = self.live_slots.len();
        if snap == 0 && core::ptr::eq(chunk, self.chunk) {
            self.live_slots.extend_from_slice(slots);
        }

        // Frame snapshots caller's source/path so render doesn't borrow live chunk pointers.
        let call_byte_pos = self.pending.call_byte_pos.take().unwrap_or(0);
        // Method-call paths set `method_binding` immediately before invoking `exec_call`, plain function calls leave it `None`.
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
    pub(crate) fn back_propagate_nonlocals(&mut self, fi: usize, body: &SSAChunk, callee: Val, chunk: &SSAChunk, slots: &mut [Val], fn_slots: &[Val]) {
        if self.nonlocal_tables[fi].is_empty() { return; }
        // Snapshot to release borrows on self before the `heap.get_mut` writes.
        let nl_pairs: Vec<(usize, usize)> = self.nonlocal_tables[fi].clone();
        let name_index = self.chunk_name_versions.get(&(chunk as *const _));
        for (canon_body, ni) in nl_pairs {
            let Some(&val) = fn_slots.get(canon_body) else { continue };
            if val.is_undef() { continue; }
            // Each nonlocal writes back into its own name only.
            if let Some(idx) = name_index && let Some(versions) = body.nonlocals.get(ni).and_then(|base| idx.get(base.as_str())) {
                for &(_, si) in versions {
                    if si < slots.len() { slots[si] = val; }
                }
            }
            // Write into the shared cell so sibling closures over this variable observe the nonlocal write. Access `self.heap` directly (not via &mut self helpers) so it stays disjoint from the `name_index` borrow above.
            let cell = if let HeapObj::Func(_, _, caps, _) = self.heap.get(callee) {
                caps.iter().find(|(ci, _)| *ci == canon_body).map(|(_, c)| *c)
            } else { None };
            match cell {
                Some(c) => if let HeapObj::List(rc) = self.heap.get(c) {
                    self.heap.growing(&mut *rc.borrow_mut(), |b| if b.is_empty() { b.push(val); } else { b[0] = val; });
                },
                // Nonlocal target not captured at MakeFunction (rare), attach a fresh cell so the next call sees it.
                None => if let Ok(c) = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(vec![val]))))
                    && let HeapObj::Func(_, _, caps, _) = self.heap.get_mut(callee) {
                        let before = caps.bytes();
                        caps.push((canon_body, c));
                        let grown = caps.bytes() - before;
                        self.heap.charge(grown);
                    },
            }
        }
    }


    /* CallExtern's operand packs `(extern_idx<<8)|(kw<<4)|pos`. Pop kw `name,val` pairs then `pos` positional vals, pack pairs into a heap dict via `pack_kw_dict` and hand it off as the explicit `Option<Val>` kwargs slot. Pure externs leave the impurity flag alone, bodies whose only side-effects are pure externs stay memoizable. */
    pub(crate) fn call_extern(&mut self, operand: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let extern_idx = (operand >> 8) as usize;
        // A star spread grows the counts the operand was written with.
        let (pos, kw) = self.close_spread((operand & 0xF) as usize, ((operand >> 4) & 0xF) as usize);
        let extern_fn = chunk.extern_table.get(extern_idx).ok_or_else(|| cold_runtime("CallExtern: extern index out of bounds"))?;
        let func = extern_fn.func.clone(); // Arc clone, refcount bump only
        let pure = extern_fn.pure;
        let kw_flat = if kw > 0 { self.pop_n(kw * 2)? } else { Vec::new() };
        let positional = self.pop_n(pos)?;
        self.invoke_extern(&func, pure, &positional, &kw_flat)
    }

    /* Runs a native binding with its args rooted, a plugin can call back into code that collects. */
    fn invoke_extern(&mut self, func: &crate::vm::types::ExternCallable, pure: bool, positional: &[Val], kw_flat: &[Val]) -> Result<(), VmErr> {
        if !pure { self.mark_impure(); }
        let kwargs = Self::pack_kw_dict(&mut self.heap, kw_flat)?;
        let roots = positional.iter().chain(kw_flat).copied().chain(kwargs);
        match self.with_roots(roots, |vm| func(&mut vm.heap, positional, kwargs)) {
            Ok(result) => { self.push(result); Ok(()) }
            Err(VmErr::HostCallDeferred) => { self.park_host_call(); Ok(()) }
            Err(e) => Err(e),
        }
    }

    /* Park a native that deferred to the host with a `None` placeholder (overwritten by `set_host_result_by_id`), correlation id, yield. */
    fn park_host_call(&mut self) {
        self.push(Val::none());
        self.pending.host_call_id = self.next_host_call_id;
        self.next_host_call_id += 1;
        self.pending.host_call_request = true;
        self.yielded = true;
    }

    /* A builtin that takes keywords sees each one as the positional it names, `int("ff", base=16)`. */
    #[cold]
    fn dispatch_native_named(&mut self, id: super::super::types::NativeFnId, positional: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use super::super::types::NativeFnId::*;
        let names: &[&str] = match id {
            Int => &["x", "base"],
            Round => &["number", "ndigits"],
            Sum => &["iterable", "start"],
            Pow => &["base", "exp", "mod"],
            Str => &["object", "encoding", "errors"],
            Bytes => &["source", "encoding", "errors"],
            _ => return Err(VmErr::TypeMsg(s!(str id.name(), "() takes no keyword arguments"))),
        };
        let mut args: Vec<Option<Val>> = positional.iter().map(|&v| Some(v)).collect();
        for pair in kw.chunks(2) {
            let name = self.kw_name(pair[0]).unwrap_or("");
            let Some(i) = names.iter().position(|&n| n == name) else {
                return Err(VmErr::TypeMsg(s!("'", str name, "' is an invalid keyword argument for ", str id.name(), "()")));
            };
            if args.get(i).is_some_and(|a| a.is_some()) { return Err(VmErr::TypeMsg(s!(str id.name(), "() got multiple values for argument '", str name, "'"))); }
            if args.len() <= i { args.resize(i + 1, None); }
            args[i] = Some(pair[1]);
        }
        let args: Option<Vec<Val>> = args.into_iter().collect();
        let args = args.ok_or_else(|| VmErr::TypeMsg(s!(str id.name(), "() is missing a positional argument")))?;
        self.dispatch_native(id, &args, &[], chunk, slots)
    }

    pub(crate) fn dispatch_native(&mut self, id: super::super::types::NativeFnId, positional: &[Val], kw: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use super::super::types::NativeFnId::*;

        // `sorted()` extracts `key=`/`reverse=` before the no-kw guard, print/min/max/enumerate/dict parse their own kwargs below like the fused opcodes do.
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

        let kw_aware = matches!(id, Print | Min | Max | Enumerate | Dict);
        if !kw_remaining.is_empty() && !kw_aware {
            return self.dispatch_native_named(id, positional, kw_remaining, chunk, slots);
        }
        let argc = positional.len() as u16;

        // Pre-validate arity to keep the stack clean on error.
        if !id.takes(argc) { return Err(arity_err(id, argc)); }

        for &v in positional { self.push(v); }
        // Repack pos/kw counts so the handlers pop the same layout a fused call leaves.
        let operand = if kw_aware && !kw_remaining.is_empty() {
            for &v in kw_remaining { self.push(v); }
            (((kw_remaining.len() / 2) as u16) << 8) | argc
        } else { argc };
        if iterates(id) { self.lift_builtin_args(id, argc as usize, kw_remaining.len() / 2, chunk, slots)?; }
        match id {
            Sorted => self.call_sorted_with_key(sort_key, sort_reverse, chunk, slots),
            // CallPrint is statement-shaped, reached through Call its result is popped, so it leaves None.
            Print => { self.run_native(id, operand, chunk, slots)?; self.push(Val::none()); Ok(()) }
            _ => self.run_native(id, operand, chunk, slots),
        }
    }

    /* Runs a builtin on stacked args, `operand` a count or packed counts for keyword-aware ones. */
    fn run_native(&mut self, id: super::super::types::NativeFnId, operand: u16, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        use super::super::types::NativeFnId::*;
        match id {
            // Variadic
            Print => { self.mark_impure(); self.call_print(operand, chunk, slots) }
            Range => self.call_range(operand),
            Round => self.call_round(operand),
            Min => self.call_min(operand, chunk, slots),
            Max => self.call_max(operand, chunk, slots),
            Sum => self.call_sum(operand),
            Zip => self.call_zip(operand),
            Dict => self.call_dict(operand, chunk, slots),
            Set => self.call_set(operand, chunk, slots),
            Pow => self.call_pow(operand),
            All => self.call_all(operand),
            Any => self.call_any(operand),
            GetAttr => self.call_getattr(operand, chunk, slots),
            Format => self.call_format(operand, chunk, slots),
            // 0/1/2-arg
            Input => { self.mark_impure(); self.call_input() }
            Len => self.call_len(chunk, slots),
            Abs => self.call_abs(chunk, slots),
            Str => self.call_str(operand, chunk, slots),
            Int => self.call_int(operand, chunk, slots),
            Float => self.call_float(operand, chunk, slots),
            Bool => self.call_bool(operand, chunk, slots),
            Type => self.call_type(),
            Chr => self.call_chr(),
            Ord => self.call_ord(),
            Sorted => self.call_sorted(false, chunk, slots),
            Enumerate => self.call_enumerate(operand),
            List => self.call_list(operand, chunk, slots),
            Tuple => self.call_tuple(operand, chunk, slots),
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
            HasAttr => self.call_hasattr(chunk, slots),
            Next => self.call_next(operand, chunk, slots),
            Run => self.call_run(operand),
            Sleep => self.call_sleep(),
            Receive => self.call_receive(),
            SendMsg => self.call_send(),
            Map => self.call_map(operand, chunk, slots),
            Filter => self.call_filter(chunk, slots),
            Iter => self.call_iter(operand, chunk, slots),
            Bytes => self.call_bytes(operand),
            Slice => self.call_slice(operand),
            Vars => self.call_vars(),
            SetAttr => self.call_setattr(chunk, slots),
            DelAttr => self.call_delattr(),
            ImportModule => self.call_import_module(),
            Gather => self.call_gather(operand),
            WithTimeout => self.call_with_timeout(),
            Cancel => self.call_cancel(),
            BytesFromHex => self.call_bytes_fromhex(),
            IntFromBytes => self.call_int_from_bytes(),
            IntToBytes => self.call_int_to_bytes(),
            FrozenSet => self.call_frozenset(operand, chunk, slots),
            Globals => self.call_globals(chunk, slots),
            Locals => self.call_locals(chunk, slots),
            Super => self.call_super(),
            Property => self.call_property(operand),
            StaticMethod => self.call_staticmethod(operand),
            ClassMethod => self.call_classmethod(operand),
        }
    }
}
