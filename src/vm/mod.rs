/* The value model moved to `crate::value`, kept here so existing embedder paths keep resolving. */
#[doc(hidden)]
pub use crate::value as types;
/* The optimizer moved to `crate::optimizer`, kept here so existing embedder paths keep resolving. */
#[doc(hidden)]
pub use crate::optimizer;

mod cache;
mod value_ops;
mod format_spec;
pub(crate) mod globals;
pub(crate) mod opcodes;
pub(crate) mod methods;
pub mod snapshot;

mod dispatch;
mod gc;
mod helpers;
mod init;

use crate::s;
use crate::parser::{SSAChunk, BUILTIN_TYPES};
use crate::util::hash::FxHashMap as HashMap;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, Templates};
use alloc::{string::{String, ToString}, vec::Vec};

pub(crate) use types::ExceptionFrame;

#[derive(Clone, Copy)]
pub(crate) enum ParamKind { Normal, Star, DoubleStar, KwOnly }

/* Side-channel state passed between opcodes in one dispatch frame, grouped for auditability. */
pub(crate) struct Pending {
    /* Star/double-star spread bumps the next Call's argument count. */
    pub pos_delta: i32,
    pub kw_delta: i32,
    // Saved enclosing spread deltas (BeginArgs).
    pub delta_save: alloc::vec::Vec<(i32, i32)>,
    /* Current Call's byte offset, consumed by the traceback renderer. */
    pub call_byte_pos: Option<u32>,
    /* Wakeup deadline set by `sleep()` and consumed by the scheduler. */
    pub sleep_until_ns: Option<u64>,
    /* Set by `frame()`, consumed by `scheduler_step` to transition the coro to `WaitingFrame`. */
    pub host_frame_request: bool,
    /* Set by `receive()` on empty queue, transitions the coro to `WaitingEvent`. */
    pub event_wait_request: bool,
    /* Set by `call_extern` on deferred native, transitions the coro to `WaitingHostCall`. */
    pub host_call_request: bool,
    /* Correlation id of the deferred call, read by `scheduler_step` into `WaitingHostCall(id)`. */
    pub host_call_id: u64,
    /* Set by `call_run` / `call_gather` / `call_with_timeout` when they yield, transitions the outer to `WaitingForChildren`. */
    pub waiting_for_children: Option<(Vec<Val>, types::WaitKind)>,
    /* Lifted ExcInstance from `raise X(...)` so `except X as e` binds the real instance. */
    pub exc_val: Option<Val>,
    /* `(class, self)` for the next user-function call when it's invoked as a method, populated by method-dispatch paths and consumed by `run_body_with_frame`. */
    pub method_binding: Option<(Val, Val)>,
    /* Set at preempt, `top_loop` yields `Preempted`. */
    pub preempt_request: bool,
}

impl Pending {
    const fn new() -> Self {
        Self {
            pos_delta: 0,
            kw_delta: 0,
            delta_save: alloc::vec::Vec::new(),
            call_byte_pos: None,
            sleep_until_ns: None,
            host_frame_request: false,
            event_wait_request: false,
            host_call_request: false,
            host_call_id: 0,
            waiting_for_children: None,
            exc_val: None,
            method_binding: None,
            preempt_request: false,
        }
    }
}

/* `bare_name -> [(version, slot), ...]` for one chunk's `chunk.names`. */
pub(crate) type NameVersionIndex = crate::util::hash::FxHashMap<String, Vec<(i64, usize)>>;

/* Free-load propagation entry (bare name, body slot, referenced version, caller [(version, slot)] candidates). */
pub(crate) type FreeLoadEntry = (String, u32, i64, Vec<(i64, u32)>);

/* Static caller->callee propagation data for one (caller chunk, callee fi) pair, built once, then per call is pure slot reads (no string hashing). */
pub(crate) struct PropInfo {
    /* Caller/callee share the lexical scope and module, so late binding overwrites freely. */
    pub same_scope: bool,
    /* (caller slot, body slot) exact-name matches. */
    pub pairs: Vec<(u32, u32)>,
    pub free: Vec<FreeLoadEntry>,
}
pub(crate) type PropagationMap = alloc::rc::Rc<PropInfo>;

