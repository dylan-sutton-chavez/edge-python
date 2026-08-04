// The FFI safety contract lives in docs/reference/wasm-abi.md, per-fn sections would duplicate it.
#![allow(clippy::missing_safety_doc)]

use crate::abi::{classify_decode, classify_encode, DecodeBits, EncodeRequest, ErrorKind, ErrorStash, HandleTable, Op, PrimitiveBytes, TAG_INVALID};
use crate::vm::types::{DictMap, HeapObj, Val, VmErr};
use crate::vm::methods::{lookup_method, dispatch_method};
use crate::vm::VM;
use alloc::{rc::Rc, string::{String, ToString}, vec::Vec};
use core::cell::RefCell;
use core::ptr::NonNull;
use crate::s;

/* All bridge state behind one accessor so `with_bridge` is the sole unsafe point. */
pub(crate) struct BridgeState {
    pub handles: HandleTable,
    pub error_stash: ErrorStash,
    /* Set and cleared by `VmGuard` or the paused-run stashes, deref only while its VM lives. */
    pub current_vm: Option<NonNull<VM<'static>>>,
}

static mut BRIDGE: BridgeState = BridgeState {
    handles: HandleTable::new(),
    error_stash: ErrorStash::new(),
    current_vm: None,
};

// SAFETY holds because WASM and the native engine are single-threaded, re-entry routes through `with_vm`.
pub(crate) fn with_bridge<R>(f: impl FnOnce(&mut BridgeState) -> R) -> R {
    unsafe { f(&mut *core::ptr::addr_of_mut!(BRIDGE)) }
}

pub fn put_val(v: Val) -> u32 { with_bridge(|b| b.handles.put(v.0)) }
pub fn get_val(h: u32) -> Option<Val> { with_bridge(|b| b.handles.get(h).map(Val)) }

// Release a batch of handles in one bridge borrow.
pub fn release_handles(handles: &[u32]) {
    with_bridge(|b| for &h in handles { b.handles.release(h); });
}

