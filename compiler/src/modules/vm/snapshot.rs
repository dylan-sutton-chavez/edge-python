use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::hash::Hasher;

use crate::modules::parser::types::{ImportKind, SSAChunk};
use crate::util::fx::{FxHashMap, FxHasher};
use super::{Pending, VM};
use super::types::*;

const MAGIC: u32 = 0x4E53_5045;
const FORMAT: u32 = 1;

pub type SnapErr = String;

type ExternMap = FxHashMap<String, ExternFn>;

fn s_err(what: &str, detail: &str) -> SnapErr {
    crate::s!("snapshot: ", str what, " '", str detail, "'")
}

struct W {
    b: Vec<u8>,
}

impl W {
    fn new() -> Self { Self { b: Vec::with_capacity(4096) } }
    fn u8(&mut self, v: u8) { self.b.push(v); }
    fn u16(&mut self, v: u16) { self.b.extend_from_slice(&v.to_le_bytes()); }
    fn u32(&mut self, v: u32) { self.b.extend_from_slice(&v.to_le_bytes()); }
    fn u64(&mut self, v: u64) { self.b.extend_from_slice(&v.to_le_bytes()); }
    fn i64(&mut self, v: i64) { self.u64(v as u64); }
    fn i32v(&mut self, v: i32) { self.u32(v as u32); }
    fn i128v(&mut self, v: i128) { self.b.extend_from_slice(&v.to_le_bytes()); }
    fn usz(&mut self, v: usize) { self.u64(v as u64); }
    fn boolean(&mut self, v: bool) { self.u8(v as u8); }
    fn bytes(&mut self, v: &[u8]) { self.usz(v.len()); self.b.extend_from_slice(v); }
    fn str(&mut self, v: &str) { self.bytes(v.as_bytes()); }
    fn val(&mut self, v: Val) { self.u64(v.0); }
    fn seq<T>(&mut self, items: &[T], f: impl Fn(&mut Self, &T)) {
        self.usz(items.len());
        for x in items { f(self, x); }
    }
    fn vals(&mut self, v: &[Val]) { self.seq(v, |w, x| w.val(*x)); }
    fn opt<T>(&mut self, v: Option<T>, f: impl Fn(&mut Self, T)) {
        match v { Some(x) => { self.u8(1); f(self, x); } None => self.u8(0) }
    }
    fn opt_val(&mut self, v: Option<Val>) { self.opt(v, Self::val); }
    fn opt_u32(&mut self, v: Option<u32>) { self.opt(v, Self::u32); }
    fn opt_u64(&mut self, v: Option<u64>) { self.opt(v, Self::u64); }
    fn opt_usz(&mut self, v: Option<usize>) { self.opt(v, Self::usz); }
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> Self { Self { b, p: 0 } }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SnapErr> {
        let end = self.p.checked_add(n).ok_or_else(|| "snapshot truncated".to_string())?;
        if end > self.b.len() { return Err("snapshot truncated".to_string()); }
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SnapErr> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, SnapErr> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, SnapErr> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, SnapErr> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64, SnapErr> { Ok(self.u64()? as i64) }
    fn i32v(&mut self) -> Result<i32, SnapErr> { Ok(self.u32()? as i32) }
    fn i128v(&mut self) -> Result<i128, SnapErr> { Ok(i128::from_le_bytes(self.take(16)?.try_into().unwrap())) }
    fn usz(&mut self) -> Result<usize, SnapErr> {
        usize::try_from(self.u64()?).map_err(|_| "snapshot value out of range".to_string())
    }
    /* Rejects counts beyond the remaining payload. */
    fn count(&mut self) -> Result<usize, SnapErr> {
        let n = self.usz()?;
        if n > self.b.len() - self.p { return Err("snapshot truncated".to_string()); }
        Ok(n)
    }
    fn boolean(&mut self) -> Result<bool, SnapErr> { Ok(self.u8()? != 0) }
    fn bytes(&mut self) -> Result<Vec<u8>, SnapErr> {
        let n = self.count()?;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Result<String, SnapErr> {
        String::from_utf8(self.bytes()?).map_err(|_| "snapshot string not utf-8".to_string())
    }
    /* Static-str decode leaks; stored errors are rare. */
    fn leakstr(&mut self) -> Result<&'static str, SnapErr> {
        Ok(alloc::boxed::Box::leak(self.str()?.into_boxed_str()))
    }
    fn val(&mut self) -> Result<Val, SnapErr> { Ok(Val(self.u64()?)) }
    fn seq<T>(&mut self, f: impl Fn(&mut Self) -> Result<T, SnapErr>) -> Result<Vec<T>, SnapErr> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n { v.push(f(self)?); }
        Ok(v)
    }
    fn vals(&mut self) -> Result<Vec<Val>, SnapErr> { self.seq(Self::val) }
    fn opt<T>(&mut self, f: impl Fn(&mut Self) -> Result<T, SnapErr>) -> Result<Option<T>, SnapErr> {
        Ok(if self.u8()? == 1 { Some(f(self)?) } else { None })
    }
    fn opt_val(&mut self) -> Result<Option<Val>, SnapErr> { self.opt(Self::val) }
    fn opt_u32(&mut self) -> Result<Option<u32>, SnapErr> { self.opt(Self::u32) }
    fn opt_u64(&mut self) -> Result<Option<u64>, SnapErr> { self.opt(Self::u64) }
    fn opt_usz(&mut self) -> Result<Option<usize>, SnapErr> { self.opt(Self::usz) }
}