pub struct VM<'a> {
    pub(crate) stack: Vec<Val>,
    pub(crate) heap: HeapPool,
    pub(crate) iter_stack: Vec<IterFrame>,
    pub(crate) yields: Vec<Val>,
    pub(crate) chunk: &'a SSAChunk,
    pub(crate) globals: HashMap<String, Val>,
    /* User-mutated module-level state, keyed by bare name, mirrors entry-chunk stores and backs `global` declarations. */
    pub(crate) module_state: HashMap<String, Val>,
    pub(crate) live_slots: Vec<Val>,
    pub(crate) templates: Templates,
    pub(crate) budget: usize,
    pub(crate) depth: usize,
    pub(crate) max_calls: usize,
    pub(crate) observed_impure: Vec<bool>,
    // C3 method-resolution order per class, keyed by the class Val's heap bits. Computed once at MakeClass (the class graph is a static DAG). A reused slot is overwritten by its new class, so stale entries are never read (lookup checks HeapObj::Class first). Not a GC root because MRO members stay reachable via the class's own `bases`.
    pub(crate) mro_cache: HashMap<u64, alloc::rc::Rc<Vec<Val>>>,
    pub(crate) exception_stack: Vec<ExceptionFrame>,
    /* Active finally/with cleanup reasons (innermost last), EndFinally pops one per body. */
    pub(crate) unwind_stack: Vec<types::Unwind>,
    /* Exception currently being handled in an except block, a bare `raise` re-raises it. */
    pub(crate) handling_exc: Option<Val>,
    pub(crate) functions: Vec<&'a (Vec<String>, SSAChunk, u16, u16)>,
    // (chunk_ptr, global fn ids), linear scan over a tiny list avoids HashMap monomorphization.
    pub(crate) fn_index: Vec<(*const SSAChunk, Vec<u32>)>,
    // function_parents maps to the lexical enclosing fi (None at module level), body_to_fi maps chunk->fi.
    pub(crate) function_parents: Vec<Option<usize>>,
    pub(crate) body_to_fi: HashMap<*const SSAChunk, usize>,
    pub(crate) body_maps: Vec<HashMap<String, usize>>,
    pub(crate) param_slots: Vec<Vec<(ParamKind, usize)>>,
    pub(crate) slot_templates: Vec<Vec<Val>>,
    /* Deduped template values, templates are static after init, so the GC marks this flat list instead of every per-function template. */
    pub(crate) template_roots: Vec<Val>,
    pub(crate) nonlocal_tables: Vec<Vec<(usize, usize)>>,
    /* Recycled fn_slots buffers, popped in exec_call, pushed back on normal return. Never a GC root (entries are cleared before reuse). */
    pub(crate) slot_pool: Vec<Vec<Val>>,
    pub(crate) needs_caller_slots: Vec<bool>,
    /* Bitmap of slots bound to a formal parameter, protected from caller-slot propagation. */
    pub(crate) is_param_slot: Vec<Vec<bool>>,
    /* Free-variable body slots (bare_name, slot, referenced version), used for caller-chunk base-name fallback. */
    pub(crate) body_free_loads: Vec<Vec<(String, usize, i64)>>,
    /* Per-chunk bare names the chunk itself binds (stores, Phi, params), drives closure-cell capture. */
    pub(crate) chunk_local_binds: HashMap<*const SSAChunk, alloc::rc::Rc<crate::util::hash::FxHashSet<String>>>,
    /* Coroutines currently inside `resume_coroutine`, re-entry raises like Python's already-executing guard. Transient, never snapshotted. */
    pub(crate) executing_coros: Vec<u64>,
    /* True once any builtin name is rebound at module scope, fused call sites then consult `module_state` first. */
    pub(crate) builtins_rebound: bool,
    pub(crate) is_async: Vec<bool>,
    pub(crate) default_slots: Vec<Vec<(usize, Val)>>,
    /* Pre-resolved `<name>_0` body slot for self-reference binding, None for lambdas. */
    pub(crate) self_ref_slot: Vec<Option<usize>>,
    pub(crate) opcode_caches: HashMap<*const SSAChunk, OpcodeCache>,
    /* Per-chunk `bare -> [(version, slot)]` index for the free-load fallback. */
    pub(crate) chunk_name_versions: HashMap<*const SSAChunk, NameVersionIndex>,
    /* Cached per (caller chunk, callee fi), name matching is static, so hash it once, not per call. */
    pub(crate) propagation_maps: HashMap<(*const SSAChunk, usize), PropagationMap>,
    /* Const-pool ptrs for caches currently checked out by live exec() frames. */
    pub(crate) active_const_pools: Vec<*const [Val]>,
    /* Slot-slice ptrs for every live exec() frame, GC roots so a frame's mutating locals survive a nested resume. */
    pub(crate) active_slots: Vec<*const [Val]>,
    pub(crate) with_stack: Vec<Val>,
    /* GC roots for operands popped off the stack but still read after a dunder call that can collect. */
    pub(crate) temp_roots: Vec<Val>,
    /* Weak flags for lists produced by iterator builtins (iter/map/filter/zip/enumerate/reversed), next() drains only these and plain lists raise TypeError. Weak so a swept slot can never alias a fresh list. */
    pub(crate) iter_marks: Vec<alloc::rc::Weak<core::cell::RefCell<Vec<Val>>>>,
    pub(crate) pending: Pending,
    /* Monotonic correlation id handed to each deferred host call, matched by `set_host_result_by_id`. */
    pub(crate) next_host_call_id: u64,
    /* Sync helpers that suspended during the current resume, drained into the active Coroutine on yield-save. Lives at VM scope (not `Pending`) because it propagates across dispatch frames, not within one. */
    pub(crate) pending_sync_frames: Vec<types::SyncFrame>,
    /* Overrides `exec`'s captured `exc_base`. Set by `resume_coroutine` to the level *before* restored exception frames so dispatch's handler search includes them, consumed once at exec entry. */
    pub(crate) pending_exec_exc_base: Option<usize>,
    /* Back-edges until the next preempt, 0 disables. */
    pub(crate) preempt_left: usize,
    pub(crate) preempt_every: usize,
    /* True while this `exec` frame can unwind. */
    pub(crate) frame_safe: bool,
    pub(crate) pending_exec_safe: bool,
    /* Cancellation drive flags set only within one `scheduler_step`, never snapshotted. */
    pub(crate) cancelling: bool,
    pub(crate) cancel_raise: bool,
    pub(crate) yielded: bool,
    /* Return value of the most recently exhausted iterator, read by `LoadYieldFrom` so `x = yield from it` evaluates to the subiterator's StopIteration value. */
    pub(crate) yield_from_value: Val,
    pub(crate) resume_ip: usize,
    pub output: Vec<String>,
    /* True when the last `output` entry is an unterminated line (print(end="") left it open). */
    pub(crate) output_open: bool,
    pub print_hook: Option<fn(&str)>,
    pub input_buffer: Vec<String>,
    pub event_queue: Vec<Val>,
    pub strict_input: bool,
    /* Byte offset of the deepest propagating error in the last run(). */
    pub(crate) error_byte_pos: Option<u32>,
    /* spec -> Module Val, populated by `init_modules`, read by LoadModule / import_module(). */
    pub(crate) module_table: HashMap<String, Val>,
    /* `fi -> module spec`, scopes the free-load fallback to the fn's own module. */
    pub(crate) fn_module: Vec<Option<String>>,
    /* Function names parallel to `functions`, consumed by traceback render. Empty = lambda. */
    pub(crate) function_names: Vec<String>,
    /* Active call frames (innermost at end), drained by the traceback renderer on error. */
    pub(crate) call_stack: Vec<CallFrame>,
    /* Cooperative scheduler for `run` / `gather` / `with_timeout`, one handle per coroutine. Single-driver model where only `top_loop` drives this, async builtins yield instead of recursing. */
    pub(crate) scheduler: Vec<CoroutineHandle>,
    /* Count of scheduler entries in `WaitingForChildren`, gates the sweep so the common (no-nested-run) tick is one comparison. */
    pub(crate) waiting_for_children_count: usize,
    /* Host-installed wall-clock (ns). */
    pub(crate) time_hook: Option<fn() -> u64>,
    /* Fallback monotonic counter when `time_hook` is None, reset each `run()`. */
    pub(crate) virtual_clock_ns: u64,
}