/* Drops handles, stash and VM pointer together, a paused run holding stale handles must go with them. */
#[cfg(target_arch = "wasm32")]
pub(crate) fn reset() {
    with_bridge(|b| {
        b.handles.clear();
        b.error_stash.clear();
        b.current_vm = None;
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn set_current_vm(ptr: Option<NonNull<VM<'static>>>) {
    with_bridge(|b| b.current_vm = ptr);
}

/* RAII publisher for the live VM pointer. Holding the guard across `run()` ensures a panic or early return cannot leave a stale pointer for later `host_edge_op` calls. */
pub struct VmGuard;

impl VmGuard {
    pub fn new(vm: &mut VM<'_>) -> Self {
        // 'static is storage-only, deref only inside the `run()` frame holding the guard.
        let ptr: NonNull<VM<'static>> = NonNull::from(vm).cast();
        with_bridge(|b| b.current_vm = Some(ptr));
        Self
    }
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        with_bridge(|b| b.current_vm = None);
    }
}

pub(crate) fn with_vm<R>(f: impl FnOnce(&mut VM<'static>) -> R) -> Option<R> {
    // Drop the bridge borrow before `f`, VM dispatch re-enters `with_bridge`.
    let ptr = with_bridge(|b| b.current_vm)?;
    Some(f(unsafe { &mut *ptr.as_ptr() }))
}

/* Builds a `&[u8]` from an FFI `(ptr, len)`, empty on null or zero length, `from_raw_parts` would UB on either. */
pub(crate) unsafe fn safe_bytes<'a>(ptr: *const u8, len: u32) -> &'a [u8] {
    if ptr.is_null() || len == 0 { return &[]; }
    unsafe { core::slice::from_raw_parts(ptr, len as usize) }
}

/* Same for `&[u32]` argv arrays. */
pub(crate) unsafe fn safe_handles<'a>(ptr: *const u32, len: u32) -> &'a [u32] {
    if ptr.is_null() || len == 0 { return &[]; }
    unsafe { core::slice::from_raw_parts(ptr, len as usize) }
}

/* Owned UTF-8 string from an FFI `(ptr, len)`; empty on null or invalid UTF-8. */
pub(crate) unsafe fn safe_str_owned(ptr: *const u8, len: u32) -> String {
    core::str::from_utf8(unsafe { safe_bytes(ptr, len) }).unwrap_or("").to_string()
}

/* `with_vm` that errors when called outside run(). */
pub(crate) fn in_vm(err: &'static str, f: impl FnOnce(&mut VM<'static>) -> Result<Val, VmErr>) -> Result<Val, VmErr> {
    with_vm(f).ok_or(VmErr::Runtime(err))?
}

/* `dispatch_*` prologue: resolve `recv_h` and run `f` against the live VM. Fails on stale handle or call outside `run()`. */
pub(crate) fn with_recv<F>(invalid_recv_msg: &'static str, recv_h: u32, f: F) -> Result<Val, VmErr>
where F: FnOnce(&mut VM<'static>, Val) -> Result<Val, VmErr>
{ let recv = get_val(recv_h).ok_or(VmErr::Runtime(invalid_recv_msg))?; with_vm(|vm| f(vm, recv)).ok_or(VmErr::Runtime("edge_op called outside run()"))? }

/* VmErr classifier for the ABI boundary. */
pub(crate) fn err_to_kind(e: &VmErr) -> ErrorKind {
    match e {
        VmErr::Type(_) | VmErr::TypeMsg(_) => ErrorKind::Type,
        VmErr::Value(_) => ErrorKind::Value,
        VmErr::Runtime(_) => ErrorKind::Runtime,
        VmErr::Attribute(_) | VmErr::Name(_) => ErrorKind::Attribute,
        VmErr::Raised(s) => {
            if s.starts_with("ValueError") { ErrorKind::Value }
            else if s.starts_with("IndexError") { ErrorKind::Index }
            else if s.starts_with("KeyError") { ErrorKind::Key }
            else { ErrorKind::Runtime }
        }
        _ => ErrorKind::Runtime,
    }
}

pub(crate) fn stash_error(e: VmErr) {
    let kind = err_to_kind(&e);
    let msg = e.render();
    with_bridge(|b| b.error_stash.set_typed(kind, msg));
}

/* Message-only stash for contexts without a `VmErr`, e.g. the WASM panic handler. */
#[cfg(target_arch = "wasm32")]
pub(crate) fn stash_raw_error(kind: u32, msg: String) {
    with_bridge(|b| b.error_stash.set(kind, msg));
}

pub fn take_error() -> Option<(u32, String)> {
    with_bridge(|b| b.error_stash.take())
}

/* Inverse of `err_to_kind`: rebuilds a `VmErr` from (kind, msg). Exhaustive over `ErrorKind` so new variants can't slip into `Raised`. */
pub fn error_from_kind(kind: u32, msg: String) -> VmErr {
    match ErrorKind::from_u32(kind) {
        Some(ErrorKind::Type) => VmErr::TypeMsg(msg),
        Some(ErrorKind::Value) => VmErr::Raised(s!("ValueError: ", str &msg)),
        Some(ErrorKind::Runtime) => VmErr::Raised(s!("RuntimeError: ", str &msg)),
        Some(ErrorKind::Attribute) => VmErr::Attribute(msg),
        Some(ErrorKind::Index) => VmErr::Raised(s!("IndexError: ", str &msg)),
        Some(ErrorKind::Key) => VmErr::Raised(s!("KeyError: ", str &msg)),
        // Custom kinds carry the user-defined class name in `msg` (`<ClassName>: <text>`); pass through unchanged.
        Some(ErrorKind::Custom) | None => VmErr::Raised(msg),
    }
}

// Universal dispatch. Returns 0 + handle in `*out_handle`, or 1 + stashed error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_op(op: u32, recv: u32, name_ptr: *const u8, name_len: u32,argv_ptr: *const u32, argc: u32, out_handle: *mut u32) -> i32 {
    let name = unsafe { safe_str_owned(name_ptr, name_len) };
    let args: Vec<Val> = unsafe { safe_handles(argv_ptr, argc) }.iter().filter_map(|&h| get_val(h)).collect();

    let result: Result<Val, VmErr> = match Op::from_u32(op) {
        Some(Op::Call) => dispatch_call(recv, &name, &args),
        Some(Op::GetAttr) => dispatch_get_attr(recv, &name),
        Some(Op::SetAttr) => dispatch_set_attr(recv, &name, &args),
        Some(Op::GetItem) => dispatch_get_item(recv, &args),
        Some(Op::SetItem) => dispatch_set_item(recv, &args),
        Some(Op::Len) => dispatch_len(recv),
        Some(Op::Iter) => dispatch_iter(recv),
        Some(Op::IterNext) => dispatch_iter_next(recv),
        Some(Op::NewDict) => in_vm("edge_op new_dict called outside run()", |vm| vm.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(DictMap::new()))))),
        Some(Op::NewList) => in_vm("edge_op new_list called outside run()", |vm| vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(Vec::new()))))),
        Some(Op::TypeOf) => dispatch_type_of(recv),
        Some(Op::NewTuple) => in_vm("edge_op new_tuple called outside run()", |vm| vm.tuple_from_items(args.to_vec())),
        Some(Op::NewSet) => in_vm("edge_op new_set called outside run()", |vm| vm.set_from_items(args.to_vec())),
        Some(Op::NewFrozenSet) => in_vm("edge_op new_frozenset called outside run()", |vm| vm.frozenset_from_items(args.to_vec())),
        None => Err(VmErr::Raised(s!("edge_op: unsupported op ", int op as i64))),
    };

    match result {
        Ok(v) => { unsafe { *out_handle = put_val(v); } 0 }
        Err(e) => { stash_error(e); 1 }
    }
}