/* One declaration emits both codec sides. */
macro_rules! codec {
    (struct $T:ident, $put:ident, $get:ident { $($f:ident: $s:tt),* $(,)? }) => {
        fn $put(w: &mut W, x: &$T) { $( codec!(@put w, (&x.$f), $s); )* }
        fn $get(r: &mut R) -> Result<$T, SnapErr> { Ok($T { $($f: codec!(@get r, $s)),* }) }
    };
    (enum $T:ident, $put:ident, $get:ident {
        $($tag:literal $V:ident $({ $($sf:ident: $ss:tt),* })? $(( $($tf:ident: $ts:tt),* ))?),* $(,)?
    }) => {
        fn $put(w: &mut W, x: &$T) {
            match x {
                $( $T::$V $({ $($sf),* })? $(( $($tf),* ))? => {
                    w.u8($tag);
                    $($( codec!(@put w, ($sf), $ss); )*)?
                    $($( codec!(@put w, ($tf), $ts); )*)?
                } )*
            }
        }
        fn $get(r: &mut R) -> Result<$T, SnapErr> {
            Ok(match r.u8()? {
                $( $tag => $T::$V $({ $($sf: codec!(@get r, $ss)),* })? $(( $(codec!(@get r, $ts)),* ))?, )*
                t => return Err(s_err("unknown tag", itoa::Buffer::new().format(t))),
            })
        }
    };
    (@put $w:ident, ($x:expr), str) => { $w.str($x) };
    (@put $w:ident, ($x:expr), vals) => { $w.vals($x) };
    (@put $w:ident, ($x:expr), leakstr) => { $w.str($x) };
    (@put $w:ident, ($x:expr), default) => { let _ = $x; };
    (@put $w:ident, ($x:expr), [str]) => { $w.seq($x, |w, v| w.str(v)) };
    (@put $w:ident, ($x:expr), [$m:ident]) => { $w.seq($x, |w, v| w.$m(*v)) };
    (@put $w:ident, ($x:expr), [$p:ident, $g:ident]) => { $w.seq($x, $p) };
    (@put $w:ident, ($x:expr), ($p:ident, $g:ident)) => { $p($w, $x) };
    (@put $w:ident, ($x:expr), $m:ident) => { $w.$m(*$x) };
    (@get $r:ident, str) => { $r.str()? };
    (@get $r:ident, vals) => { $r.vals()? };
    (@get $r:ident, leakstr) => { $r.leakstr()? };
    (@get $r:ident, default) => { Default::default() };
    (@get $r:ident, [str]) => { $r.seq(|r| r.str())? };
    (@get $r:ident, [$m:ident]) => { $r.seq(|r| r.$m())? };
    (@get $r:ident, [$p:ident, $g:ident]) => { $r.seq($g)? };
    (@get $r:ident, ($p:ident, $g:ident)) => { $g($r)? };
    (@get $r:ident, $m:ident) => { $r.$m()? };
}

