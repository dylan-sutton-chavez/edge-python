use alloc::borrow::ToOwned;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::modules::parser::types::{ImportKind, SSAChunk};
use super::VM;
use super::types::*;

const MAGIC: u32 = 0x4E53_5045;
const FORMAT: u32 = 1;

pub type SnapErr = String;

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
    fn vals(&mut self, v: &[Val]) { self.usz(v.len()); for x in v { self.val(*x); } }
    fn opt_val(&mut self, v: Option<Val>) {
        match v { Some(x) => { self.u8(1); self.val(x); } None => self.u8(0) }
    }
    fn opt_u64(&mut self, v: Option<u64>) {
        match v { Some(x) => { self.u8(1); self.u64(x); } None => self.u8(0) }
    }
    fn opt_usz(&mut self, v: Option<usize>) {
        match v { Some(x) => { self.u8(1); self.usz(x); } None => self.u8(0) }
    }
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
        let v = self.u64()?;
        usize::try_from(v).map_err(|_| "snapshot value out of range".to_string())
    }
    /* Bounded count: rejects lengths beyond the remaining payload so corrupt blobs cannot force huge allocations. */
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
    fn val(&mut self) -> Result<Val, SnapErr> { Ok(Val(self.u64()?)) }
    fn vals(&mut self) -> Result<Vec<Val>, SnapErr> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n { v.push(self.val()?); }
        Ok(v)
    }
    fn opt_val(&mut self) -> Result<Option<Val>, SnapErr> {
        Ok(if self.u8()? == 1 { Some(self.val()?) } else { None })
    }
    fn opt_u64(&mut self) -> Result<Option<u64>, SnapErr> {
        Ok(if self.u8()? == 1 { Some(self.u64()?) } else { None })
    }
    fn opt_usz(&mut self) -> Result<Option<usize>, SnapErr> {
        Ok(if self.u8()? == 1 { Some(self.usz()?) } else { None })
    }
}

/* Structural chunk fingerprint (FNV-1a): pins the blob to bytecode the restore-side re-parse must reproduce. */
pub fn fingerprint(chunk: &SSAChunk) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    fp_chunk(chunk, &mut h);
    h
}

fn fp_mix(h: &mut u64, v: u64) {
    for b in v.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn fp_chunk(chunk: &SSAChunk, h: &mut u64) {
    fp_mix(h, chunk.instructions.len() as u64);
    for ins in &chunk.instructions {
        fp_mix(h, ((ins.opcode as u64) << 16) | ins.operand as u64);
    }
    fp_mix(h, chunk.constants.len() as u64);
    fp_mix(h, chunk.names.len() as u64);
    fp_mix(h, chunk.extern_table.len() as u64);
    fp_mix(h, chunk.functions.len() as u64);
    for (params, body, defaults, name_slot) in &chunk.functions {
        fp_mix(h, params.len() as u64);
        fp_mix(h, ((*defaults as u64) << 16) | *name_slot as u64);
        fp_chunk(body, h);
    }
    fp_mix(h, chunk.classes.len() as u64);
    for body in &chunk.classes { fp_chunk(body, h); }
    fp_mix(h, chunk.imports.len() as u64);
    for entry in &chunk.imports {
        fp_mix(h, entry.spec.len() as u64);
        if let ImportKind::Code(sub) = &entry.kind { fp_chunk(sub, h); }
    }
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
            w.u8(7); w.usz(*fi); w.vals(captures);
            w.usz(defaults.len());
            for (slot, v) in defaults { w.usz(*slot); w.val(*v); }
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
            w.u8(17); w.str(n); w.vals(bases);
            let m = members.borrow();
            w.usz(m.len());
            for (name, v) in m.iter() { w.str(name); w.val(*v); }
        }
        HeapObj::Instance(cls, dict) => { w.u8(18); w.val(*cls); put_dict(w, &dict.borrow()); }
        HeapObj::BoundUserMethod(a, b, c) => { w.u8(19); w.val(*a); w.val(*b); w.val(*c); }
        HeapObj::Super(a, b) => { w.u8(20); w.val(*a); w.val(*b); }
        HeapObj::Property(a, b) => { w.u8(21); w.val(*a); w.val(*b); }
        HeapObj::PropertySetter(a) => { w.u8(22); w.val(*a); }
        HeapObj::StaticMethod(a) => { w.u8(23); w.val(*a); }
        HeapObj::Coroutine(ip, slots, stack, body, iters, syncs, excs) => {
            w.u8(24); w.usz(*ip); w.vals(slots); w.vals(stack);
            put_body_ref(w, body);
            w.usz(iters.len());
            for f in iters { put_iter_frame(w, f); }
            w.usz(syncs.len());
            for f in syncs { put_sync_frame(w, f); }
            w.usz(excs.len());
            for f in excs { put_exc_frame(w, f); }
        }
        HeapObj::Module(spec, attrs) => {
            w.u8(25); w.str(spec);
            w.usz(attrs.len());
            for (name, v) in attrs { w.str(name); w.val(*v); }
        }
        HeapObj::Extern(f) => { w.u8(26); w.str(&f.name); }
    }
}

