use crate::abi::{classify_decode, classify_encode, DecodeBits, EncodeRequest, ErrorKind, Op, PrimitiveBytes, TAG_INVALID};
use crate::modules::vm::types::{DictMap, HeapObj, Val, VmErr};
use crate::modules::vm::handlers::methods::{lookup_method, dispatch_method};
use crate::modules::packages::NativeBinding;
use alloc::{rc::Rc, string::{String, ToString}, sync::Arc, vec::Vec};
use core::cell::RefCell;
use crate::s;

use super::{get_val, host_call_native, in_vm, put_val, release_handles, safe_bytes, safe_handles, safe_str_owned, with_recv, with_runtime, with_vm};
use super::errors::{error_from_kind, stash_error};

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
            let chunk: &crate::modules::parser::SSAChunk = unsafe { &*(vm.chunk as *const _) };
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
fn wire_to_val(vm: &mut crate::modules::vm::VM, w: &crate::abi::WireValue) -> Result<Val, VmErr> {
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
fn val_to_wire(vm: &crate::modules::vm::VM, v: Val, depth: u32, seen: &mut Vec<u64>) -> Option<crate::abi::WireValue> {
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
            let guard = |vm: &crate::modules::vm::VM, seen: &mut Vec<u64>, items: &[Val]| -> Option<Vec<crate::abi::WireValue>> {
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
    with_runtime(|rt| rt.handles.release(h));
}

// Stash a guest error for the host. Overwrites any pending error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_throw(kind: u32, msg_ptr: *const u8, msg_len: u32) {
    let msg = unsafe { safe_str_owned(msg_ptr, msg_len) };
    with_runtime(|rt| rt.error_stash.set(kind, msg));
}

// Drain the most recent error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn host_edge_take_error(out_kind: *mut u32, dst: *mut u8, dst_max: u32) -> i32 {
    // Peek first so buffer-too-small callers can retry.
    let (kind, len) = match with_runtime(|rt| rt.error_stash.peek().map(|(k, m)| (k, m.len()))) {
        Some(p) => p,
        None => return -1,
    };
    if len > dst_max as usize { return -(len as i32); }
    // Buffer fits, drain and copy. None on `take()` means a lost peek/take race; return no-pending-error instead of panicking across FFI.
    let Some((_, msg)) = with_runtime(|rt| rt.error_stash.take()) else { return -1; };
    let bytes = msg.as_bytes();
    unsafe {
        *out_kind = kind;
        if !bytes.is_empty() {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
    }
    bytes.len() as i32
}

/* Builds a NativeBinding that marshals handles around `host_call_native`. Kept out of resolver.rs so the resolver stays ABI-agnostic. */
pub(super) fn make_native_binding(name: String, id: u32) -> NativeBinding {
    let closure = move |_: &mut crate::modules::vm::types::HeapPool, args: &[Val], kwargs: Option<Val>| -> Result<Val, VmErr> {
        /* 1. Register positional args as handles the guest will see; append the kwargs handle (0 means no kwargs). */
        let mut argv: Vec<u32> = args.iter().map(|v| put_val(*v)).collect();
        argv.push(kwargs.map_or(0, put_val));
        let mut out_handle: u32 = 0;

        // call_id is what call_extern will park with on defer; lets the host route the result back.
        let call_id = with_vm(|vm| vm.next_host_call_id as u32).unwrap_or(0);
        let status = unsafe {
            host_call_native(
                id, call_id,
                argv.as_ptr(), argv.len() as u32,
                &mut out_handle as *mut u32,
            )
        };

        /* 3. Read result BEFORE releasing argv: a returned input would point into slots we're about to free. */
        // Status 2 = DEFERRED: handler has captured what it needs; release argv and park the VM.
        if status == 2 {
            release_handles(&argv);
            return Err(VmErr::HostCallDeferred);
        }
        if status != 0 {
            release_handles(&argv);
            let (kind, msg) = with_runtime(|rt| rt.error_stash.take())
                .unwrap_or((ErrorKind::Runtime as u32, String::from("native call failed")));
            return Err(error_from_kind(kind, msg));
        }
        let result = get_val(out_handle).ok_or(VmErr::Runtime("native returned invalid handle"))?;
        argv.push(out_handle);
        release_handles(&argv);
        Ok(result)
    };
    NativeBinding { name, func: Arc::new(closure), pure: false }
}