fn put_val_pair(w: &mut W, p: &(Val, Val)) { w.val(p.0); w.val(p.1); }
fn get_val_pair(r: &mut R) -> Result<(Val, Val), SnapErr> { Ok((r.val()?, r.val()?)) }
fn put_slot_val(w: &mut W, p: &(usize, Val)) { w.usz(p.0); w.val(p.1); }
fn get_slot_val(r: &mut R) -> Result<(usize, Val), SnapErr> { Ok((r.usz()?, r.val()?)) }
fn put_name_val(w: &mut W, p: &(String, Val)) { w.str(&p.0); w.val(p.1); }
fn get_name_val(r: &mut R) -> Result<(String, Val), SnapErr> { Ok((r.str()?, r.val()?)) }
fn put_i32_pair(w: &mut W, p: &(i32, i32)) { w.i32v(p.0); w.i32v(p.1); }
fn get_i32_pair(r: &mut R) -> Result<(i32, i32), SnapErr> { Ok((r.i32v()?, r.i32v()?)) }

codec!(enum BlockKind, put_block_kind, get_block_kind { 0 Except, 1 Finally });

codec!(enum BodyRef, put_body_ref, get_body_ref { 0 Fn(fi: usz), 1 Module });

codec!(enum IterFrame, put_iter_frame, get_iter_frame {
    0 Seq { items: vals, idx: usz },
    1 Range { cur: i64, end: i64, step: i64 },
    2 Coroutine(v: val),
    3 UserDefined(v: val),
});

codec!(struct ExceptionFrame, put_exc_frame, get_exc_frame {
    kind: (put_block_kind, get_block_kind),
    handler_ip: usz,
    stack_depth: usz,
    iter_depth: usz,
    with_depth: usz,
    unwind_depth: usz,
});

codec!(struct SyncFrame, put_sync_frame, get_sync_frame {
    ip: usz,
    fi: usz,
    slots: vals,
    stack_delta: vals,
    iter_delta: [put_iter_frame, get_iter_frame],
    exception_delta: [put_exc_frame, get_exc_frame],
});

codec!(enum SchedulerStatus, put_sched, get_sched {
    0 Done,
    1 PendingTimer(d: u64),
    2 PendingFrame,
    3 PendingEvent,
    4 PendingHostCall,
    5 Preempted,
});

codec!(enum VmErr, put_vm_err, get_vm_err {
    0 CallDepth,
    1 Heap,
    2 Budget,
    3 ZeroDiv,
    4 Overflow,
    5 Name(s: str),
    6 Type(s: leakstr),
    7 TypeMsg(s: str),
    8 Value(s: leakstr),
    9 Runtime(s: leakstr),
    10 Attribute(s: str),
    11 Raised(s: str),
    12 HostYield(st: (put_sched, get_sched)),
    13 HostCallDeferred,
});

codec!(enum Unwind, put_unwind, get_unwind {
    0 Normal,
    1 Return(v: val),
    2 Goto { target: usz, remaining: u16 },
    3 Reraise(e: (put_vm_err, get_vm_err)),
});

codec!(enum WaitKind, put_wait_kind, get_wait_kind {
    0 Run(v: val),
    1 Gather,
    2 Timeout { deadline_ns: u64, target: val },
});

codec!(enum CoroState, put_coro_state, get_coro_state {
    0 Ready,
    1 Sleeping(d: u64),
    2 WaitingFrame,
    3 WaitingEvent,
    4 WaitingHostCall(id: u64),
    5 WaitingForChildren { tasks: vals, kind: (put_wait_kind, get_wait_kind) },
    6 CancelPending,
    7 Done(v: val),
    8 Errored(e: (put_vm_err, get_vm_err)),
    9 Cancelled,
});

codec!(struct CoroutineHandle, put_handle, get_handle {
    coro: val,
    state: (put_coro_state, get_coro_state),
});

fn put_wfc(w: &mut W, v: &Option<(Vec<Val>, WaitKind)>) {
    match v {
        Some((tasks, kind)) => { w.u8(1); w.vals(tasks); put_wait_kind(w, kind); }
        None => w.u8(0),
    }
}

fn get_wfc(r: &mut R) -> Result<Option<(Vec<Val>, WaitKind)>, SnapErr> {
    Ok(if r.u8()? == 1 { Some((r.vals()?, get_wait_kind(r)?)) } else { None })
}

fn put_binding(w: &mut W, v: &Option<(Val, Val)>) {
    match v {
        Some((a, b)) => { w.u8(1); w.val(*a); w.val(*b); }
        None => w.u8(0),
    }
}

fn get_binding(r: &mut R) -> Result<Option<(Val, Val)>, SnapErr> {
    Ok(if r.u8()? == 1 { Some((r.val()?, r.val()?)) } else { None })
}