fn dispatch_call(recv_h: u32, name: &str, args: &[Val]) -> Result<Val, VmErr> {
    with_recv("edge_op call: invalid receiver handle", recv_h, |vm, recv| {
        // `__call__` means "invoke `recv` as a callable", letting plugins forward arbitrary Python hooks (lambdas, builtins, classes) through `Handle::call("__call__", args)`. Pushes args + callee then drives `exec_call` so every callable kind (`Extern`, `NativeFn`, `Func`, `BoundMethod`, `Class`, ...) routes through the same dispatch path the VM uses normally. Empty caller-slots are fine because lambdas/hooks that escape a plugin call cannot reference caller-frame locals, they can still capture their own defining scope through the regular Func captures vector.
        if name == "__call__" {
            // Stack layout for `Call`: callee at the bottom, then positional args (top is the rightmost). `parse_call_args` pops args first, then `exec_call` pops the callee.
            let stack_before = vm.stack.len();
            vm.stack.push(recv);
            for a in args { vm.stack.push(*a); }
            let operand = args.len() as u16; // (num_kw<<8)|num_pos; no kwargs from FFI hooks.
            let chunk: &crate::parser::SSAChunk = unsafe { &*(vm.chunk as *const _) };
            let mut empty_slots: [Val; 0] = [];
            vm.exec_call(operand, chunk, &mut empty_slots)?;
            if vm.stack.len() != stack_before + 1 {
                return Err(VmErr::Runtime("edge_op call(__call__): callable left no result"));
            }
            return vm.stack.pop().ok_or(VmErr::Runtime("edge_op call(__call__): stack drained"));
        }
        let ty = vm.type_name(recv);
        let mid = lookup_method(ty, name).ok_or_else(|| VmErr::Attribute(s!("'", str ty, "' object has no method '", str name, "'")))?;
        let stack_before = vm.stack.len();
        dispatch_method(vm, mid, recv, args, &[])?;
        if vm.stack.len() != stack_before + 1 {
            return Err(VmErr::Runtime("edge_op call: method left no result"));
        }
        // The length check above guarantees a value is present; `ok_or` keeps the FFI boundary panic-free if a future change drops the invariant.
        vm.stack.pop().ok_or(VmErr::Runtime("edge_op call: stack drained mid-dispatch"))
    })
}

