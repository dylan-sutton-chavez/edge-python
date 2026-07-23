use alloc::{rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;
use crate::util::fx::FxHashMap as HashMap;

pub mod coro;
pub mod eq;
pub mod err;
pub mod math;
pub mod scheduler;

pub use coro::*;
pub use eq::*;
pub use err::*;
pub use math::*;
pub use scheduler::*;

/* Per-execution caps: recursion depth, op budget, heap quota. */
pub struct Limits { pub calls: usize, pub ops: usize, pub heap: usize }

impl Limits {
    pub fn none() -> Self { Self { calls: 1_000, ops: usize::MAX, heap: 10_000_000 } }
    pub fn sandbox() -> Self { Self { calls: 256, ops: 100_000_000, heap: 100_000 } }
}

/* Host-provided callable, resolved at compile time and dispatched by `CallExtern`. `Arc<dyn Fn>` lets loaders capture stateful handles; `pure` enables memoization. Third arg is the kwargs slot, `None` for plain positional calls, `Some(dict_val)` when the caller used `name=value` syntax. */
pub type ExternCallable =
    alloc::sync::Arc<dyn Fn(&mut HeapPool, &[Val], Option<Val>) -> Result<Val, VmErr> + Send + Sync>;

#[derive(Clone)]
pub struct ExternFn {
    pub name: String,
    pub func: ExternCallable,
    pub pure: bool,
}

impl core::fmt::Debug for ExternFn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternFn").field("name", &self.name).field("pure", &self.pure).finish()
    }
}

/* NaN-boxed 8-byte value (47-bit int, float, bool, None, undef, heap idx); layout in `abi::nan_box`. */
use crate::abi::nan_box::{
    QNAN, SIGN, TAG_UNDEF, TAG_NONE, TAG_TRUE, TAG_FALSE, TAG_INT, TAG_HEAP,
    INT_PAYLOAD_MASK,
};

#[derive(Clone, Copy, Debug)]
pub struct Val(pub(crate) u64);

impl PartialEq for Val {
    #[inline] fn eq(&self, o: &Self) -> bool {
        if self.0 == o.0 { return true; }
        // Mirror Hash: numeric immediates unify by value so True==1==1.0 share a dict/set key.
        let num = |v: &Val| -> Option<f64> {
            if v.is_int() { Some(v.as_int() as f64) }
            else if v.is_bool() { Some(v.as_bool() as i64 as f64) }
            else if v.is_float() { Some(v.as_float()) }
            else { None }
        };
        match (num(self), num(o)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }
}
impl Eq for Val {}

impl core::hash::Hash for Val {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Hash ints as i64 and integral floats as that same int so 1 and 1.0 share a key (47-bit ints fit f64 losslessly). Bit-hashing int-valued f64s clusters (zero low mantissa bits) and degrades int-keyed dict/set to O(n^2).
        if self.is_int() {
            state.write_i64(self.as_int());
        } else if self.is_bool() {
            // bool unifies with int 0/1 so True and 1 share a dict/set slot.
            state.write_i64(self.as_bool() as i64);
        } else if self.is_float() {
            let f = self.as_float();
            // Round-trip via i64 tests "integral and in i64 range" in one step (the cast saturates out-of-range / non-finite, so they fail to round-trip). No std `f64::fract`.
            if f as i64 as f64 == f { state.write_i64(f as i64); }
            else { state.write_u64(f.to_bits()); }
        } else {
            self.0.hash(state);
        }
    }
}