impl<'a> VM<'a> {
    pub fn new(chunk: &'a SSAChunk) -> Self { Self::with_limits(chunk, Limits::sandbox()) }

    pub fn with_limits(chunk: &'a SSAChunk, limits: Limits) -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            iter_stack: Vec::with_capacity(16),
            yields: Vec::new(),
            chunk,
            heap: HeapPool::new(limits.heap),
            globals: HashMap::default(),
            module_state: HashMap::default(),
            live_slots: Vec::new(),
            templates: Templates::new(),
            budget: limits.ops,
            depth: 0,
            max_calls: limits.calls,
            with_stack: Vec::new(),
            temp_roots: Vec::new(),
            iter_marks: Vec::new(),
            pending: Pending::new(),
            next_host_call_id: 0,
            pending_sync_frames: Vec::new(),
            pending_exec_exc_base: None,
            preempt_left: 0,
            preempt_every: 0,
            frame_safe: false,
            pending_exec_safe: false,
            cancelling: false,
            cancel_raise: false,
            yielded: false,
            yield_from_value: Val::none(),
            resume_ip: 0,
            strict_input: false,
            output: Vec::new(),
            output_open: false,
            print_hook: None,
            input_buffer: Vec::new(),
            event_queue: Vec::new(),
            observed_impure: Vec::new(),
            mro_cache: HashMap::default(),
            exception_stack: Vec::new(),
            unwind_stack: Vec::new(),
            handling_exc: None,
            error_byte_pos: None,
            module_table: HashMap::default(),
            fn_module: Vec::new(),
            function_names: Vec::new(),
            call_stack: Vec::new(),
            scheduler: Vec::new(),
            waiting_for_children_count: 0,
            time_hook: None,
            virtual_clock_ns: 0,
            functions: Vec::new(),
            fn_index: Vec::new(),
            function_parents: Vec::new(),
            body_to_fi: HashMap::default(),
            body_maps: Vec::new(),
            param_slots: Vec::new(),
            slot_templates: Vec::new(),
            template_roots: Vec::new(),
            nonlocal_tables: Vec::new(),
            slot_pool: Vec::new(),
            needs_caller_slots: Vec::new(),
            is_param_slot: Vec::new(),
            body_free_loads: Vec::new(),
            chunk_local_binds: HashMap::default(),
            executing_coros: Vec::new(),
            builtins_rebound: false,
            is_async: Vec::new(),
            default_slots: Vec::new(),
            self_ref_slot: Vec::new(),
            opcode_caches: HashMap::default(),
            chunk_name_versions: HashMap::default(),
            propagation_maps: HashMap::default(),
            active_const_pools: Vec::new(),
            active_slots: Vec::new(),
        };
        vm.build_function_table(chunk, None, None);
        vm.index_functions(0);
        for &name in BUILTIN_TYPES {
            if let Ok(type_obj) = vm.heap.alloc(HeapObj::Type(name.to_string())) {
                vm.globals.insert(name.to_string(), type_obj);
                vm.globals.insert(s!(str name, "_0"), type_obj);
            }
        }
        // Entry chunk's `__name__` is "__main__", inserted before slot_templates is built.
        if let Ok(main_name) = vm.heap.alloc(HeapObj::Str("__main__".to_string())) {
            vm.globals.insert("__name__".to_string(), main_name);
            vm.globals.insert("__name___0".to_string(), main_name);
        }
        // `NotImplemented` singleton, dunders return it to delegate to the reflected operator.
        if let Ok(ni) = vm.heap.alloc(HeapObj::NotImplemented) {
            vm.globals.insert("NotImplemented".to_string(), ni);
            vm.globals.insert("NotImplemented_0".to_string(), ni);
        }
        // Builtins as first-class NativeFn values so they can be rebound/passed around.
        for &id in NativeFnId::ALL {
            let name = id.name();
            if BUILTIN_TYPES.contains(&name) { continue; } // type names stay Type objects
            if let Ok(v) = vm.heap.alloc(HeapObj::NativeFn(id)) {
                vm.globals.insert(name.to_string(), v);
                vm.globals.insert(s!(str name, "_0"), v);
            }
        }
        // Slot templates built after all globals are registered.
        vm.index_templates(0);
        vm
    }