/* GetAttr: module/instance attr, or bind builtin method as BoundMethod. */
fn dispatch_get_attr(recv_h: u32, name: &str) -> Result<Val, VmErr> {
    with_recv("edge_op get_attr: invalid receiver handle", recv_h, |vm, recv| {
        // Module attribute.
        if recv.is_heap() && let HeapObj::Module(_, attrs) = vm.heap.get(recv)
        {
            let bare = name;
            if let Some((_, v)) = attrs.iter().find(|(n, _)| n == bare) {
                return Ok(*v);
            }
            return Err(VmErr::Attribute(s!("module has no attribute '", str name, "'")));
        }
        // Instance attribute.
        if recv.is_heap() && let HeapObj::Instance(_cls, attrs) = vm.heap.get(recv)
        {
            let entries = attrs.borrow().entries.clone();
            for (k, v) in &entries {
                if k.is_heap()
                    && let HeapObj::Str(s) = vm.heap.get(*k)
                    && s == name
                {
                    return Ok(*v);
                }
            }
            return Err(VmErr::Attribute(s!("instance has no attribute '", str name, "'")));
        }
        // Builtin method -> BoundMethod.
        let ty = vm.type_name(recv);
        if let Some(mid) = lookup_method(ty, name) {
            return vm.heap.alloc(HeapObj::BoundMethod(recv, mid));
        }
        Err(VmErr::Attribute(s!("'", str ty, "' object has no attribute '", str name, "'")))
    })
}

/* SetAttr: writes to instance `__dict__`; rejects modules and builtins. */
fn dispatch_set_attr(recv_h: u32, name: &str, args: &[Val]) -> Result<Val, VmErr> {
    if args.len() != 1 {
        return Err(VmErr::TypeMsg(s!("set_attr expects exactly 1 value, got ", int args.len() as i64)));
    }
    let value = args[0];
    with_recv("edge_op set_attr: invalid receiver handle", recv_h, |vm, recv| {
        if !recv.is_heap() {
            return Err(VmErr::Type("cannot set attribute on this type"));
        }
        if let HeapObj::Instance(_cls, attrs) = vm.heap.get(recv) {
            let attrs = attrs.clone();
            let key = vm.heap.alloc(HeapObj::Str(name.to_string()))?;
            attrs.borrow_mut().insert(key, value, &vm.heap);
            return Ok(Val::none());
        }
        Err(VmErr::Type("cannot set attribute on this type"))
    })
}

/* GetItem: built-in indexing only, FFI has no bytecode frame to drive instance `__getitem__` dispatch. */
fn dispatch_get_item(recv_h: u32, args: &[Val]) -> Result<Val, VmErr> {
    if args.len() != 1 {
        return Err(VmErr::TypeMsg(s!("get_item expects 1 index, got ", int args.len() as i64)));
    }
    let idx = args[0];
    with_recv("edge_op get_item: invalid receiver handle", recv_h, |vm, recv| {
        let stack_before = vm.stack.len();
        let _ = vm.get_item_builtin(recv, idx)?; // Discard the bool (slice-path indicator).
        if vm.stack.len() != stack_before + 1 {
            return Err(VmErr::Runtime("edge_op get_item: get_item left no result"));
        }
        vm.stack.pop().ok_or(VmErr::Runtime("edge_op get_item: stack drained mid-dispatch"))
    })
}

/* SetItem: built-in item-assignment only, same rationale as `dispatch_get_item`. */
fn dispatch_set_item(recv_h: u32, args: &[Val]) -> Result<Val, VmErr> {
    if args.len() != 2 {
        return Err(VmErr::TypeMsg(s!("set_item expects (index, value), got ", int args.len() as i64, " args")));
    }
    let idx = args[0];
    let value = args[1];
    with_recv("edge_op set_item: invalid receiver handle", recv_h, |vm, recv| {
        vm.store_item_builtin(recv, idx, value)?;
        Ok(Val::none())
    })
}