/* Sets decode empty; their items are stashed and inserted in the rehash pass once the heap exists. */
enum SetFill {
    Mutable(Vec<Val>),
    Frozen(Vec<Val>),
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
        7 => {
            let fi = r.usz()?;
            let captures = r.vals()?;
            let n = r.count()?;
            let mut defaults = Vec::with_capacity(n);
            for _ in 0..n { defaults.push((r.usz()?, r.val()?)); }
            HeapObj::Func(fi, captures, defaults)
        }
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
            let id = NativeFnId::from_name(&name).ok_or_else(|| s_err("unknown builtin", &name))?;
            HeapObj::NativeFn(id)
        }
        17 => {
            let n = r.str()?;
            let bases = r.vals()?;
            let count = r.count()?;
            let mut members = Vec::with_capacity(count);
            for _ in 0..count { members.push((r.str()?, r.val()?)); }
            HeapObj::Class(n, bases, Rc::new(RefCell::new(members)))
        }
        18 => HeapObj::Instance(r.val()?, Rc::new(RefCell::new(get_dict(r)?))),
        19 => HeapObj::BoundUserMethod(r.val()?, r.val()?, r.val()?),
        20 => HeapObj::Super(r.val()?, r.val()?),
        21 => HeapObj::Property(r.val()?, r.val()?),
        22 => HeapObj::PropertySetter(r.val()?),
        23 => HeapObj::StaticMethod(r.val()?),
        24 => {
            let ip = r.usz()?;
            let slots = r.vals()?;
            let stack = r.vals()?;
            let body = get_body_ref(r)?;
            let n = r.count()?;
            let mut iters = Vec::with_capacity(n);
            for _ in 0..n { iters.push(get_iter_frame(r)?); }
            let n = r.count()?;
            let mut syncs = Vec::with_capacity(n);
            for _ in 0..n { syncs.push(get_sync_frame(r)?); }
            let n = r.count()?;
            let mut excs = Vec::with_capacity(n);
            for _ in 0..n { excs.push(get_exc_frame(r)?); }
            HeapObj::Coroutine(ip, slots, stack, body, iters, syncs, excs)
        }
        25 => {
            let spec = r.str()?;
            let n = r.count()?;
            let mut attrs = Vec::with_capacity(n);
            for _ in 0..n { attrs.push((r.str()?, r.val()?)); }
            HeapObj::Module(spec, attrs)
        }
        26 => {
            let name = r.str()?;
            let f = externs.get(&name).ok_or_else(|| s_err("unknown native binding", &name))?;
            HeapObj::Extern(f.clone())
        }
        t => return Err(s_err("unknown heap tag", &t.to_string())),
    })
}

fn s_err(what: &str, detail: &str) -> SnapErr {
    let mut s = String::from("snapshot: ");
    s.push_str(what);
    s.push_str(" '");
    s.push_str(detail);
    s.push('\'');
    s
}

fn put_dict(w: &mut W, d: &DictMap) {
    w.usz(d.entries.len());
    for (k, v) in &d.entries { w.val(*k); w.val(*v); }
}