codec!(struct Pending, put_pending, get_pending {
    pos_delta: i32v,
    kw_delta: i32v,
    delta_save: [put_i32_pair, get_i32_pair],
    call_byte_pos: opt_u32,
    sleep_until_ns: opt_u64,
    host_frame_request: boolean,
    event_wait_request: boolean,
    host_call_request: boolean,
    host_call_id: u64,
    waiting_for_children: (put_wfc, get_wfc),
    exc_val: opt_val,
    method_binding: (put_binding, get_binding),
    preempt_request: default,
});

fn put_dict(w: &mut W, d: &DictMap) { w.seq(&d.entries, put_val_pair); }

fn get_dict(r: &mut R) -> Result<DictMap, SnapErr> {
    Ok(DictMap::from_entries(r.seq(get_val_pair)?))
}

fn put_set(w: &mut W, s: &ValSet) {
    let items: Vec<Val> = s.iter().copied().collect();
    w.vals(&items);
}

/* Sorted so identical states produce identical blobs. */
fn put_map(w: &mut W, m: &FxHashMap<String, Val>) {
    let mut pairs: Vec<(&String, &Val)> = m.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    w.usz(pairs.len());
    for (k, v) in pairs { w.str(k); w.val(*v); }
}

fn get_map(r: &mut R) -> Result<FxHashMap<String, Val>, SnapErr> {
    let n = r.count()?;
    let mut m = FxHashMap::default();
    for _ in 0..n {
        let k = r.str()?;
        m.insert(k, r.val()?);
    }
    Ok(m)
}

/* Set items are inserted in the rehash pass. */
enum SetFill {
    Mutable(Vec<Val>),
    Frozen(Vec<Val>),
}

fn put_obj(w: &mut W, obj: &HeapObj) {
    match obj {
        HeapObj::Str(s) => { w.u8(0); w.str(s); }
        HeapObj::Bytes(b) => { w.u8(1); w.bytes(b); }
        HeapObj::List(rc) => { w.u8(2); w.vals(&rc.borrow()); }
        HeapObj::Dict(rc) => { w.u8(3); put_dict(w, &rc.borrow()); }
        HeapObj::Set(rc) => { w.u8(4); put_set(w, &rc.borrow()); }
        HeapObj::FrozenSet(rc) => { w.u8(5); put_set(w, rc); }
        HeapObj::Tuple(v) => { w.u8(6); w.vals(v); }
        HeapObj::Func(fi, captures, defaults) => {
            w.u8(7); w.usz(*fi); w.vals(captures); w.seq(defaults, put_slot_val);
        }
        HeapObj::Range(s, e, st) => { w.u8(8); w.i64(*s); w.i64(*e); w.i64(*st); }
        HeapObj::Slice(a, b, c) => { w.u8(9); w.val(*a); w.val(*b); w.val(*c); }
        HeapObj::Ellipsis => w.u8(10),
        HeapObj::Type(n) => { w.u8(11); w.str(n); }
        HeapObj::NotImplemented => w.u8(12),
        HeapObj::LongInt(i) => { w.u8(13); w.i128v(*i); }
        HeapObj::ExcInstance(n, args) => { w.u8(14); w.str(n); w.vals(args); }
        HeapObj::BoundMethod(recv, id) => { w.u8(15); w.val(*recv); w.u8(id.raw()); }
        HeapObj::NativeFn(id) => { w.u8(16); w.str(id.name()); }
        HeapObj::Class(n, bases, members) => {
            w.u8(17); w.str(n); w.vals(bases); w.seq(&members.borrow(), put_name_val);
        }
        HeapObj::Instance(cls, dict) => { w.u8(18); w.val(*cls); put_dict(w, &dict.borrow()); }
        HeapObj::BoundUserMethod(a, b, c) => { w.u8(19); w.val(*a); w.val(*b); w.val(*c); }
        HeapObj::Super(a, b) => { w.u8(20); w.val(*a); w.val(*b); }
        HeapObj::Property(a, b) => { w.u8(21); w.val(*a); w.val(*b); }
        HeapObj::PropertySetter(a) => { w.u8(22); w.val(*a); }
        HeapObj::StaticMethod(a) => { w.u8(23); w.val(*a); }
        HeapObj::ClassMethod(a) => { w.u8(27); w.val(*a); }
        HeapObj::Coroutine(ip, slots, stack, body, iters, syncs, excs) => {
            w.u8(24); w.usz(*ip); w.vals(slots); w.vals(stack); put_body_ref(w, body);
            w.seq(iters, put_iter_frame); w.seq(syncs, put_sync_frame); w.seq(excs, put_exc_frame);
        }
        HeapObj::Module(spec, attrs) => { w.u8(25); w.str(spec); w.seq(attrs, put_name_val); }
        HeapObj::Extern(f) => { w.u8(26); w.str(&f.name); }
    }
}