impl Val {
    /* Canonical NaN kept outside the tag space so is_float() stays true. */
    const CANON_NAN: u64 = 0x7FF8_0000_0000_0000;
    #[inline(always)] pub fn float(f: f64) -> Self {
        let bits = f.to_bits();
        if (bits & QNAN) == QNAN { Self(Self::CANON_NAN) } else { Self(bits) }
    }
    pub const INT_MAX: i64 = 0x0000_7FFF_FFFF_FFFF;
    pub const INT_MIN: i64 = -0x0000_8000_0000_0000;
    #[inline(always)] pub fn int(i: i64) -> Self {
        Self(TAG_INT | (i as u64 & INT_PAYLOAD_MASK))
    }
    #[inline(always)] pub fn int_checked(i: i64) -> Option<Self> {
        if !(Self::INT_MIN..=Self::INT_MAX).contains(&i) { None } else { Some(Self::int(i)) }
    }
    #[inline(always)] pub fn none() -> Self { Self(TAG_NONE) }
    #[inline(always)] pub fn bool(b: bool) -> Self { Self(if b { TAG_TRUE } else { TAG_FALSE }) }
    #[inline(always)] pub fn heap(idx: u32) -> Self { Self(TAG_HEAP | ((idx as u64) << 4)) }
    /* Unbound-local sentinel; lets slots stay Vec<Val> and LoadName check via one u64 compare. */
    #[inline(always)] pub fn undef() -> Self { Self(TAG_UNDEF) }

    #[inline(always)] pub fn is_float(&self) -> bool { (self.0 & QNAN) != QNAN }
    #[inline(always)] pub fn is_int(&self) -> bool { (self.0 & (QNAN | SIGN)) == TAG_INT }
    #[inline(always)] pub fn is_none(&self) -> bool { self.0 == TAG_NONE }
    #[inline(always)] pub fn is_true(&self) -> bool { self.0 == TAG_TRUE }
    #[inline(always)] pub fn is_false(&self) -> bool { self.0 == TAG_FALSE }
    #[inline(always)] pub fn is_bool(&self) -> bool { self.0 == TAG_TRUE || self.0 == TAG_FALSE }
    #[inline(always)] pub fn is_undef(&self) -> bool { self.0 == TAG_UNDEF }
    #[inline(always)] pub fn is_heap(&self) -> bool {
        (self.0 & QNAN) == QNAN && (self.0 & SIGN) == 0 && (self.0 & 0xF) >= 4
    }

    #[inline(always)] pub fn as_float(&self) -> f64 { f64::from_bits(self.0) }
    /* Wire-format accessors (FFI / WASM loader / SDK). */
    #[inline(always)] pub fn raw(&self) -> u64 { self.0 }
    /// # Safety
    ///
    /// `u` must come from `Val::raw()` on a live heap slot in the same VM.
    #[inline(always)] pub unsafe fn from_raw(u: u64) -> Self { Self(u) }
    #[inline(always)] pub fn as_int(&self) -> i64 {
        let raw = (self.0 & INT_PAYLOAD_MASK) as i64;
        (raw << 16) >> 16
    }
    #[inline(always)] pub fn as_bool(&self) -> bool { self.0 == TAG_TRUE }
    #[inline(always)] pub fn as_heap(&self) -> u32 { ((self.0 >> 4) & 0x0FFF_FFFF) as u32 }
}