fn get_dict(r: &mut R) -> Result<DictMap, SnapErr> {
    let n = r.count()?;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n { entries.push((r.val()?, r.val()?)); }
    Ok(DictMap::from_entries(entries))
}

fn put_set(w: &mut W, s: &ValSet) {
    let items: Vec<Val> = s.iter().copied().collect();
    w.vals(&items);
}

fn put_body_ref(w: &mut W, b: &BodyRef) {
    match b {
        BodyRef::Fn(fi) => { w.u8(0); w.usz(*fi); }
        BodyRef::Module => w.u8(1),
    }
}

fn get_body_ref(r: &mut R) -> Result<BodyRef, SnapErr> {
    Ok(match r.u8()? {
        0 => BodyRef::Fn(r.usz()?),
        _ => BodyRef::Module,
    })
}

fn put_iter_frame(w: &mut W, f: &IterFrame) {
    match f {
        IterFrame::Seq { items, idx } => { w.u8(0); w.vals(items); w.usz(*idx); }
        IterFrame::Range { cur, end, step } => { w.u8(1); w.i64(*cur); w.i64(*end); w.i64(*step); }
        IterFrame::Coroutine(v) => { w.u8(2); w.val(*v); }
        IterFrame::UserDefined(v) => { w.u8(3); w.val(*v); }
    }
}

fn get_iter_frame(r: &mut R) -> Result<IterFrame, SnapErr> {
    Ok(match r.u8()? {
        0 => IterFrame::Seq { items: r.vals()?, idx: r.usz()? },
        1 => IterFrame::Range { cur: r.i64()?, end: r.i64()?, step: r.i64()? },
        2 => IterFrame::Coroutine(r.val()?),
        _ => IterFrame::UserDefined(r.val()?),
    })
}

fn put_sync_frame(w: &mut W, f: &SyncFrame) {
    w.usz(f.ip);
    w.usz(f.fi);
    w.vals(&f.slots);
    w.vals(&f.stack_delta);
    w.usz(f.iter_delta.len());
    for it in &f.iter_delta { put_iter_frame(w, it); }
    w.usz(f.exception_delta.len());
    for e in &f.exception_delta { put_exc_frame(w, e); }
}

fn get_sync_frame(r: &mut R) -> Result<SyncFrame, SnapErr> {
    let ip = r.usz()?;
    let fi = r.usz()?;
    let slots = r.vals()?;
    let stack_delta = r.vals()?;
    let n = r.count()?;
    let mut iter_delta = Vec::with_capacity(n);
    for _ in 0..n { iter_delta.push(get_iter_frame(r)?); }
    let n = r.count()?;
    let mut exception_delta = Vec::with_capacity(n);
    for _ in 0..n { exception_delta.push(get_exc_frame(r)?); }
    Ok(SyncFrame { ip, fi, slots, stack_delta, iter_delta, exception_delta })
}

fn put_exc_frame(w: &mut W, f: &ExceptionFrame) {
    w.u8(matches!(f.kind, BlockKind::Finally) as u8);
    w.usz(f.handler_ip);
    w.usz(f.stack_depth);
    w.usz(f.iter_depth);
    w.usz(f.with_depth);
    w.usz(f.unwind_depth);
}

fn get_exc_frame(r: &mut R) -> Result<ExceptionFrame, SnapErr> {
    let kind = if r.u8()? == 1 { BlockKind::Finally } else { BlockKind::Except };
    Ok(ExceptionFrame {
        kind,
        handler_ip: r.usz()?,
        stack_depth: r.usz()?,
        iter_depth: r.usz()?,
        with_depth: r.usz()?,
        unwind_depth: r.usz()?,
    })
}

