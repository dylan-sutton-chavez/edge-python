pub mod types;
mod cache;
mod ops;
mod builtins;
pub(crate) mod handlers;
pub mod optimizer;
pub mod snapshot;

mod dispatch;
mod gc;
mod helpers;
mod init;

use crate::s;
use crate::modules::parser::{SSAChunk, BUILTIN_TYPES};
use crate::util::fx::FxHashMap as HashMap;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, Templates};
use alloc::{string::{String, ToString}, vec::Vec};

pub(crate) use types::ExceptionFrame;

#[derive(Clone, Copy)]
pub(crate) enum ParamKind { Normal, Star, DoubleStar, KwOnly }

/* Side-channel state passed between opcodes in one dispatch frame; grouped for auditability. */
pub(crate) struct Pending {
    /* Star/double-star spread bumps the next Call's argument count. */
    pub pos_delta: i32,
    pub kw_delta: i32,
    // Saved enclosing spread deltas (BeginArgs).
    pub delta_save: alloc::vec::Vec<(i32, i32)>,
    /* Current Call's byte offset; consumed by the traceback renderer. */
    pub call_byte_pos: Option<u32>,
    /* Wakeup deadline set by `sleep()` and consumed by the scheduler. */
    pub sleep_until_ns: Option<u64>,
    /* Set by `frame()`; consumed by `scheduler_step` to transition the coro to `WaitingFrame`. */
    pub host_frame_request: bool,
    /* Set by `receive()` on empty queue; transitions the coro to `WaitingEvent`. */
    pub event_wait_request: bool,
    /* Set by `call_extern` on deferred native; transitions the coro to `WaitingHostCall`. */
    pub host_call_request: bool,
    /* Correlation id of the deferred call; read by `scheduler_step` into `WaitingHostCall(id)`. */
    pub host_call_id: u64,
    /* Set by `call_run` / `call_gather` / `call_with_timeout` when they yield; transitions the outer to `WaitingForChildren`. */
    pub waiting_for_children: Option<(Vec<Val>, types::WaitKind)>,
    /* Lifted ExcInstance from `raise X(...)` so `except X as e` binds the real instance. */
    pub exc_val: Option<Val>,
    /* `(class, self)` for the next user-function call when it's invoked as a method; populated by method-dispatch paths and consumed by `run_body_with_frame`. */
    pub method_binding: Option<(Val, Val)>,
    /* Set when the preempt interval elapses; read by `top_loop` to yield `Preempted`. */
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
pub(crate) type NameVersionIndex = crate::util::fx::FxHashMap<String, Vec<(i64, usize)>>;

/* Free-load propagation entry: (bare name, body slot, caller [(version, slot)] candidates). */
pub(crate) type FreeLoadEntry = (String, u32, Vec<(i64, u32)>);

/* Static caller->callee propagation data for one (caller chunk, callee fi) pair; built once, then per call is pure slot reads (no string hashing). */
pub(crate) struct PropInfo {
    /* Caller/callee share the lexical scope and module: late-binding, overwrite freely. */
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
    /* User-mutated module-level state, keyed by bare name; mirrors entry-chunk stores and backs `global` declarations. */
    pub(crate) module_state: HashMap<String, Val>,
    pub(crate) live_slots: Vec<Val>,
    pub(crate) templates: Templates,
    pub(crate) budget: usize,
    pub(crate) depth: usize,
    pub(crate) max_calls: usize,
    pub(crate) observed_impure: Vec<bool>,
    // C3 method-resolution order per class, keyed by the class Val's heap bits. Computed once at MakeClass (the class graph is a static DAG); a reused slot is overwritten by its new class, so stale entries are never read (lookup checks HeapObj::Class first). Not a GC root: MRO members stay reachable via the class's own `bases`.
    pub(crate) mro_cache: HashMap<u64, alloc::rc::Rc<Vec<Val>>>,
    pub(crate) exception_stack: Vec<ExceptionFrame>,
    /* Active finally/with cleanup reasons (innermost last); EndFinally pops one per body. */
    pub(crate) unwind_stack: Vec<types::Unwind>,
    /* Exception currently being handled in an except block; a bare `raise` re-raises it. */
    pub(crate) handling_exc: Option<Val>,
    pub(crate) functions: Vec<&'a (Vec<String>, SSAChunk, u16, u16)>,
    // (chunk_ptr, global fn ids); linear scan over a tiny list avoids HashMap monomorphization.
    pub(crate) fn_index: Vec<(*const SSAChunk, Vec<u32>)>,
    // function_parents: lexical enclosing fi (None at module level); body_to_fi: chunk->fi.
    pub(crate) function_parents: Vec<Option<usize>>,
    pub(crate) body_to_fi: HashMap<*const SSAChunk, usize>,
    pub(crate) body_maps: Vec<HashMap<String, usize>>,
    pub(crate) param_slots: Vec<Vec<(ParamKind, usize)>>,
    pub(crate) slot_templates: Vec<Vec<Val>>,
    pub(crate) nonlocal_tables: Vec<Vec<(usize, usize)>>,
    /* Recycled fn_slots buffers; popped in exec_call, pushed back on normal return. Never a GC root (entries are cleared before reuse). */
    pub(crate) slot_pool: Vec<Vec<Val>>,
    pub(crate) needs_caller_slots: Vec<bool>,
    /* Bitmap: slot bound to a formal parameter; protected from caller-slot propagation. */
    pub(crate) is_param_slot: Vec<Vec<bool>>,
    /* Free-variable body slots (bare_name, slot); used for caller-chunk base-name fallback. */
    pub(crate) body_free_loads: Vec<Vec<(String, usize)>>,
    pub(crate) is_async: Vec<bool>,
    pub(crate) default_slots: Vec<Vec<(usize, Val)>>,
    /* Pre-resolved `<name>_0` body slot for self-reference binding; None for lambdas. */
    pub(crate) self_ref_slot: Vec<Option<usize>>,
    pub(crate) opcode_caches: HashMap<*const SSAChunk, OpcodeCache>,
    /* Per-chunk `bare -> [(version, slot)]` index for the free-load fallback. */
    pub(crate) chunk_name_versions: HashMap<*const SSAChunk, NameVersionIndex>,
    /* Cached per (caller chunk, callee fi); name matching is static, so hash it once, not per call. */
    pub(crate) propagation_maps: HashMap<(*const SSAChunk, usize), PropagationMap>,
    /* Const-pool ptrs for caches currently checked out by live exec() frames. */
    pub(crate) active_const_pools: Vec<*const [Val]>,
    /* Slot-slice ptrs for every live exec() frame; GC roots so a frame's mutating locals survive a nested resume. */
    pub(crate) active_slots: Vec<*const [Val]>,
    /* Cached `ops == usize::MAX` so the hot path skips the budget decrement. */
    pub(crate) sandbox_off: bool,
    pub(crate) with_stack: Vec<Val>,
    /* GC roots for operands popped off the stack but still read after a dunder call that can collect. */
    pub(crate) temp_roots: Vec<Val>,
    pub(crate) pending: Pending,
    /* Monotonic correlation id handed to each deferred host call; matched by `set_host_result_by_id`. */
    pub(crate) next_host_call_id: u64,
    /* Sync helpers that suspended during the current resume; drained into the active Coroutine on yield-save. Lives at VM scope (not `Pending`) because it propagates across dispatch frames, not within one. */
    pub(crate) pending_sync_frames: Vec<types::SyncFrame>,
    /* Overrides `exec`'s captured `exc_base`. Set by `resume_coroutine` to the level *before* restored exception frames so dispatch's handler search includes them; consumed once at exec entry. */
    pub(crate) pending_exec_exc_base: Option<usize>,
    /* Back-edges left before the next preempt; 0 disables. Reloaded from `preempt_every`. */
    pub(crate) preempt_left: usize,
    pub(crate) preempt_every: usize,
    /* True while this `exec` frame can unwind; only the Call path and `scheduler_step` set it. */
    pub(crate) frame_safe: bool,
    pub(crate) pending_exec_safe: bool,
    pub(crate) yielded: bool,
    /* Return value of the most recently exhausted iterator; read by `LoadYieldFrom` so `x = yield from it` evaluates to the subiterator's StopIteration value. */
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
    /* spec -> Module Val, populated by `init_modules`; read by LoadModule / import_module(). */
    pub(crate) module_table: HashMap<String, Val>,
    /* `fi -> module spec`; scopes the free-load fallback to the fn's own module. */
    pub(crate) fn_module: Vec<Option<String>>,
    /* Function names parallel to `functions`; consumed by traceback render. Empty = lambda. */
    pub(crate) function_names: Vec<String>,
    /* Active call frames (innermost at end); drained by the traceback renderer on error. */
    pub(crate) call_stack: Vec<CallFrame>,
    /* Cooperative scheduler for `run` / `gather` / `with_timeout`; one handle per coroutine. Single-driver model: only `top_loop` drives this, async builtins yield instead of recursing. */
    pub(crate) scheduler: Vec<CoroutineHandle>,
    /* Count of scheduler entries in `WaitingForChildren`; gates the sweep so the common (no-nested-run) tick is one comparison. */
    pub(crate) waiting_for_children_count: usize,
    /* Host-installed wall-clock (ns). */
    pub(crate) time_hook: Option<fn() -> u64>,
    /* Fallback monotonic counter when `time_hook` is None; reset each `run()`. */
    pub(crate) virtual_clock_ns: u64,
}

impl<'a> VM<'a> {
    pub fn new(chunk: &'a SSAChunk) -> Self { Self::with_limits(chunk, Limits::none()) }

    pub fn with_limits(chunk: &'a SSAChunk, limits: Limits) -> Self {
        let sandbox_off = limits.ops == usize::MAX;
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
            pending: Pending::new(),
            next_host_call_id: 0,
            pending_sync_frames: Vec::new(),
            pending_exec_exc_base: None,
            preempt_left: 0,
            preempt_every: 0,
            frame_safe: false,
            pending_exec_safe: false,
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
            nonlocal_tables: Vec::new(),
            slot_pool: Vec::new(),
            needs_caller_slots: Vec::new(),
            is_param_slot: Vec::new(),
            body_free_loads: Vec::new(),
            is_async: Vec::new(),
            default_slots: Vec::new(),
            self_ref_slot: Vec::new(),
            opcode_caches: HashMap::default(),
            chunk_name_versions: HashMap::default(),
            propagation_maps: HashMap::default(),
            active_const_pools: Vec::new(),
            active_slots: Vec::new(),
            sandbox_off,
        };
        vm.build_function_table(chunk, None, None);
        vm.body_maps = vm.functions.iter().map(|(_, body, _, _)| {
            body.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect()
        }).collect();
        vm.param_slots = (0..vm.functions.len()).map(|fi| {
            let (params, _, _, _) = vm.functions[fi];
            let bm = &vm.body_maps[fi];
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
                let bare = crate::modules::parser::types::param_base_name(p);
                let slot = bm.get(&s!(str bare, "_0")).copied().unwrap_or(usize::MAX);
                (kind, slot)
            }).collect()
        }).collect();