    /* Derived per-function tables for functions[start..], the REPL re-invokes this to extend them for each adopted chunk. */
    pub(crate) fn index_functions(&mut self, start: usize) {
        let end = self.functions.len();
        let new: Vec<HashMap<String, usize>> = self.functions[start..end].iter().map(|(_, body, _, _)| {
            body.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect()
        }).collect();
        self.body_maps.truncate(start);
        self.body_maps.extend(new);
        let new: Vec<Vec<(ParamKind, usize)>> = (start..end).map(|fi| {
            let (params, _, _, _) = self.functions[fi];
            let bm = &self.body_maps[fi];
            params.iter().map(|p| {
                // `~` prefix marks kw-only parameters (after a lone `*`).
                let kind = if p.starts_with("**") {
                    ParamKind::DoubleStar
                } else if p.starts_with('*') {
                    ParamKind::Star
                } else if p.starts_with('~') {
                    ParamKind::KwOnly
                } else {
                    ParamKind::Normal
                };
                // Strips both prefix and the `=` default marker for slot lookup.
                let bare = crate::parser::types::param_base_name(p);
                let slot = bm.get(&s!(str bare, "_0")).copied().unwrap_or(usize::MAX);
                (kind, slot)
            }).collect()
        }).collect();
        self.param_slots.truncate(start);
        self.param_slots.extend(new);

        // Pre-compute nonlocal resolution (canonical_body_slot, canonical_body_slot).
        let new: Vec<Vec<(usize, usize)>> = self.functions[start..end].iter().map(|(_, body, _, _)| {
            body.nonlocals.iter().filter_map(|base| {
                // Require an explicit `_<digits>` suffix, bare Nonlocal-operand slots aren't canonical.
                let canon = body.names.iter().enumerate()
                    .find(|(_, n)| crate::parser::SsaName::parse(n).map(|s| s.bare) == Some(base.as_str()))
                    .map(|(i, _)| body.alias_groups.get(i).and_then(|g| g.first().copied()).unwrap_or(i as u16) as usize)?;
                Some((canon, canon))
            }).collect()
        }).collect();
        self.nonlocal_tables.truncate(start);
        self.nonlocal_tables.extend(new);

        // True iff the body references names not in params/builtins/captures.
        let new: Vec<bool> = (start..end).map(|fi| {
            let (params, body, _, _) = self.functions[fi];
            let param_names: crate::util::hash::FxHashSet<&str> = params.iter().map(|p| crate::parser::types::param_base_name(p)).collect();
            body.names.iter().any(|n| {
                let base = crate::parser::ssa_strip(n);
                !param_names.contains(base) && !self.globals.contains_key(n)
            })
        }).collect();
        self.needs_caller_slots.truncate(start);
        self.needs_caller_slots.extend(new);

        // Bitmap of param-bound slots, avoids per-call BTreeSet allocation.
        let new: Vec<Vec<bool>> = (start..end).map(|fi| {
            let (_, body, _, _) = self.functions[fi];
            let n_slots = body.names.len();
            let mut bm = alloc::vec![false; n_slots];
            for &(_, slot) in &self.param_slots[fi] { if slot < n_slots { bm[slot] = true; } }
            bm
        }).collect();
        self.is_param_slot.truncate(start);
        self.is_param_slot.extend(new);

        // Canonical, non-param, never-written slots.
        let new: Vec<Vec<(String, usize, i64)>> = (start..end).map(|fi| {
            let (_, body, _, _) = self.functions[fi];
            let param_bm = &self.is_param_slot[fi];
            let mut written: crate::util::hash::FxHashSet<usize> = crate::util::hash::FxHashSet::default();
            for ins in &body.instructions {
                if matches!(ins.opcode, crate::parser::OpCode::StoreName | crate::parser::OpCode::Phi) {
                    written.insert(ins.operand as usize);
                }
            }
            body.names.iter().enumerate().filter_map(|(slot, name)| {
                let canon = body.alias_groups.get(slot).and_then(|g| g.first().copied()).unwrap_or(slot as u16) as usize;
                if canon != slot { return None; }
                if param_bm.get(slot).copied().unwrap_or(false) { return None; }
                if written.contains(&slot) { return None; }
                let parsed = crate::parser::SsaName::parse(name)?;
                Some((parsed.bare.to_string(), slot, parsed.version as i64))
            }).collect()
        }).collect();
        self.body_free_loads.truncate(start);
        self.body_free_loads.extend(new);

        // Self-reference slot, resolved once to avoid per-call `<base>_0` allocation.
        let new: Vec<Option<usize>> = (start..end).map(|fi| {
            let bare = self.function_names.get(fi)?;
            if bare.is_empty() { return None; }
            let key = s!(str bare, "_0");
            self.body_maps[fi].get(key.as_str()).copied()
        }).collect();
        self.self_ref_slot.truncate(start);
        self.self_ref_slot.extend(new);

        // Default-slot table of (slot, placeholder) entries the call path overwrites.
        let new: Vec<Vec<(usize, Val)>> = (start..end).map(|fi| {
            let (params, _, n_defaults, _) = self.functions[fi];
            if *n_defaults == 0 { return Vec::new(); }
            // Defaults map to `=`-marked params in source order, not the trailing N.
            params.iter().zip(self.param_slots[fi].iter())
                .filter(|(p, _)| p.ends_with('='))
                .map(|(_, &(_, slot))| (slot, Val::none()))
                .collect()
        }).collect();
        self.default_slots.truncate(start);
        self.default_slots.extend(new);
    }