fn put_vm_err(w: &mut W, e: &VmErr) {
    match e {
        VmErr::CallDepth => w.u8(0),
        VmErr::Heap => w.u8(1),
        VmErr::Budget => w.u8(2),
        VmErr::ZeroDiv => w.u8(3),
        VmErr::Overflow => w.u8(4),
        VmErr::Name(s) => { w.u8(5); w.str(s); }
        VmErr::Type(s) => { w.u8(6); w.str(s); }
        VmErr::TypeMsg(s) => { w.u8(7); w.str(s); }
        VmErr::Value(s) => { w.u8(8); w.str(s); }
        VmErr::Runtime(s) => { w.u8(9); w.str(s); }
        VmErr::Attribute(s) => { w.u8(10); w.str(s); }
        VmErr::Raised(s) => { w.u8(11); w.str(s); }
        VmErr::HostYield(st) => {
            w.u8(12);
            match st {
                SchedulerStatus::Done => w.u8(0),
                SchedulerStatus::PendingTimer(d) => { w.u8(1); w.u64(*d); }
                SchedulerStatus::PendingFrame => w.u8(2),
                SchedulerStatus::PendingEvent => w.u8(3),
                SchedulerStatus::PendingHostCall => w.u8(4),
                SchedulerStatus::Preempted => w.u8(5),
            }
        }
        VmErr::HostCallDeferred => w.u8(13),
    }
}

/* Static-str variants leak their decoded message; stored errors are rare, so the cost is a few bytes per restore. */
fn get_vm_err(r: &mut R) -> Result<VmErr, SnapErr> {
    fn leak(s: String) -> &'static str { alloc::boxed::Box::leak(s.into_boxed_str()) }
    Ok(match r.u8()? {
        0 => VmErr::CallDepth,
        1 => VmErr::Heap,
        2 => VmErr::Budget,
        3 => VmErr::ZeroDiv,
        4 => VmErr::Overflow,
        5 => VmErr::Name(r.str()?),
        6 => VmErr::Type(leak(r.str()?)),
        7 => VmErr::TypeMsg(r.str()?),
        8 => VmErr::Value(leak(r.str()?)),
        9 => VmErr::Runtime(leak(r.str()?)),
        10 => VmErr::Attribute(r.str()?),
        11 => VmErr::Raised(r.str()?),
        12 => VmErr::HostYield(match r.u8()? {
            0 => SchedulerStatus::Done,
            1 => SchedulerStatus::PendingTimer(r.u64()?),
            2 => SchedulerStatus::PendingFrame,
            3 => SchedulerStatus::PendingEvent,
            4 => SchedulerStatus::PendingHostCall,
            _ => SchedulerStatus::Preempted,
        }),
        _ => VmErr::HostCallDeferred,
    })
}

fn put_unwind(w: &mut W, u: &Unwind) {
    match u {
        Unwind::Normal => w.u8(0),
        Unwind::Return(v) => { w.u8(1); w.val(*v); }
        Unwind::Goto { target, remaining } => { w.u8(2); w.usz(*target); w.u16(*remaining); }
        Unwind::Reraise(e) => { w.u8(3); put_vm_err(w, e); }
    }
}

fn get_unwind(r: &mut R) -> Result<Unwind, SnapErr> {
    Ok(match r.u8()? {
        0 => Unwind::Normal,
        1 => Unwind::Return(r.val()?),
        2 => Unwind::Goto { target: r.usz()?, remaining: r.u16()? },
        _ => Unwind::Reraise(get_vm_err(r)?),
    })
}

fn put_wait_kind(w: &mut W, k: &WaitKind) {
    match k {
        WaitKind::Run(v) => { w.u8(0); w.val(*v); }
        WaitKind::Gather => w.u8(1),
        WaitKind::Timeout { deadline_ns, target } => { w.u8(2); w.u64(*deadline_ns); w.val(*target); }
    }
}

fn get_wait_kind(r: &mut R) -> Result<WaitKind, SnapErr> {
    Ok(match r.u8()? {
        0 => WaitKind::Run(r.val()?),
        1 => WaitKind::Gather,
        _ => WaitKind::Timeout { deadline_ns: r.u64()?, target: r.val()? },
    })
}

fn put_coro_state(w: &mut W, s: &CoroState) {
    match s {
        CoroState::Ready => w.u8(0),
        CoroState::Sleeping(d) => { w.u8(1); w.u64(*d); }
        CoroState::WaitingFrame => w.u8(2),
        CoroState::WaitingEvent => w.u8(3),
        CoroState::WaitingHostCall(id) => { w.u8(4); w.u64(*id); }
        CoroState::WaitingForChildren { tasks, kind } => { w.u8(5); w.vals(tasks); put_wait_kind(w, kind); }
        CoroState::CancelPending => w.u8(6),
        CoroState::Done(v) => { w.u8(7); w.val(*v); }
        CoroState::Errored(e) => { w.u8(8); put_vm_err(w, e); }
        CoroState::Cancelled => w.u8(9),
    }
}