        // Pre-compute nonlocal resolution: (canonical_body_slot, canonical_body_slot).
        vm.nonlocal_tables = vm.functions.iter().map(|(_, body, _, _)| {
            body.nonlocals.iter().filter_map(|base| {
                // Require an explicit `_<digits>` suffix; bare Nonlocal-operand slots aren't canonical.
                let canon = body.names.iter().enumerate()
                    .find(|(_, n)| crate::modules::parser::SsaName::parse(n).map(|s| s.bare) == Some(base.as_str()))
                    .map(|(i, _)| body.alias_groups.get(i).and_then(|g| g.first().copied()).unwrap_or(i as u16) as usize)?;
                Some((canon, canon))
            }).collect()
        }).collect();

        // True iff the body references names not in params/builtins/captures.
        vm.needs_caller_slots = (0..vm.functions.len()).map(|fi| {
            let (params, body, _, _) = vm.functions[fi];
            let param_names: crate::util::fx::FxHashSet<&str> = params.iter().map(|p| crate::modules::parser::types::param_base_name(p)).collect();
            body.names.iter().any(|n| {
                let base = crate::modules::parser::ssa_strip(n);
                !param_names.contains(base) && !vm.globals.contains_key(n)
            })
        }).collect();

        // Bitmap of param-bound slots; avoids per-call BTreeSet allocation.
        vm.is_param_slot = (0..vm.functions.len()).map(|fi| {
            let (_, body, _, _) = vm.functions[fi];
            let n_slots = body.names.len();
            let mut bm = alloc::vec![false; n_slots];
            for &(_, slot) in &vm.param_slots[fi] { if slot < n_slots { bm[slot] = true; } }
            bm
        }).collect();