fn dispatch_len(recv_h: u32) -> Result<Val, VmErr> {
    with_recv("edge_op len: invalid receiver handle", recv_h, |vm, recv| {
        let n: i64 = match vm.heap.get(recv) {
            HeapObj::Str(s) => s.chars().count() as i64,
            HeapObj::List(rc) => rc.borrow().len() as i64,
            HeapObj::Dict(rc) => rc.borrow().entries.len() as i64,
            HeapObj::Set(rc) => rc.borrow().len() as i64,
            HeapObj::Tuple(t) => t.len() as i64,
            _ => return Err(VmErr::TypeMsg(s!("object of type '", str vm.type_name(recv), "' has no len()"))),
        };
        Ok(Val::int(n))
    })
}

/* Iter: flatten any iterable into a List for guest GetItem/Len access. */
fn dispatch_iter(recv_h: u32) -> Result<Val, VmErr> {
    with_recv("edge_op iter: invalid receiver handle", recv_h, |vm, recv| {
        let items: Vec<Val> = match vm.heap.get(recv) {
            HeapObj::List(rc) => rc.borrow().clone(),
            HeapObj::Tuple(t) => t.clone(),
            HeapObj::Set(rc) => rc.borrow().iter().copied().collect(),
            HeapObj::Dict(rc) => rc.borrow().keys().collect(),
            HeapObj::Range(s, e, st) => {
                let mut out = Vec::new();
                let (mut cur, end, step) = (*s, *e, *st);
                if step > 0 {
                    while cur < end { out.push(Val::int(cur)); cur += step; }
                } else if step < 0 {
                    while cur > end { out.push(Val::int(cur)); cur += step; }
                }
                out
            }
            HeapObj::Str(s) => {
                let chars: Vec<String> = s.chars().map(|c| c.to_string()).collect();
                chars.into_iter()
                    .map(|cs| vm.heap.alloc(HeapObj::Str(cs)))
                    .collect::<Result<Vec<_>, _>>()?
            }
            _ => return Err(VmErr::TypeMsg(s!("object of type '", str vm.type_name(recv), "' is not iterable"))),
        };
        vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))
    })
}

/* IterNext: pops list head; raises StopIteration when empty. */
fn dispatch_iter_next(recv_h: u32) -> Result<Val, VmErr> {
    with_recv("edge_op iter_next: invalid receiver handle", recv_h, |vm, recv| {
        if let HeapObj::List(rc) = vm.heap.get(recv) {
            let mut v = rc.borrow_mut();
            if v.is_empty() {
                return Err(VmErr::Raised(s!("StopIteration")));
            }
            Ok(v.remove(0))
        } else {
            Err(VmErr::TypeMsg(s!("iter_next expects a List iterator (produced by Op::Iter), got '", str vm.type_name(recv), "'")))
        }
    })
}

fn dispatch_type_of(recv_h: u32) -> Result<Val, VmErr> {
    with_recv("edge_op type_of: invalid receiver handle", recv_h, |vm, recv| {
        let name = vm.type_name(recv).to_string();
        vm.heap.alloc(HeapObj::Str(name))
    })
}

// Bootstrap encoder: classifies (tag, bytes) into a Val handle; returns 0 on Invalid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_encode(tag: u32, ptr: *const u8, len: u32) -> u32 {
    // Alloc in the live VM; 0 when outside run() or OOM.
    fn alloc_and_put(obj: HeapObj) -> u32 {
        match with_vm(|vm| vm.heap.alloc(obj).ok()).flatten() {
            Some(val) => put_val(val),
            None => 0,
        }
    }
    let bytes = unsafe { safe_bytes(ptr, len) };
    match classify_encode(tag, bytes) {
        EncodeRequest::Direct(bits) => put_val(Val(bits)),
        EncodeRequest::AllocStr(s) => alloc_and_put(HeapObj::Str(s.to_string())),
        EncodeRequest::AllocBytes(b) => alloc_and_put(HeapObj::Bytes(b.to_vec())),
        EncodeRequest::AllocLongInt(i) => alloc_and_put(HeapObj::LongInt(i)),
        EncodeRequest::Composite(w) => {
            match with_vm(|vm| wire_to_val(vm, &w).ok()).flatten() {
                Some(val) => put_val(val),
                None => 0,
            }
        }
        EncodeRequest::Invalid => 0,
    }
}