fn get_coro_state(r: &mut R) -> Result<CoroState, SnapErr> {
    Ok(match r.u8()? {
        0 => CoroState::Ready,
        1 => CoroState::Sleeping(r.u64()?),
        2 => CoroState::WaitingFrame,
        3 => CoroState::WaitingEvent,
        4 => CoroState::WaitingHostCall(r.u64()?),
        5 => CoroState::WaitingForChildren { tasks: r.vals()?, kind: get_wait_kind(r)? },
        6 => CoroState::CancelPending,
        7 => CoroState::Done(r.val()?),
        8 => CoroState::Errored(get_vm_err(r)?),
        _ => CoroState::Cancelled,
    })
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

    let objs: Vec<Option<&HeapObj>> = vm.heap.snapshot_objs().collect();
    w.usz(objs.len());
    for obj in objs {
        match obj {
            None => w.u8(0),
            Some(o) => { w.u8(1); put_obj(&mut w, o); }
        }
    }

    w.vals(&vm.stack);
    w.usz(vm.iter_stack.len());
    for f in &vm.iter_stack { put_iter_frame(&mut w, f); }
    w.vals(&vm.yields);
    w.vals(&vm.live_slots);
    w.vals(&vm.with_stack);
    w.vals(&vm.temp_roots);
    w.vals(&vm.event_queue);

    put_str_val_map(&mut w, vm.globals.iter());
    put_str_val_map(&mut w, vm.module_state.iter());
    put_str_val_map(&mut w, vm.module_table.iter());

    w.usz(vm.observed_impure.len());
    for &b in &vm.observed_impure { w.boolean(b); }
    // Filled by the MakeCoroutine opcode, not derivable from the chunk.
    w.usz(vm.is_async.len());
    for &b in &vm.is_async { w.boolean(b); }

    w.usz(vm.exception_stack.len());
    for f in &vm.exception_stack { put_exc_frame(&mut w, f); }
    w.usz(vm.unwind_stack.len());
    for u in &vm.unwind_stack { put_unwind(&mut w, u); }
    w.opt_val(vm.handling_exc);

    w.usz(vm.pending_sync_frames.len());
    for f in &vm.pending_sync_frames { put_sync_frame(&mut w, f); }
    w.opt_usz(vm.pending_exec_exc_base);

    let p = &vm.pending;
    w.i32v(p.pos_delta);
    w.i32v(p.kw_delta);
    w.usz(p.delta_save.len());
    for (a, b) in &p.delta_save { w.i32v(*a); w.i32v(*b); }
    match p.call_byte_pos { Some(v) => { w.u8(1); w.u32(v); } None => w.u8(0) }
    w.opt_u64(p.sleep_until_ns);
    w.boolean(p.host_frame_request);
    w.boolean(p.event_wait_request);
    w.boolean(p.host_call_request);
    w.u64(p.host_call_id);
    match &p.waiting_for_children {
        Some((tasks, kind)) => { w.u8(1); w.vals(tasks); put_wait_kind(&mut w, kind); }
        None => w.u8(0),
    }
    w.opt_val(p.exc_val);
    match p.method_binding { Some((a, b)) => { w.u8(1); w.val(a); w.val(b); } None => w.u8(0) }

    w.usz(vm.call_stack.len());
    for f in &vm.call_stack {
        w.usz(f.fi);
        w.u32(f.call_byte_pos);
        w.str(&f.caller_path);
        w.opt_val(f.current_class);
        w.opt_val(f.current_self);
        w.usz(f.cells.len());
        for (slot, v) in &f.cells { w.usz(*slot); w.val(*v); }
    }

    w.usz(vm.scheduler.len());
    for h in &vm.scheduler {
        w.val(h.coro);
        put_coro_state(&mut w, &h.state);
    }

    w.u64(vm.next_host_call_id);
    w.boolean(vm.yielded);
    w.val(vm.yield_from_value);
    w.usz(vm.resume_ip);
    w.u64(vm.virtual_clock_ns);
    match vm.error_byte_pos { Some(v) => { w.u8(1); w.u32(v); } None => w.u8(0) }

    w.usz(vm.output.len());
    for line in &vm.output { w.str(line); }
    w.boolean(vm.output_open);
    w.usz(vm.input_buffer.len());
    for line in &vm.input_buffer { w.str(line); }

    w.b
}