        // Canonical, non-param, never-written slots, built once at VM init.
        vm.body_free_loads = (0..vm.functions.len()).map(|fi| {
            let (_, body, _, _) = vm.functions[fi];
            let param_bm = &vm.is_param_slot[fi];
            let mut written: crate::util::fx::FxHashSet<usize> = crate::util::fx::FxHashSet::default();
            for ins in &body.instructions {
                if matches!(ins.opcode, crate::modules::parser::OpCode::StoreName | crate::modules::parser::OpCode::Phi) {
                    written.insert(ins.operand as usize);
                }
            }
            body.names.iter().enumerate().filter_map(|(slot, name)| {
                let canon = body.alias_groups.get(slot).and_then(|g| g.first().copied()).unwrap_or(slot as u16) as usize;
                if canon != slot { return None; }
                if param_bm.get(slot).copied().unwrap_or(false) { return None; }
                if written.contains(&slot) { return None; }
                let parsed = crate::modules::parser::SsaName::parse(name)?;
                Some((parsed.bare.to_string(), slot))
            }).collect()
        }).collect();

        // Self-reference slot, resolved once to avoid per-call `<base>_0` allocation.
        vm.self_ref_slot = (0..vm.functions.len()).map(|fi| {
            let bare = vm.function_names.get(fi)?;
            if bare.is_empty() { return None; }
            let key = s!(str bare, "_0");
            vm.body_maps[fi].get(key.as_str()).copied()
        }).collect();