/* Heap-allocated value variants in HeapPool's arena, indexed via Val::heap. */
#[derive(Clone, Debug)]
pub enum HeapObj {
    Str(String),
    Bytes(Vec<u8>),
    List(Rc<RefCell<Vec<Val>>>),
    Dict(Rc<RefCell<DictMap>>),
    Set(Rc<RefCell<ValSet>>),
    /* Immutable, hashable counterpart of Set; built via `frozenset(iter)`. */
    FrozenSet(Rc<ValSet>),
    Tuple(Vec<Val>),
    Func(usize, Vec<Val>, Vec<(usize, Val)>),
    Range(i64, i64, i64),
    Slice(Val, Val, Val),
    // True `...` singleton, distinct from any string.
    Ellipsis,
    Type(String),
    // `NotImplemented` singleton; dunder return sentinel that triggers the reflected operator fallback.
    NotImplemented,
    /* Wide-int slow path (i128); `int_to_val` canonicalises so 47-bit values stay inline. */
    LongInt(i128),
    /* Exception instance: type name + ctor args (exposed via `.args`). */
    ExcInstance(String, Vec<Val>),
    BoundMethod(Val, BuiltinMethodId),
    NativeFn(NativeFnId),
    // `bases` lists direct parents in declared order; `resolve_attr` DFS-walks them on miss.
    // Members are mutable so a decorator or `cls.attr = ...` can add or replace class attributes.
    Class(String, Vec<Val>, Rc<RefCell<Vec<(String, Val)>>>),
    Instance(Val, Rc<RefCell<DictMap>>),
    // `(recv, func, class)`; `class` is where `func` was found so the called frame knows what `super()` should skip past.
    BoundUserMethod(Val, Val, Val),
    // `super()` proxy: attribute access walks the bases of `cls` (skipping `cls` itself); methods bind to `recv`.
    Super(Val, Val),
    // `(getter, setter)`; `setter == none()` for getter-only properties, written via `@property` / `@x.setter`.
    Property(Val, Val),
    // Intermediate produced by `prop.setter`: callable that takes a function and returns a new `Property` with the setter attached.
    PropertySetter(Val),
    // `staticmethod(func)`: wraps a function so attribute lookup returns it unbound.
    StaticMethod(Val),
    // `classmethod(func)`: attribute lookup binds the class.
    ClassMethod(Val),
    // Trailing `Vec<SyncFrame>` stacks suspended sync sub-calls (innermost-last); resume walks inside-out, each return lands on next frame's Call site. `BodyRef` discriminates user-fn coros from the implicit module-body coro. Final `Vec<ExceptionFrame>` carries try/except across yields.
    Coroutine(usize, Vec<Val>, Vec<Val>, BodyRef, Vec<IterFrame>, Vec<SyncFrame>, Vec<ExceptionFrame>),
    /* Produced by `import m`; attr access via LoadAttr, calls fuse through CallMethod. */
    Module(String, Vec<(String, Val)>),
    /* A native binding lifted to a first-class callable. */
    Extern(ExternFn),
}

pub use crate::modules::vm::handlers::methods::BuiltinMethodId;

// Single source of truth per builtin: variant => name, arity (`var` = variadic, validated in handler).
macro_rules! builtins {
    ( $( $variant:ident => $name:literal, $arity:tt );* $(;)? ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum NativeFnId { $( $variant ),* }

        impl NativeFnId {
            // All variants in declaration order; drives global registration.
            pub const ALL: &'static [NativeFnId] = &[ $( NativeFnId::$variant ),* ];
            // Python-visible name.
            pub fn name(self) -> &'static str { match self { $( NativeFnId::$variant => $name ),* } }
            // Inverse of `name`; used by snapshot restore.
            pub fn from_name(n: &str) -> Option<Self> { match n { $( $name => Some(NativeFnId::$variant), )* _ => None } }
            // Fixed positional arity; None means the handler validates the count.
            pub fn arity(self) -> Option<u16> { match self { $( NativeFnId::$variant => builtins!(@a $arity) ),* } }
        }
    };
    (@a var) => { None };
    (@a $n:literal) => { Some($n) };
}

builtins! {
    Print => "print", var;
    Len => "len", 1; Abs => "abs", 1; Str => "str", var; Int => "int", var; Float => "float", var;
    Bool => "bool", var; Type => "type", 1; Chr => "chr", 1; Ord => "ord", 1;
    Range => "range", var; Round => "round", var; Min => "min", var; Max => "max", var; Sum => "sum", var;
    Sorted => "sorted", 1; Enumerate => "enumerate", var; Zip => "zip", var;
    List => "list", var; Tuple => "tuple", var; Dict => "dict", var; Set => "set", var;
    IsInstance => "isinstance", 2; IsSubclass => "issubclass", 2; Input => "input", 0;
    All => "all", var; Any => "any", var;
    Bin => "bin", 1; Oct => "oct", 1; Hex => "hex", 1; Divmod => "divmod", 2; Pow => "pow", var;
    Repr => "repr", 1; Reversed => "reversed", 1; Callable => "callable", 1; Id => "id", 1; Hash => "hash", 1;
    Format => "format", var; GetAttr => "getattr", var; HasAttr => "hasattr", 2; SetAttr => "setattr", 3; DelAttr => "delattr", 2;
    Next => "next", var; Run => "run", var; Sleep => "sleep", 1;
    Receive => "receive", 0; Map => "map", var; Filter => "filter", 2; Iter => "iter", var;
    Bytes => "bytes", var; ImportModule => "import_module", 1; Slice => "slice", var; Vars => "vars", 1;
    Gather => "gather", var; WithTimeout => "with_timeout", 2; Cancel => "cancel", 1;
    BytesFromHex => "bytes_fromhex", 1; IntFromBytes => "int_from_bytes", 2; IntToBytes => "int_to_bytes", 3; FrozenSet => "frozenset", var;
    Globals => "globals", 0; Locals => "locals", 0;
    Super => "super", 0;
    Property => "property", var;
    StaticMethod => "staticmethod", 1;
    Frame => "frame", var;
    ClassMethod => "classmethod", 1;
}