fn get_obj(r: &mut R, externs: &ExternMap, fills: &mut Vec<(u32, SetFill)>, slot: u32) -> Result<HeapObj, SnapErr> {
    Ok(match r.u8()? {
        0 => HeapObj::Str(r.str()?),
        1 => HeapObj::Bytes(r.bytes()?),
        2 => HeapObj::List(Rc::new(RefCell::new(r.vals()?))),
        3 => HeapObj::Dict(Rc::new(RefCell::new(get_dict(r)?))),
        4 => {
            fills.push((slot, SetFill::Mutable(r.vals()?)));
            HeapObj::Set(Rc::new(RefCell::new(ValSet::default())))
        }
        5 => {
            fills.push((slot, SetFill::Frozen(r.vals()?)));
            HeapObj::FrozenSet(Rc::new(ValSet::default()))
        }
        6 => HeapObj::Tuple(r.vals()?),
        7 => HeapObj::Func(r.usz()?, r.vals()?, r.seq(get_slot_val)?),
        8 => HeapObj::Range(r.i64()?, r.i64()?, r.i64()?),
        9 => HeapObj::Slice(r.val()?, r.val()?, r.val()?),
        10 => HeapObj::Ellipsis,
        11 => HeapObj::Type(r.str()?),
        12 => HeapObj::NotImplemented,
        13 => HeapObj::LongInt(r.i128v()?),
        14 => HeapObj::ExcInstance(r.str()?, r.vals()?),
        15 => {
            let recv = r.val()?;
            let id = BuiltinMethodId::from_raw(r.u8()?).ok_or_else(|| "unknown builtin method id".to_string())?;
            HeapObj::BoundMethod(recv, id)
        }
        16 => {
            let name = r.str()?;
            HeapObj::NativeFn(NativeFnId::from_name(&name).ok_or_else(|| s_err("unknown builtin", &name))?)
        }
        17 => HeapObj::Class(r.str()?, r.vals()?, Rc::new(RefCell::new(r.seq(get_name_val)?))),
        18 => HeapObj::Instance(r.val()?, Rc::new(RefCell::new(get_dict(r)?))),
        19 => HeapObj::BoundUserMethod(r.val()?, r.val()?, r.val()?),
        20 => HeapObj::Super(r.val()?, r.val()?),
        21 => HeapObj::Property(r.val()?, r.val()?),
        22 => HeapObj::PropertySetter(r.val()?),
        23 => HeapObj::StaticMethod(r.val()?),
        24 => HeapObj::Coroutine(
            r.usz()?, r.vals()?, r.vals()?, get_body_ref(r)?,
            r.seq(get_iter_frame)?, r.seq(get_sync_frame)?, r.seq(get_exc_frame)?,
        ),
        25 => HeapObj::Module(r.str()?, r.seq(get_name_val)?),
        26 => {
            let name = r.str()?;
            HeapObj::Extern(externs.get(&name).ok_or_else(|| s_err("unknown native binding", &name))?.clone())
        }
        27 => HeapObj::ClassMethod(r.val()?),
        t => return Err(s_err("unknown heap tag", itoa::Buffer::new().format(t))),
    })
}

/* Structural fingerprint pins the blob to its bytecode. */
pub fn fingerprint(chunk: &SSAChunk) -> u64 {
    let mut h = FxHasher::default();
    fp_chunk(chunk, &mut h);
    h.finish()
}

fn fp_chunk(chunk: &SSAChunk, h: &mut FxHasher) {
    h.write_u64(chunk.instructions.len() as u64);
    for ins in &chunk.instructions {
        h.write_u64(((ins.opcode as u64) << 16) | ins.operand as u64);
    }
    h.write_u64(chunk.constants.len() as u64);
    h.write_u64(chunk.names.len() as u64);
    h.write_u64(chunk.extern_table.len() as u64);
    h.write_u64(chunk.functions.len() as u64);
    for (params, body, defaults, name_slot) in &chunk.functions {
        h.write_u64(params.len() as u64);
        h.write_u64(((*defaults as u64) << 16) | *name_slot as u64);
        fp_chunk(body, h);
    }
    h.write_u64(chunk.classes.len() as u64);
    for body in &chunk.classes { fp_chunk(body, h); }
    h.write_u64(chunk.imports.len() as u64);
    for entry in &chunk.imports {
        h.write_u64(entry.spec.len() as u64);
        if let ImportKind::Code(sub) = &entry.kind { fp_chunk(sub, h); }
    }
}