/* Wire tree to heap value. Str keys only for dicts; mirrors `classify_encode` inline-int split. */
fn wire_to_val(vm: &mut crate::vm::VM, w: &crate::abi::WireValue) -> Result<Val, VmErr> {
    use crate::abi::WireValue;
    Ok(match w {
        WireValue::None => Val::none(),
        WireValue::Bool(b) => Val::bool(*b),
        WireValue::Int(i) => match crate::abi::inline_int_bits(*i) {
            Some(bits) => Val(bits),
            None => vm.heap.alloc(HeapObj::LongInt(*i))?,
        },
        WireValue::Float(f) => Val::float(*f),
        WireValue::Bytes(b) => {
            let s = core::str::from_utf8(b).map_err(|_| VmErr::Value("invalid UTF-8 in wire str"))?;
            vm.heap.alloc(HeapObj::Str(s.to_string()))?
        }
        WireValue::Raw(b) => vm.heap.alloc(HeapObj::Bytes(b.clone()))?,
        WireValue::List(items) => {
            let vals = items.iter().map(|it| wire_to_val(vm, it)).collect::<Result<Vec<_>, _>>()?;
            vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(vals))))?
        }
        WireValue::Dict(pairs) => {
            let mut map = DictMap::new();
            for (k, v) in pairs {
                let WireValue::Bytes(kb) = k else { return Err(VmErr::Type("wire dict keys must be str")); };
                let ks = core::str::from_utf8(kb).map_err(|_| VmErr::Value("invalid UTF-8 in wire str"))?;
                let key = vm.heap.alloc(HeapObj::Str(ks.to_string()))?;
                let val = wire_to_val(vm, v)?;
                map.insert(key, val, &vm.heap);
            }
            vm.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(map))))?
        }
    })
}

/* Heap value to wire tree. `None` on cycles, depth past the cap, or non-transit members. */
fn val_to_wire(vm: &crate::vm::VM, v: Val, depth: u32, seen: &mut Vec<u64>) -> Option<crate::abi::WireValue> {
    use crate::abi::{DecodeBits as DB, PrimitiveBytes as PB, WireValue, MAX_WIRE_DEPTH};
    if depth > MAX_WIRE_DEPTH { return None; }
    match classify_decode(v.0) {
        DB::Primitive { tag, bytes } => {
            let buf: Vec<u8> = match bytes {
                PB::None => Vec::new(),
                PB::Bool(b) => alloc::vec![b],
                PB::Eight(a) => a.to_vec(),
                PB::Sixteen(a) => a.to_vec(),
            };
            WireValue::decode_body(tag, &buf)
        }
        DB::Heap => {
            // Shared references are fine; re-entering a value mid-walk is a cycle.
            let guard = |vm: &crate::vm::VM, seen: &mut Vec<u64>, items: &[Val]| -> Option<Vec<crate::abi::WireValue>> {
                items.iter().map(|it| val_to_wire(vm, *it, depth + 1, seen)).collect()
            };
            if seen.contains(&v.0) { return None; }
            match vm.heap.get(v) {
                HeapObj::Str(s) => Some(WireValue::Bytes(s.as_bytes().to_vec())),
                HeapObj::Bytes(b) => Some(WireValue::Raw(b.clone())),
                HeapObj::LongInt(i) => Some(WireValue::Int(*i)),
                HeapObj::List(rc) => {
                    seen.push(v.0);
                    let items = guard(vm, seen, &rc.borrow())?;
                    seen.pop();
                    Some(WireValue::List(items))
                }
                HeapObj::Tuple(t) => {
                    seen.push(v.0);
                    let items = guard(vm, seen, t)?;
                    seen.pop();
                    Some(WireValue::List(items))
                }
                HeapObj::Dict(rc) => {
                    seen.push(v.0);
                    let mut pairs = Vec::new();
                    for (k, val) in rc.borrow().entries.iter() {
                        let key = match val_to_wire(vm, *k, depth + 1, seen)? {
                            key @ WireValue::Bytes(_) => key,
                            _ => return None,
                        };
                        pairs.push((key, val_to_wire(vm, *val, depth + 1, seen)?));
                    }
                    seen.pop();
                    Some(WireValue::Dict(pairs))
                }
                _ => None,
            }
        }
        DB::Invalid => None,
    }
}