/* Content-hashed set: equal-by-content values dedup in O(1) regardless of heap identity. */
#[derive(Clone, Debug, Default)]
pub struct ValSet {
    t: hashbrown::HashTable<Val>,
}

impl ValSet {
    pub fn new() -> Self { Self { t: hashbrown::HashTable::new() } }
    pub fn with_capacity(cap: usize) -> Self { Self { t: hashbrown::HashTable::with_capacity(cap) } }
    pub fn len(&self) -> usize { self.t.len() }
    pub fn is_empty(&self) -> bool { self.t.is_empty() }
    pub fn clear(&mut self) { self.t.clear(); }
    pub fn iter(&self) -> impl Iterator<Item = &Val> + '_ { self.t.iter() }

    pub fn contains(&self, v: Val, heap: &HeapPool) -> bool {
        let h = hash_val_with_heap(v, heap);
        self.t.find(h, |&k| eq_vals_with_heap(k, v, heap)).is_some()
    }
    /* Returns true when newly inserted (content-equal value not already present). */
    pub fn insert(&mut self, v: Val, heap: &HeapPool) -> bool {
        let h = hash_val_with_heap(v, heap);
        if self.t.find(h, |&k| eq_vals_with_heap(k, v, heap)).is_some() { return false; }
        self.t.insert_unique(h, v, |&k| hash_val_with_heap(k, heap));
        true
    }
    pub fn remove(&mut self, v: Val, heap: &HeapPool) -> bool {
        let h = hash_val_with_heap(v, heap);
        match self.t.find_entry(h, |&k| eq_vals_with_heap(k, v, heap)) {
            Ok(e) => { e.remove(); true }
            Err(_) => false,
        }
    }
}

/* Insertion-ordered dict: Vec for ordering, content-hashed HashTable<usize> index for O(1) get. */
#[derive(Clone, Debug)]
pub struct DictMap {
    pub entries: Vec<(Val, Val)>,
    index: hashbrown::HashTable<usize>,
}

impl DictMap {
    /* Entries only; `rebuild_index` once the heap lives. */
    pub(crate) fn from_entries(entries: Vec<(Val, Val)>) -> Self {
        Self { entries, index: hashbrown::HashTable::new() }
    }