    /* Templates read `globals`, so they build after builtin registration, rebuilding the deduped roots is cheap. */
    pub(crate) fn index_templates(&mut self, start: usize) {
        let new: Vec<Vec<Val>> = self.functions[start..].iter().map(|(_, body, _, _)| {
            self.fill_builtins(&body.names)
        }).collect();
        self.slot_templates.truncate(start);
        self.slot_templates.extend(new);
        let mut seen: crate::util::hash::FxHashSet<u64> = crate::util::hash::FxHashSet::default();
        self.template_roots = self.slot_templates.iter().flatten()
            .filter(|v| !v.is_undef() && seen.insert(v.0))
            .copied()
            .collect();
    }

    /* For the REPL, adopt `chunk` as the new entry module, state persists, only the new chunk executes. */
    pub fn adopt_entry_chunk(&mut self, chunk: &'a SSAChunk) {
        let start = self.functions.len();
        self.build_function_table(chunk, None, None);
        self.index_functions(start);
        self.index_templates(start);
        self.chunk = chunk;
    }

    /* Fresh op budget for the next REPL input. */
    pub fn reset_budget(&mut self, ops: usize) {
        self.budget = ops;
    }

    /* Mirror compile-time extern bindings by name, runs pre-exec so user rebinds win. */
    pub fn bind_chunk_externs(&mut self) -> Result<(), VmErr> {
        let chunk = self.chunk;
        for (name, &idx) in chunk.extern_index.iter() {
            if let Some(b) = chunk.extern_table.get(idx as usize) {
                let v = self.heap.alloc(HeapObj::Extern(b.clone()))?;
                self.module_state.insert(name.clone(), v);
            }
        }
        Ok(())
    }

    /* Discard transient execution state so a parked REPL interpreter can run its next input. The heap, globals, and module bindings persist. */
    pub fn clear_error_state(&mut self) {
        self.stack.clear();
        self.iter_stack.clear();
        self.exception_stack.clear();
        self.with_stack.clear();
        self.unwind_stack.clear();
        self.temp_roots.clear();
        self.call_stack.clear();
        self.scheduler.clear();
        self.pending_sync_frames.clear();
        self.executing_coros.clear();
        self.handling_exc = None;
        self.cancelling = false;
        self.cancel_raise = false;
        self.yielded = false;
        self.resume_ip = 0;
        self.depth = 0;
        self.pending = Pending::new();
    }
}