/* Single field list drives save and restore. */
macro_rules! vm_state {
    ($($f:ident: $s:tt),* $(,)?) => {
        fn put_vm_state(w: &mut W, vm: &VM) { $( codec!(@put w, (&vm.$f), $s); )* }
        fn get_vm_state(r: &mut R, vm: &mut VM) -> Result<(), SnapErr> {
            $( vm.$f = codec!(@get r, $s); )*
            Ok(())
        }
    };
}

vm_state! {
    stack: vals,
    iter_stack: [put_iter_frame, get_iter_frame],
    yields: vals,
    live_slots: vals,
    with_stack: vals,
    temp_roots: vals,
    event_queue: vals,
    globals: (put_map, get_map),
    module_state: (put_map, get_map),
    module_table: (put_map, get_map),
    observed_impure: [boolean],
    is_async: [boolean], // Filled by MakeCoroutine, not chunk-derivable.
    exception_stack: [put_exc_frame, get_exc_frame],
    unwind_stack: [put_unwind, get_unwind],
    handling_exc: opt_val,
    pending_sync_frames: [put_sync_frame, get_sync_frame],
    pending_exec_exc_base: opt_usz,
    pending: (put_pending, get_pending),
    scheduler: [put_handle, get_handle],
    next_host_call_id: u64,
    yielded: boolean,
    yield_from_value: val,
    resume_ip: usz,
    virtual_clock_ns: u64,
    error_byte_pos: opt_u32,
    output: [str],
    output_open: boolean,
    input_buffer: [str],
}

fn put_call_frame(w: &mut W, f: &CallFrame) {
    w.usz(f.fi);
    w.u32(f.call_byte_pos);
    w.str(&f.caller_path);
    w.opt_val(f.current_class);
    w.opt_val(f.current_self);
    w.seq(&f.cells, put_slot_val);
}

pub fn save(vm: &VM, source: &str) -> Vec<u8> {
    let mut w = W::new();
    w.u32(MAGIC);
    w.u32(FORMAT);
    w.u64(fingerprint(vm.chunk));
    w.str(source);
    w.usz(vm.budget);
    w.usz(vm.max_calls);
    w.usz(vm.heap.limit());
    w.boolean(vm.sandbox_off);
    w.boolean(vm.strict_input);
    w.usz(vm.heap.snapshot_objs().count());
    for obj in vm.heap.snapshot_objs() {
        match obj {
            None => w.u8(0),
            Some(o) => { w.u8(1); put_obj(&mut w, o); }
        }
    }
    put_vm_state(&mut w, vm);
    w.seq(&vm.call_stack, put_call_frame);
    w.b
}

struct Header<'a> {
    source: &'a str,
    fingerprint: u64,
    body: usize,
}

fn header(blob: &[u8]) -> Result<Header<'_>, SnapErr> {
    let mut r = R::new(blob);
    if r.u32()? != MAGIC { return Err("not an edge-python snapshot".to_string()); }
    if r.u32()? != FORMAT { return Err("unsupported snapshot format".to_string()); }
    let fp = r.u64()?;
    let n = r.count()?;
    let start = r.p;
    let source = core::str::from_utf8(r.take(n)?).map_err(|_| "snapshot source not utf-8".to_string())?;
    Ok(Header { source, fingerprint: fp, body: start + source.len() })
}

/* Host re-parses the embedded source before restoring. */
pub fn source_of(blob: &[u8]) -> Result<&str, SnapErr> {
    Ok(header(blob)?.source)
}

/* Sandbox profile recorded at save time. */
pub fn limits_of(blob: &[u8]) -> Result<Limits, SnapErr> {
    let h = header(blob)?;
    let mut r = R::new(blob);
    r.p = h.body;
    let _budget = r.usz()?;
    let calls = r.usz()?;
    let heap = r.usz()?;
    let sandbox_off = r.boolean()?;
    Ok(Limits { calls, ops: if sandbox_off { usize::MAX } else { 1 }, heap })
}