/* Sorted by key so identical states produce identical blobs. */
fn put_str_val_map<'m>(w: &mut W, it: impl Iterator<Item = (&'m String, &'m Val)>) {
    let mut pairs: Vec<(&String, &Val)> = it.collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    w.usz(pairs.len());
    for (k, v) in pairs { w.str(k); w.val(*v); }
}

fn get_str_val_map(r: &mut R) -> Result<crate::util::fx::FxHashMap<String, Val>, SnapErr> {
    let n = r.count()?;
    let mut m = crate::util::fx::FxHashMap::default();
    for _ in 0..n {
        let k = r.str()?;
        m.insert(k, r.val()?);
    }
    Ok(m)
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

/* Embedded source; the host re-parses it and boots the VM the restore is applied onto. */
pub fn source_of(blob: &[u8]) -> Result<&str, SnapErr> {
    Ok(header(blob)?.source)
}

/* Sandbox profile recorded at save time; boot the restore VM with it. */
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

type ExternMap = crate::util::fx::FxHashMap<String, ExternFn>;

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

/* Overwrite `vm`'s dynamic state from `blob`. `vm` must be freshly booted (`with_limits`) from the blob's own source; chunk-derived tables stay, caches stay empty and rebuild lazily. */
pub fn restore(vm: &mut VM, blob: &[u8]) -> Result<(), SnapErr> {
    let h = header(blob)?;
    if h.fingerprint != fingerprint(vm.chunk) {
        return Err("snapshot does not match this program or compiler version".to_string());
    }
    let mut r = R::new(blob);
    r.p = h.body;

    vm.budget = r.usz()?;
    vm.max_calls = r.usz()?;
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

    vm.stack = r.vals()?;
    let n = r.count()?;
    vm.iter_stack = Vec::with_capacity(n);
    for _ in 0..n { vm.iter_stack.push(get_iter_frame(&mut r)?); }
    vm.yields = r.vals()?;
    vm.live_slots = r.vals()?;
    vm.with_stack = r.vals()?;
    vm.temp_roots = r.vals()?;
    vm.event_queue = r.vals()?;

    vm.globals = get_str_val_map(&mut r)?;
    vm.module_state = get_str_val_map(&mut r)?;
    vm.module_table = get_str_val_map(&mut r)?;

    let n = r.count()?;
    vm.observed_impure = Vec::with_capacity(n);
    for _ in 0..n { vm.observed_impure.push(r.boolean()?); }
    let n = r.count()?;
    vm.is_async = Vec::with_capacity(n);
    for _ in 0..n { vm.is_async.push(r.boolean()?); }

    let n = r.count()?;
    vm.exception_stack = Vec::with_capacity(n);
    for _ in 0..n { vm.exception_stack.push(get_exc_frame(&mut r)?); }
    let n = r.count()?;
    vm.unwind_stack = Vec::with_capacity(n);
    for _ in 0..n { vm.unwind_stack.push(get_unwind(&mut r)?); }
    vm.handling_exc = r.opt_val()?;

    let n = r.count()?;
    vm.pending_sync_frames = Vec::with_capacity(n);
    for _ in 0..n { vm.pending_sync_frames.push(get_sync_frame(&mut r)?); }
    vm.pending_exec_exc_base = r.opt_usz()?;

    vm.pending.pos_delta = r.i32v()?;
    vm.pending.kw_delta = r.i32v()?;
    let n = r.count()?;
    vm.pending.delta_save = Vec::with_capacity(n);
    for _ in 0..n { vm.pending.delta_save.push((r.i32v()?, r.i32v()?)); }
    vm.pending.call_byte_pos = if r.u8()? == 1 { Some(r.u32()?) } else { None };
    vm.pending.sleep_until_ns = r.opt_u64()?;
    vm.pending.host_frame_request = r.boolean()?;
    vm.pending.event_wait_request = r.boolean()?;
    vm.pending.host_call_request = r.boolean()?;
    vm.pending.host_call_id = r.u64()?;
    vm.pending.waiting_for_children = if r.u8()? == 1 {
        Some((r.vals()?, get_wait_kind(&mut r)?))
    } else { None };
    vm.pending.exc_val = r.opt_val()?;
    vm.pending.method_binding = if r.u8()? == 1 { Some((r.val()?, r.val()?)) } else { None };

    let mut sources: Vec<&SSAChunk> = Vec::new();
    source_index(vm.chunk, &mut sources);
    let n = r.count()?;
    let mut call_stack = Vec::with_capacity(n);
    for _ in 0..n {
        let fi = r.usz()?;
        let call_byte_pos = r.u32()?;
        let path = r.str()?;
        let current_class = r.opt_val()?;
        let current_self = r.opt_val()?;
        let cn = r.count()?;
        let mut cells = Vec::with_capacity(cn);
        for _ in 0..cn { cells.push((r.usz()?, r.val()?)); }
        let owner = sources.iter().find(|c| c.path.as_str() == path).copied().unwrap_or(vm.chunk);
        call_stack.push(CallFrame {
            fi,
            call_byte_pos,
            caller_source: owner.source.clone(),
            caller_path: owner.path.clone(),
            current_class,
            current_self,
            cells,
        });
    }
    vm.call_stack = call_stack;

    let n = r.count()?;
    vm.scheduler = Vec::with_capacity(n);
    for _ in 0..n {
        let coro = r.val()?;
        let state = get_coro_state(&mut r)?;
        vm.scheduler.push(CoroutineHandle { coro, state });
    }
    vm.waiting_for_children_count = vm.scheduler.iter()
        .filter(|h| matches!(h.state, CoroState::WaitingForChildren { .. }))
        .count();

    vm.next_host_call_id = r.u64()?;
    vm.yielded = r.boolean()?;
    vm.yield_from_value = r.val()?;
    vm.resume_ip = r.usz()?;
    vm.virtual_clock_ns = r.u64()?;
    vm.error_byte_pos = if r.u8()? == 1 { Some(r.u32()?) } else { None };

    let n = r.count()?;
    vm.output = Vec::with_capacity(n);
    for _ in 0..n { vm.output.push(r.str()?); }
    vm.output_open = r.boolean()?;
    let n = r.count()?;
    vm.input_buffer = Vec::with_capacity(n);
    for _ in 0..n { vm.input_buffer.push(r.str()?); }

    if r.p != r.b.len() { return Err("snapshot has trailing bytes".to_string()); }

    rehash(vm, fills)?;
    rebuild_mro(vm)?;
    Ok(())
}

/* The uncached lookup path is a plain DFS, not C3, so every restored class recomputes its linearization. Slot order is creation order, so bases are always cached before their subclasses. */
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

/* Second pass: hashing reads the heap, so dict indexes and set tables can only be built once every slot is live. */
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

/* JSON object of module-level bindings and their reprs, read from the parked module-body coroutine. */
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

/* JSON array describing each scheduled coroutine: state, function, ip, and suspended sync frames. */
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
            CoroState::Ready => "ready".to_owned(),
            CoroState::Sleeping(_) => "sleeping".to_owned(),
            CoroState::WaitingFrame => "waiting_frame".to_owned(),
            CoroState::WaitingEvent => "waiting_event".to_owned(),
            CoroState::WaitingHostCall(_) => "waiting_host_call".to_owned(),
            CoroState::WaitingForChildren { .. } => "waiting_for_children".to_owned(),
            CoroState::CancelPending => "cancel_pending".to_owned(),
            CoroState::Done(_) => "done".to_owned(),
            CoroState::Errored(_) => "errored".to_owned(),
            CoroState::Cancelled => "cancelled".to_owned(),
        };
        out.push_str("{\"state\":\"");
        out.push_str(&state);
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