        // Default-slot table: (slot, placeholder) entries the call path overwrites.
        vm.default_slots = (0..vm.functions.len()).map(|fi| {
            let (params, _, n_defaults, _) = vm.functions[fi];
            if *n_defaults == 0 { return Vec::new(); }
            // Defaults map to `=`-marked params in source order, not the trailing N.
            params.iter().zip(vm.param_slots[fi].iter())
                .filter(|(p, _)| p.ends_with('='))
                .map(|(_, &(_, slot))| (slot, Val::none()))
                .collect()
        }).collect();
        for &name in BUILTIN_TYPES {
            if let Ok(type_obj) = vm.heap.alloc(HeapObj::Type(name.to_string())) {
                vm.globals.insert(name.to_string(), type_obj);
                vm.globals.insert(s!(str name, "_0"), type_obj);
            }
        }
        // Entry chunk's `__name__` is "__main__"; inserted before slot_templates is built.
        if let Ok(main_name) = vm.heap.alloc(HeapObj::Str("__main__".to_string())) {
            vm.globals.insert("__name__".to_string(), main_name);
            vm.globals.insert("__name___0".to_string(), main_name);
        }
        // `NotImplemented` singleton; dunders return it to delegate to the reflected operator.
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
        vm.slot_templates = vm.functions.iter().map(|(_, body, _, _)| {
            vm.fill_builtins(&body.names)
        }).collect();
        vm
    }
}