fn collect_externs(chunk: &SSAChunk, map: &mut ExternMap) {
    for f in &chunk.extern_table {
        map.entry(f.name.clone()).or_insert_with(|| f.clone());
    }
    for entry in &chunk.imports {
        match &entry.kind {
            ImportKind::Code(sub) => collect_externs(sub, map),
            ImportKind::Native { funcs, classes, consts } => {
                for f in funcs.iter().chain(consts.iter()) {
                    map.entry(f.name.clone()).or_insert_with(|| f.clone());
                }
                for c in classes {
                    for m in &c.methods {
                        map.entry(m.name.clone()).or_insert_with(|| m.clone());
                    }
                }
            }
        }
    }
    for (_, body, _, _) in &chunk.functions { collect_externs(body, map); }
    for body in &chunk.classes { collect_externs(body, map); }
}

fn source_index<'c>(chunk: &'c SSAChunk, out: &mut Vec<&'c SSAChunk>) {
    out.push(chunk);
    for entry in &chunk.imports {
        if let ImportKind::Code(sub) = &entry.kind { source_index(sub, out); }
    }
}

/* Overlay saved state onto a freshly booted VM. */
pub fn restore(vm: &mut VM, blob: &[u8]) -> Result<(), SnapErr> {
    let h = header(blob)?;
    if h.fingerprint != fingerprint(vm.chunk) {
        return Err("snapshot does not match this program or compiler version".to_string());
    }
    let mut r = R::new(blob);
    r.p = h.body;

    vm.budget = r.usz()?;
    vm.max_calls = r.usz()?;
    // Boot limits win over the recorded value.
    let _heap_limit = r.usz()?;
    vm.sandbox_off = r.boolean()?;
    vm.strict_input = r.boolean()?;

    let mut externs = ExternMap::default();
    collect_externs(vm.chunk, &mut externs);
    let nslots = r.count()?;
    let mut objs: Vec<Option<HeapObj>> = Vec::with_capacity(nslots);
    let mut fills: Vec<(u32, SetFill)> = Vec::new();
    for slot in 0..nslots {
        match r.u8()? {
            0 => objs.push(None),
            _ => objs.push(Some(get_obj(&mut r, &externs, &mut fills, slot as u32)?)),
        }
    }
    vm.heap.restore_objs(objs);

    get_vm_state(&mut r, vm)?;
    vm.waiting_for_children_count = vm.scheduler.iter()
        .filter(|h| matches!(h.state, CoroState::WaitingForChildren { .. }))
        .count();

    let chunk = vm.chunk;
    let mut sources: Vec<&SSAChunk> = Vec::new();
    source_index(chunk, &mut sources);
    vm.call_stack = r.seq(|r| {
        let fi = r.usz()?;
        let call_byte_pos = r.u32()?;
        let path = r.str()?;
        let current_class = r.opt_val()?;
        let current_self = r.opt_val()?;
        let cells = r.seq(get_slot_val)?;
        let owner = sources.iter().find(|c| c.path.as_str() == path).copied().unwrap_or(chunk);
        Ok(CallFrame {
            fi,
            call_byte_pos,
            caller_source: owner.source.clone(),
            caller_path: owner.path.clone(),
            current_class,
            current_self,
            cells,
        })
    })?;

    if r.p != r.b.len() { return Err("snapshot has trailing bytes".to_string()); }
    // Derived flag, not serialized: recompute from the restored module bindings.
    vm.builtins_rebound = vm.module_state.keys().any(|k| NativeFnId::from_name(k).is_some());
    rehash(vm, fills)?;
    rebuild_mro(vm)
}

/* Slot order caches bases before their subclasses. */
fn rebuild_mro(vm: &mut VM) -> Result<(), SnapErr> {
    let nslots = vm.heap.snapshot_objs().count();
    for idx in 0..nslots {
        let v = Val::heap(idx as u32);
        let bases = match vm.heap.try_get(v) {
            Some(HeapObj::Class(_, bases, _)) => bases.clone(),
            _ => continue,
        };
        let tail = vm.c3_merge(&bases).map_err(|_| "snapshot class hierarchy is inconsistent".to_string())?;
        let mut mro = Vec::with_capacity(tail.len() + 1);
        mro.push(v);
        mro.extend(tail);
        vm.mro_cache.insert(v.0, Rc::new(mro));
    }
    Ok(())
}