// Bootstrap decoder: writes tag to `*out_tag`, bytes to `dst[..dst_max]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_decode(h: u32, out_tag: *mut u32, dst: *mut u8, dst_max: u32) -> i32 {
    let copy_into = |tag: u32, bytes: &[u8]| -> i32 {
        unsafe { *out_tag = tag; }
        if bytes.len() > dst_max as usize { return -(bytes.len() as i32); }
        if !bytes.is_empty() {
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            }
        }
        bytes.len() as i32
    };

    let v = match get_val(h) {
        Some(v) => v,
        None => { unsafe { *out_tag = TAG_INVALID; } return 0; }
    };

    match classify_decode(v.0) {
        DecodeBits::Primitive { tag, bytes } => match bytes {
            PrimitiveBytes::None => copy_into(tag, &[]),
            PrimitiveBytes::Bool(b) => copy_into(tag, &[b]),
            PrimitiveBytes::Eight(a) => copy_into(tag, &a),
            PrimitiveBytes::Sixteen(a) => copy_into(tag, &a),
        },
        DecodeBits::Heap => {
            // Str, Bytes, LongInt and TLV composites decode; sets, instances and other non-transit values go through `edge_op`.
            enum Decoded { Str(alloc::string::String), Bytes(Vec<u8>), LongInt(i128), Wire(u32, Vec<u8>), Other }
            let decoded = with_vm(|vm| match vm.heap.get(v) {
                HeapObj::Str(s) => Decoded::Str(s.clone()),
                HeapObj::Bytes(b) => Decoded::Bytes(b.clone()),
                HeapObj::LongInt(i) => Decoded::LongInt(*i),
                HeapObj::List(_) | HeapObj::Tuple(_) | HeapObj::Dict(_) => {
                    match val_to_wire(vm, v, 0, &mut Vec::new()) {
                        Some(w) => {
                            let mut buf = Vec::new();
                            w.encode_body(&mut buf);
                            Decoded::Wire(w.tag(), buf)
                        }
                        None => Decoded::Other,
                    }
                }
                _ => Decoded::Other,
            }).unwrap_or(Decoded::Other);
            match decoded {
                Decoded::Str(s) => copy_into(crate::abi::Tag::Bytes as u32, s.as_bytes()),
                Decoded::Bytes(b) => copy_into(crate::abi::Tag::Raw as u32, &b),
                Decoded::LongInt(i) => copy_into(crate::abi::Tag::Int as u32, &i.to_le_bytes()),
                Decoded::Wire(tag, buf) => copy_into(tag, &buf),
                Decoded::Other => { unsafe { *out_tag = TAG_INVALID; } 0 }
            }
        }
        DecodeBits::Invalid => { unsafe { *out_tag = TAG_INVALID; } 0 }
    }
}

// Decrement refcount on a handle. No-op for invalid handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_release(h: u32) {
    with_bridge(|b| b.handles.release(h));
}

// Stash a guest error for the host. Overwrites any pending error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_throw(kind: u32, msg_ptr: *const u8, msg_len: u32) {
    let msg = unsafe { safe_str_owned(msg_ptr, msg_len) };
    with_bridge(|b| b.error_stash.set(kind, msg));
}

// Drain the most recent error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_take_error(out_kind: *mut u32, dst: *mut u8, dst_max: u32) -> i32 {
    // Peek first so buffer-too-small callers can retry.
    let (kind, len) = match with_bridge(|b| b.error_stash.peek().map(|(k, m)| (k, m.len()))) {
        Some(p) => p,
        None => return -1,
    };
    if len > dst_max as usize { return -(len as i32); }
    // Buffer fits, drain and copy. None on `take()` means a lost peek/take race; return no-pending-error instead of panicking across FFI.
    let Some((_, msg)) = with_bridge(|b| b.error_stash.take()) else { return -1; };
    let bytes = msg.as_bytes();
    unsafe {
        *out_kind = kind;
        if !bytes.is_empty() {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
    }
    bytes.len() as i32
}