    /* Rehash after restore; hashing reads the heap. */
    pub(crate) fn rebuild_index(&mut self, heap: &HeapPool) {
        self.index.clear();
        let e = &self.entries;
        for i in 0..e.len() {
            let h = hash_val_with_heap(e[i].0, heap);
            self.index.insert_unique(h, i, |&j| hash_val_with_heap(e[j].0, heap));
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self { entries: Vec::with_capacity(cap), index: hashbrown::HashTable::with_capacity(cap) }
    }

    pub fn get(&self, key: &Val, heap: &HeapPool) -> Option<&Val> {
        let e = &self.entries;
        let h = hash_val_with_heap(*key, heap);
        self.index.find(h, |&i| eq_vals_with_heap(e[i].0, *key, heap)).map(|&i| &self.entries[i].1)
    }

    pub fn contains_key(&self, key: &Val, heap: &HeapPool) -> bool {
        let e = &self.entries;
        let h = hash_val_with_heap(*key, heap);
        self.index.find(h, |&i| eq_vals_with_heap(e[i].0, *key, heap)).is_some()
    }

    pub fn insert(&mut self, key: Val, value: Val, heap: &HeapPool) {
        let h = hash_val_with_heap(key, heap);
        let e = &self.entries;
        if let Some(&i) = self.index.find(h, |&i| eq_vals_with_heap(e[i].0, key, heap)) {
            self.entries[i].1 = value;
        } else {
            let i = self.entries.len();
            self.entries.push((key, value));
            let e = &self.entries;
            self.index.insert_unique(h, i, |&j| hash_val_with_heap(e[j].0, heap));
        }
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (Val, Val)> + '_ {
        self.entries.iter().map(|&(k, v)| (k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = Val> + '_ {
        self.entries.iter().map(|&(k, _)| k)
    }

    pub fn from_pairs(pairs: Vec<(Val, Val)>, heap: &HeapPool) -> Self {
        let mut dm = Self::with_capacity(pairs.len());
        for (k, v) in pairs { dm.insert(k, v, heap); }
        dm
    }

    /* Vec shift after remove invalidates stored positions; rebuild the whole index. */
    fn reindex(&mut self, heap: &HeapPool) {
        self.index.clear();
        for i in 0..self.entries.len() {
            let e = &self.entries;
            let h = hash_val_with_heap(e[i].0, heap);
            self.index.insert_unique(h, i, |&j| hash_val_with_heap(e[j].0, heap));
        }
    }
}

impl Default for DictMap {
    fn default() -> Self { Self::new() }
}

impl DictMap {
    pub fn new() -> Self { Self { entries: Vec::new(), index: hashbrown::HashTable::new() } }

    pub fn remove(&mut self, key: &Val, heap: &HeapPool) -> Option<Val> {
        let e = &self.entries;
        let h = hash_val_with_heap(*key, heap);
        let idx = *self.index.find(h, |&i| eq_vals_with_heap(e[i].0, *key, heap))?;
        let val = self.entries[idx].1;
        self.entries.remove(idx);
        self.reindex(heap);
        Some(val)
    }
}

/* Visits every reachable `Val` once; single source of truth for GC traversal. */
pub(crate) fn for_each_val(obj: &HeapObj, mut f: impl FnMut(Val)) {
    match obj {
        HeapObj::Tuple(items) => for &v in items { f(v); },
        HeapObj::Slice(a, b, c) => { f(*a); f(*b); f(*c); }
        HeapObj::List(rc) => for &v in rc.borrow().iter() { f(v); },
        HeapObj::Dict(rc) => for (k, v) in rc.borrow().iter() { f(k); f(v); },
        HeapObj::Set(rc) => for &v in rc.borrow().iter() { f(v); },
        HeapObj::FrozenSet(rc) => for &v in rc.iter() { f(v); },
        HeapObj::BoundMethod(recv, _) => f(*recv),
        HeapObj::Class(_, bases, methods) => {
            for &v in bases { f(v); }
            for (_, v) in methods.borrow().iter() { f(*v); }
        }
        HeapObj::BoundUserMethod(r, fu, cls) => { f(*r); f(*fu); f(*cls); }
        HeapObj::Super(cls, recv) => { f(*cls); f(*recv); }
        HeapObj::Property(g, s) => { f(*g); f(*s); }
        HeapObj::PropertySetter(p) => f(*p),
        HeapObj::StaticMethod(func) => f(*func),
        HeapObj::ClassMethod(func) => f(*func),
        HeapObj::Instance(cls, attrs) => {
            f(*cls);
            for (k, v) in attrs.borrow().iter() { f(k); f(v); }
        }
        HeapObj::Coroutine(_, slots, stack, _, iters, sub_frames, _) => {
            for &v in slots { f(v); }
            for &v in stack { f(v); }
            for fr in iters { fr.for_each_val(&mut f); }
            for sf in sub_frames { sf.for_each_val(&mut f); }
        }
        HeapObj::Func(_, defaults, captures) => {
            for &v in defaults { f(v); }
            for &(_, v) in captures { f(v); }
        }
        HeapObj::Module(_, attrs) => for (_, v) in attrs { f(*v); },
        HeapObj::ExcInstance(_, args) => for &v in args { f(v); },
        // Variants without Val payloads, terminal, nothing to trace.
        HeapObj::Str(_) | HeapObj::Bytes(_) | HeapObj::LongInt(_)
        | HeapObj::Type(_) | HeapObj::NativeFn(_) | HeapObj::Range(..)
        | HeapObj::Extern(_) | HeapObj::Ellipsis | HeapObj::NotImplemented => {}
    }
}

/* Arena allocator with mark-sweep GC and string interning (<=128 bytes). */
struct HeapSlot {
    obj: Option<HeapObj>,
    marked: bool,
}

pub struct HeapPool {
    slots: Vec<HeapSlot>,
    free_list: Vec<u32>,
    live: usize,
    pub gc_threshold: usize,
    alloc_count: usize,
    limit: usize,
    strings: HashMap<String, u32>,
    /* Interns short bytes literals so equal `b"..."` share a Val (Hash uses raw bits). */
    bytes_intern: HashMap<Vec<u8>, u32>,
    /* Interns LongInt by value so equal i128s share a Val and stay hash/eq consistent. */
    longints: HashMap<i128, u32>,
    /* Interns Type objects by name so `type(x) is set` and `type(None) is type(None)` hold. */
    types: HashMap<String, u32>,
    // Cached Ellipsis slot index so `... is ...` is True (singleton parity).
    ellipsis_idx: Option<u32>,
    // Same singleton invariant as `ellipsis_idx`, but for `NotImplemented`.
    notimpl_idx: Option<u32>,
    /* Reused across mark() calls; cleared not freed, so GC never re-allocates under pressure. */
    mark_worklist: Vec<u32>,
}

impl HeapPool {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            live: 0,
            gc_threshold: 512,
            alloc_count: 0,
            limit,
            strings: HashMap::default(),
            bytes_intern: HashMap::default(),
            longints: HashMap::default(),
            types: HashMap::default(),
            ellipsis_idx: None,
            notimpl_idx: None,
            mark_worklist: Vec::with_capacity(64),
        }
    }

    /* Existing interned/singleton slot for `obj`, if any. Str/Bytes intern only when small. */
    #[inline]
    fn intern_lookup(&self, obj: &HeapObj) -> Option<u32> {
        match obj {
            HeapObj::Str(s) if s.len() <= 128 => self.strings.get(s).copied(),
            HeapObj::Bytes(b) if b.len() <= 128 => self.bytes_intern.get(b).copied(),
            HeapObj::LongInt(i) => self.longints.get(i).copied(),
            HeapObj::Type(name) => self.types.get(name).copied(),
            HeapObj::Ellipsis => self.ellipsis_idx,
            HeapObj::NotImplemented => self.notimpl_idx,
            _ => None,
        }
    }

    pub fn alloc(&mut self, obj: HeapObj) -> Result<Val, VmErr> {
        if let Some(idx) = self.intern_lookup(&obj) { return Ok(Val::heap(idx)); }
        if self.live >= self.limit { return Err(cold_heap()); }
        if self.slots.len() >= (1 << 28) { return Err(VmErr::Heap); }

        let idx = if let Some(i) = self.free_list.pop() {
            self.slots[i as usize] = HeapSlot { obj: Some(obj), marked: false };
            i
        } else {
            let i = self.slots.len() as u32;
            self.slots.push(HeapSlot { obj: Some(obj), marked: false });
            i
        };

        match self.slots[idx as usize].obj.as_ref().unwrap() {
            HeapObj::Str(s) if s.len() <= 128 => { self.strings.insert(s.clone(), idx); }
            HeapObj::Bytes(b) if b.len() <= 128 => { self.bytes_intern.insert(b.clone(), idx); }
            HeapObj::LongInt(i) => { self.longints.insert(*i, idx); }
            HeapObj::Type(name) => { self.types.insert(name.clone(), idx); }
            HeapObj::Ellipsis => { self.ellipsis_idx = Some(idx); }
            HeapObj::NotImplemented => { self.notimpl_idx = Some(idx); }
            _ => {}
        }

        self.live += 1;
        self.alloc_count += 1;
        Ok(Val::heap(idx))
    }

    pub fn mark(&mut self, v: Val) {
        if !v.is_heap() { return; }
        /* Split borrow: closure needs &mut mark_worklist while we read slots. */
        let HeapPool { slots, mark_worklist, .. } = self;
        mark_worklist.push(v.as_heap());
        while let Some(idx) = mark_worklist.pop() {
            let idx = idx as usize;
            if slots[idx].marked { continue; }
            slots[idx].marked = true;
            if let Some(obj) = &slots[idx].obj {
                for_each_val(obj, |val| { if val.is_heap() { mark_worklist.push(val.as_heap()); } });
            }
        }
    }

    pub fn sweep(&mut self) {
        for idx in 0..self.slots.len() {
            let slot = &mut self.slots[idx];
            match &slot.obj {
                None => {}
                Some(_) if slot.marked => { slot.marked = false; }
                Some(obj) => {
                    // Evict any intern-table entry, then free the slot.
                    match obj {
                        HeapObj::Str(s) => { self.strings.remove(s); }
                        HeapObj::Bytes(b) => { self.bytes_intern.remove(b); }
                        HeapObj::LongInt(i) => { self.longints.remove(i); }
                        HeapObj::Type(name) => { self.types.remove(name); }
                        // Cached singleton index becomes stale when its slot is freed.
                        HeapObj::Ellipsis if self.ellipsis_idx == Some(idx as u32) => { self.ellipsis_idx = None; }
                        HeapObj::NotImplemented if self.notimpl_idx == Some(idx as u32) => { self.notimpl_idx = None; }
                        _ => {}
                    }
                    slot.obj = None;
                    self.free_list.push(idx as u32);
                    self.live -= 1;
                }
            }
        }

        self.gc_threshold = (self.live * 2).max(512);
        self.alloc_count = 0;

        // Cap free list at 512K slots; sort to prefer low indices and reduce fragmentation.
        if self.free_list.len() > 524_288 {
            self.free_list.sort_unstable();
            self.free_list.truncate(524_288);
        }
    }

    /* One entry per slot; None when free. */
    pub(crate) fn snapshot_objs(&self) -> impl Iterator<Item = Option<&HeapObj>> {
        self.slots.iter().map(|s| s.obj.as_ref())
    }

    /* Replace the pool; rebuild free and intern tables. */
    pub(crate) fn restore_objs(&mut self, objs: Vec<Option<HeapObj>>) {
        self.slots = objs.into_iter().map(|obj| HeapSlot { obj, marked: false }).collect();
        self.free_list.clear();
        self.strings.clear();
        self.bytes_intern.clear();
        self.longints.clear();
        self.types.clear();
        self.ellipsis_idx = None;
        self.notimpl_idx = None;
        self.live = 0;
        for idx in 0..self.slots.len() {
            match &self.slots[idx].obj {
                None => self.free_list.push(idx as u32),
                Some(obj) => {
                    self.live += 1;
                    let idx = idx as u32;
                    match obj {
                        HeapObj::Str(s) if s.len() <= 128 => { self.strings.insert(s.clone(), idx); }
                        HeapObj::Bytes(b) if b.len() <= 128 => { self.bytes_intern.insert(b.clone(), idx); }
                        HeapObj::LongInt(i) => { self.longints.insert(*i, idx); }
                        HeapObj::Type(name) => { self.types.insert(name.clone(), idx); }
                        HeapObj::Ellipsis => { self.ellipsis_idx = Some(idx); }
                        HeapObj::NotImplemented => { self.notimpl_idx = Some(idx); }
                        _ => {}
                    }
                }
            }
        }
        self.gc_threshold = (self.live * 2).max(512);
        self.alloc_count = 0;
    }

    /* Swap a live slot's object during restore. */
    pub(crate) fn replace_obj(&mut self, idx: u32, obj: HeapObj) {
        self.slots[idx as usize].obj = Some(obj);
    }

    pub fn needs_gc(&self) -> bool {
        let alloc_limit = (self.live / 4).max(4096);
        self.live >= self.gc_threshold || self.alloc_count >= alloc_limit
    }

    pub fn usage(&self) -> usize { self.live }

    /* Object-count budget, reused as a byte cap for single oversized allocations. */
    pub fn limit(&self) -> usize { self.limit }

    #[inline(always)] pub fn get(&self, v: Val) -> &HeapObj {
        self.slots[v.as_heap() as usize].obj
            .as_ref()
            .expect("garbage collector invariant violated: live Val references a freed heap slot")
    }
    #[inline(always)] pub fn get_mut(&mut self, v: Val) -> &mut HeapObj {
        self.slots[v.as_heap() as usize].obj
            .as_mut()
            .expect("garbage collector invariant violated: live Val references a freed heap slot (mut)")
    }

    /* Panic-free access for type checks: None when `v` is not a live heap object. */
    #[inline] pub fn try_get(&self, v: Val) -> Option<&HeapObj> {
        if !v.is_heap() { return None; }
        self.slots.get(v.as_heap() as usize).and_then(|s| s.obj.as_ref())
    }
    #[inline] pub fn try_get_mut(&mut self, v: Val) -> Option<&mut HeapObj> {
        if !v.is_heap() { return None; }
        self.slots.get_mut(v.as_heap() as usize).and_then(|s| s.obj.as_mut())
    }


    /* Stable per-type tag for inline-cache binop specialisation; 0 for unknown/freed. */
    #[inline(always)]
    pub fn val_tag(&self, v: Val) -> u8 {
        if v.is_int() { 1 } else if v.is_float() { 2 } else if v.is_bool() { 3 }
        else if v.is_none() { 4 } else if v.is_heap() {
            match self.slots[v.as_heap() as usize].obj.as_ref() {
                Some(HeapObj::Str(_)) => 5,
                Some(HeapObj::List(_)) => 6,
                Some(HeapObj::Dict(_)) => 7,
                Some(HeapObj::Set(_)) => 8,
                Some(HeapObj::FrozenSet(_)) => 25,
                Some(HeapObj::Tuple(_)) => 9,
                Some(HeapObj::Func(_, _, _)) => 10,
                Some(HeapObj::Range(..)) => 11,
                Some(HeapObj::Slice(..)) => 12,
                Some(HeapObj::Type(_)) => 13,
                Some(HeapObj::LongInt(_)) => 14,
                Some(HeapObj::BoundMethod(_, _)) => 15,
                Some(HeapObj::NativeFn(_)) => 16,
                Some(HeapObj::BoundUserMethod(..)) => 17,
                Some(HeapObj::Class(..)) => 18,
                Some(HeapObj::Instance(..)) => 18,
                Some(HeapObj::Coroutine(..)) => 19,
                Some(HeapObj::Module(..)) => 20,
                Some(HeapObj::Extern(_)) => 21,
                Some(HeapObj::Bytes(_)) => 22,
                Some(HeapObj::ExcInstance(..)) => 24,
                Some(HeapObj::Ellipsis) => 26,
                Some(HeapObj::NotImplemented) => 27,
                Some(HeapObj::Super(..)) => 28,
                Some(HeapObj::Property(..)) => 29,
                Some(HeapObj::PropertySetter(..)) => 30,
                Some(HeapObj::StaticMethod(..)) => 31,
                Some(HeapObj::ClassMethod(..)) => 32,
                None => 0,
            }
        } else { 0 }
    }

    /* Identity probe for the `NotImplemented` singleton; consumed by the dunder dispatch protocol. */
    #[inline(always)]
    pub fn is_not_implemented(&self, v: Val) -> bool {
        v.is_heap()
            && matches!(self.slots[v.as_heap() as usize].obj.as_ref(), Some(HeapObj::NotImplemented))
    }

    /* `child` is `ancestor` or has it in its transitive bases. Identity on heap idx, classes are interned per-MakeClass and never mutated, so direct equality suffices. */
    pub fn is_subclass(&self, child: Val, ancestor: Val) -> bool {
        if child.0 == ancestor.0 { return true; }
        if !child.is_heap() { return false; }
        let HeapObj::Class(_, bases, _) = self.get(child) else { return false; };
        bases.iter().any(|&b| self.is_subclass(b, ancestor))
    }
}

/* Widens int/bool/LongInt to i128 for the slow path; None on non-integer operands. */
#[inline]
pub fn as_i128(v: Val, heap: &HeapPool) -> Option<i128> {
    if v.is_int() { Some(v.as_int() as i128) }
    else if v.is_bool() { Some(v.as_bool() as i128) }
    else if v.is_heap() {
        match heap.get(v) {
            HeapObj::LongInt(i) => Some(*i),
            _ => None,
        }
    }
    else { None }
}