/* Hashing reads the heap, so index once slots are live. */
fn rehash(vm: &mut VM, fills: Vec<(u32, SetFill)>) -> Result<(), SnapErr> {
    let nslots = vm.heap.snapshot_objs().count();
    for idx in 0..nslots {
        let v = Val::heap(idx as u32);
        let dicts: Vec<Rc<RefCell<DictMap>>> = match vm.heap.try_get(v) {
            Some(HeapObj::Dict(rc)) => alloc::vec![rc.clone()],
            Some(HeapObj::Instance(_, rc)) => alloc::vec![rc.clone()],
            _ => continue,
        };
        for rc in dicts { rc.borrow_mut().rebuild_index(&vm.heap); }
    }
    for (slot, fill) in fills {
        match fill {
            SetFill::Mutable(items) => {
                let rc = match vm.heap.try_get(Val::heap(slot)) {
                    Some(HeapObj::Set(rc)) => rc.clone(),
                    _ => return Err("snapshot set slot mismatch".to_string()),
                };
                let mut s = rc.borrow_mut();
                for v in items { s.insert(v, &vm.heap); }
            }
            SetFill::Frozen(items) => {
                let mut s = ValSet::with_capacity(items.len());
                for v in &items { s.insert(*v, &vm.heap); }
                vm.heap.replace_obj(slot, HeapObj::FrozenSet(Rc::new(s)));
            }
        }
    }
    Ok(())
}

fn json_escape(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u00");
                let b = c as u8;
                out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
                out.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
            }
            c => out.push(c),
        }
    }
}

/* Module bindings as a {name: repr} JSON object. */
pub fn inspect_globals(vm: &VM) -> String {
    let mut out = String::from("{");
    let mut first = true;
    for h in &vm.scheduler {
        let Some(HeapObj::Coroutine(_, slots, _, BodyRef::Module, ..)) = vm.heap.try_get(h.coro) else { continue; };
        let slots = slots.clone();
        for (name, v) in super::init::collect_module_attrs(vm.chunk, &slots) {
            if !first { out.push(','); }
            first = false;
            out.push('"');
            json_escape(&mut out, &name);
            out.push_str("\":\"");
            json_escape(&mut out, &vm.display(v));
            out.push('"');
        }
        break;
    }
    out.push('}');
    out
}

/* Scheduled coroutines as a JSON array. */
pub fn inspect_stack(vm: &VM) -> String {
    let fn_name = |fi: usize| -> &str {
        match vm.function_names.get(fi) {
            Some(n) if !n.is_empty() => n,
            _ => "<lambda>",
        }
    };
    let mut out = String::from("[");
    for (i, h) in vm.scheduler.iter().enumerate() {
        if i > 0 { out.push(','); }
        let state = match &h.state {
            CoroState::Ready => "ready",
            CoroState::Sleeping(_) => "sleeping",
            CoroState::WaitingFrame => "waiting_frame",
            CoroState::WaitingEvent => "waiting_event",
            CoroState::WaitingHostCall(_) => "waiting_host_call",
            CoroState::WaitingForChildren { .. } => "waiting_for_children",
            CoroState::CancelPending => "cancel_pending",
            CoroState::Done(_) => "done",
            CoroState::Errored(_) => "errored",
            CoroState::Cancelled => "cancelled",
        };
        out.push_str("{\"state\":\"");
        out.push_str(state);
        out.push_str("\",\"function\":\"");
        match vm.heap.try_get(h.coro) {
            Some(HeapObj::Coroutine(ip, _, _, body, _, syncs, _)) => {
                match body {
                    BodyRef::Module => json_escape(&mut out, "<module>"),
                    BodyRef::Fn(fi) => json_escape(&mut out, fn_name(*fi)),
                }
                out.push_str("\",\"ip\":");
                out.push_str(itoa::Buffer::new().format(*ip));
                out.push_str(",\"frames\":[");
                for (j, f) in syncs.iter().enumerate() {
                    if j > 0 { out.push(','); }
                    out.push_str("{\"function\":\"");
                    json_escape(&mut out, fn_name(f.fi));
                    out.push_str("\",\"ip\":");
                    out.push_str(itoa::Buffer::new().format(f.ip));
                    out.push('}');
                }
                out.push_str("]}");
            }
            _ => out.push_str("\",\"ip\":0,\"frames\":[]}"),
        }
    }
    out.push(']');
    out
}
