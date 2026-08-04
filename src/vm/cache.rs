use super::types::{Val, HeapObj, HeapPool, VmErr, eq_vals_with_heap};
use crate::parser::{OpCode, SSAChunk, Instruction, Value};

use alloc::{vec, vec::Vec, string::ToString};

/* Type-specialised binop variants reachable from the inline cache. */
#[derive(Debug, Clone, Copy)]
pub enum FastOp {
    AddInt, AddFloat, AddStr,
    SubInt, SubFloat,
    MulInt, MulFloat,
    LtInt, LtFloat,
    GtInt, LtEqInt, GtEqInt,
    EqInt, EqStr,
    NotEqInt,
    ModInt, FloorDivInt
}

/* Promote to `fast` after this many hits with a stable type key. */
const QUICK_THRESH: u8 = 4;

/* Pper-site monomorphic instance-dunder cache. Records the receiver's class heap idx and the pre-resolved method Val; once `hits >= QUICK_THRESH` the slot promotes and the hot dispatch skips `resolve_attr_silent` entirely. `arity` is the total operand count consumed from the stack (1 for unary, 2 for binary like `__add__`/`__getitem__`). */
#[derive(Clone, Copy)]
pub struct InstanceCache {
    pub class: u32,
    pub method_bits: u64,
    pub arity: u8,
    hits: u8,
    promoted: bool,
}

#[derive(Clone, Default)]
struct CacheSlot {
    type_key: u8,
    hits: u8,
    fast: Option<FastOp>,
    // Instance-dunder cache; orthogonal to `fast`, dispatch checks it after scalar specialisation misses.
    inst: Option<InstanceCache>,
}

pub struct OpcodeCache {
    slots: Vec<CacheSlot>,
    fused: Option<Vec<Instruction>>,
    /* Pre-materialised const pool so LoadConst is one indexed load, no per-iter alloc. */
    const_vals: Option<Vec<Val>>,
}

impl OpcodeCache {
    pub fn new(chunk: &SSAChunk) -> Self {
        Self {
            slots: vec![CacheSlot::default(); chunk.instructions.len()],
            fused: None,
            const_vals: None,
        }
    }

    /* Compile the fused instruction stream on first access; reuse afterwards. */
    pub fn ensure_fused(&mut self, chunk: &SSAChunk) -> &[Instruction] {
        if self.fused.is_none() {
            self.fused = Some(fuse_method_calls(chunk));
        }
        self.fused.as_ref().unwrap()
    }

    /* Direct access (caller must have called ensure_fused). */
    pub fn fused_ref(&self) -> &[Instruction] {
        self.fused.as_ref().expect("fused code not compiled")
    }

    /* Build the const pool: scalars inline, Str/LongInt heap-allocated once and shared. */
    pub fn ensure_const_vals(&mut self, chunk: &SSAChunk, heap: &mut HeapPool) -> Result<&[Val], VmErr> {
        if self.const_vals.is_none() {
            let mut out = Vec::with_capacity(chunk.constants.len());
            for c in &chunk.constants {
                let v = match c {
                    Value::Int(i) => {
                        if *i >= Val::INT_MIN && *i <= Val::INT_MAX { Val::int(*i) }
                        // Defensive path for FFI/wire-format chunks; parser now emits LongInt directly.
                        else { heap.alloc(HeapObj::LongInt(*i as i128))? }
                    }
                    Value::LongInt(i) => {
                        // Demote when it fits inline so hash/eq stay in sync with literals.
                        if *i >= Val::INT_MIN as i128 && *i <= Val::INT_MAX as i128 {
                            Val::int(*i as i64)
                        } else {
                            heap.alloc(HeapObj::LongInt(*i))?
                        }
                    }
                    Value::Float(f) => Val::float(*f),
                    Value::Bool(b) => Val::bool(*b),
                    Value::None => Val::none(),
                    Value::Str(s) => heap.alloc(HeapObj::Str(s.to_string()))?,
                    Value::Bytes(b) => heap.alloc(HeapObj::Bytes(b.clone()))?,
                };
                out.push(v);
            }
            self.const_vals = Some(out);
        }
        Ok(self.const_vals.as_ref().unwrap())
    }

    /* Direct access (caller must have called ensure_const_vals). */
    pub fn const_vals_ref(&self) -> &[Val] {
        self.const_vals.as_ref().expect("const pool not materialized")
    }

    pub fn const_vals_opt(&self) -> Option<&[Val]> {
        self.const_vals.as_deref()
    }

    pub fn record(&mut self, ip: usize, opcode: &OpCode, ta: u8, tb: u8) {
        let Some(s) = self.slots.get_mut(ip) else { return };
        let key = (ta << 4) | (tb & 0xF);
        if s.type_key == key {
            s.hits = s.hits.saturating_add(1);
            if s.hits >= QUICK_THRESH && s.fast.is_none() {
                s.fast = Self::specialize(opcode, ta, tb);
            }
        } else {
            // Preserve `inst`, its lifecycle is independent of scalar specialisation.
            s.type_key = key;
            s.hits = 1;
            s.fast = None;
        }
    }

    #[inline]
    pub fn get_fast(&self, ip: usize) -> Option<FastOp> {
        self.slots.get(ip).and_then(|s| s.fast)
    }

    pub fn invalidate(&mut self, ip: usize) {
        // Preserve `inst` so the instance-dunder cache survives a scalar specialisation miss at the same site.
        if let Some(s) = self.slots.get_mut(ip) {
            s.type_key = 0;
            s.hits = 0;
            s.fast = None;
        }
    }

    /* Monomorphic instance-dunder hit counter, promotes after `QUICK_THRESH` consecutive hits with the same class + method pair. Polymorphic sites churn (`record_inst` overwrites on mismatch) but never wedge. */
    pub fn record_inst(&mut self, ip: usize, class: u32, method: Val, arity: u8) {
        let Some(s) = self.slots.get_mut(ip) else { return };
        match s.inst.as_mut() {
            Some(c) if c.class == class && c.method_bits == method.0 && c.arity == arity => {
                c.hits = c.hits.saturating_add(1);
                if c.hits >= QUICK_THRESH { c.promoted = true; }
            }
            _ => {
                s.inst = Some(InstanceCache {
                    class,
                    method_bits: method.0,
                    arity,
                    hits: 1,
                    promoted: false,
                });
            }
        }
    }

    #[inline]
    pub fn get_inst(&self, ip: usize) -> Option<InstanceCache> {
        self.slots.get(ip).and_then(|s| s.inst).filter(|c| c.promoted)
    }

    pub fn invalidate_inst(&mut self, ip: usize) {
        if let Some(s) = self.slots.get_mut(ip) { s.inst = None; }
    }

    /* GC root iterator for `InstanceCache` entries: yields the cached method Val and class Val so the collector keeps both alive while the cache holds them. */
    pub fn inst_roots(&self) -> impl Iterator<Item = Val> + '_ {
        self.slots.iter().filter_map(|s| s.inst).flat_map(|c| {
            // SAFETY: `method_bits` was recorded from a live `Val`; class Val is reconstructed from the stored heap idx.
            let method = unsafe { Val::from_raw(c.method_bits) };
            let class = Val::heap(c.class);
            [method, class].into_iter()
        })
    }

    fn specialize(opcode: &OpCode, ta: u8, tb: u8) -> Option<FastOp> {
        match (opcode, ta, tb) {
            (OpCode::Add, 1, 1) => Some(FastOp::AddInt), (OpCode::Add, 2, 2) => Some(FastOp::AddFloat),
            (OpCode::Add, 5, 5) => Some(FastOp::AddStr), (OpCode::Sub, 1, 1) => Some(FastOp::SubInt),
            (OpCode::Sub, 2, 2) => Some(FastOp::SubFloat), (OpCode::Mul, 1, 1) => Some(FastOp::MulInt),
            (OpCode::Mul, 2, 2) => Some(FastOp::MulFloat), (OpCode::Lt, 1, 1) => Some(FastOp::LtInt),
            (OpCode::Lt, 2, 2) => Some(FastOp::LtFloat), (OpCode::Eq, 1, 1) => Some(FastOp::EqInt),
            (OpCode::Eq, 5, 5) => Some(FastOp::EqStr), (OpCode::Gt, 1, 1) => Some(FastOp::GtInt),
            (OpCode::LtEq, 1, 1) => Some(FastOp::LtEqInt), (OpCode::GtEq, 1, 1) => Some(FastOp::GtEqInt),
            (OpCode::NotEq, 1, 1) => Some(FastOp::NotEqInt),
            (OpCode::Mod, 1, 1) => Some(FastOp::ModInt),
            (OpCode::FloorDiv, 1, 1) => Some(FastOp::FloorDivInt),
            _ => None,
        }
    }
}

// Template memoization for pure functions.

fn args_match(e: &TplEntry, args: &[Val], h: u64, heap: &super::types::HeapPool) -> bool {
    e.hash == h
    && e.args.len() == args.len()
    && e.args.iter().zip(args).all(|(a, b)| eq_vals_with_heap(*a, *b, heap))
}

const TPL_THRESH: u32 = 2;

struct TplEntry { args: Vec<Val>, result: Val, hits: u32, hash: u64 }

fn hash_args(args: &[Val]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in args {
        h ^= v.0;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/* Memoize only when every arg is immutable; mutable containers hash by heap idx and go stale. */
fn args_memoizable(args: &[Val], heap: &super::types::HeapPool) -> bool {
    use super::types::HeapObj;
    args.iter().all(|v| {
        if !v.is_heap() { return true; }
        // Post-call args aren't rooted, so the body may have freed one; a freed slot (None) is not memoizable, nor are mutable containers.
        match heap.try_get(*v) {
            Some(o) => !matches!(o, HeapObj::List(_) | HeapObj::Dict(_) | HeapObj::Set(_) | HeapObj::Instance(..)),
            None => false,
        }
    })
}

/* Memoize only immediates (int/float/bool/None) and immutable heap objects; fresh mutable containers or tuples/sets wrapping them must stay per-call to avoid aliasing and falsifying `is`. */
fn result_memoizable(result: Val, heap: &super::types::HeapPool) -> bool {
    use super::types::HeapObj;
    if !result.is_heap() { return true; }
    matches!(heap.try_get(result), Some(HeapObj::Str(_) | HeapObj::Bytes(_) | HeapObj::LongInt(_)))
}

/* Disable a fi's memo after this many consecutive lookup misses; the scan tax outweighs stale hope. */
const MISS_LIMIT: u16 = 256;

// Indexed by dense `fi`; Vec gives O(1) lookup with no HashMap monomorphization.
pub struct Templates { slots: Vec<Vec<TplEntry>>, misses: Vec<u16> }

impl Templates {
    pub fn new() -> Self { Self { slots: Vec::new(), misses: Vec::new() } }

    fn dead(&self, fi: usize) -> bool {
        self.misses.get(fi).copied().unwrap_or(0) >= MISS_LIMIT
    }

    pub fn lookup(&mut self, fi: usize, args: &[Val], heap: &super::types::HeapPool) -> Option<Val> {
        let entries = self.slots.get(fi)?;
        if entries.is_empty() || self.dead(fi) { return None; }
        let h = hash_args(args);
        let hit = entries.iter()
            .find(|e| e.hits >= TPL_THRESH && args_match(e, args, h, heap))
            .map(|e| e.result);
        if self.misses.len() <= fi { self.misses.resize(fi + 1, 0); }
        match hit {
            Some(_) => self.misses[fi] = 0,
            None => {
                self.misses[fi] = self.misses[fi].saturating_add(1);
                // Reclaim the dead table: entries would otherwise stay GC roots forever.
                if self.misses[fi] >= MISS_LIMIT { self.slots[fi] = Vec::new(); }
            }
        }
        hit
    }

    pub fn record(&mut self, fi: usize, args: &[Val], result: Val, heap: &super::types::HeapPool) {
        if self.dead(fi) { return; }
        if !args_memoizable(args, heap) || !result_memoizable(result, heap) { return; }
        if self.slots.len() <= fi { self.slots.resize_with(fi + 1, Vec::new); }
        let h = hash_args(args);
        let v = &mut self.slots[fi];
        if let Some(e) = v.iter_mut().find(|e| args_match(e, args, h, heap)) {
            e.hits += 1; e.result = result;
        } else if v.len() < 256 {
            v.push(TplEntry { args: args.to_vec(), result, hits: 1, hash: h });
        }
    }

    pub fn mark_all(&self, heap: &mut super::types::HeapPool) {
        for slot in &self.slots {
            for e in slot {
                for &v in &e.args { heap.mark(v); }
                heap.mark(e.result);
            }
        }
    }
}

/* Fuse LoadAttr + [single-push arg loads] + Call into CallMethod+CallMethodArgs. Arg loads shift left one slot so the pair sits adjacent at the Call; only pure single-push opcodes relocate, and never across a jump target. */
fn fuse_method_calls(chunk: &SSAChunk) -> Vec<Instruction> {
    let src = &chunk.instructions;
    let n = src.len();
    let mut out = src.clone();

    // Instruction indices any jump/handler/unwind can enter; relocation across them is unsafe.
    let mut targeted = vec![false; n + 1];
    for (k, ins) in src.iter().enumerate() {
        match ins.opcode {
            OpCode::Jump | OpCode::JumpIfFalse | OpCode::JumpIfTrueOrPop | OpCode::JumpIfFalseOrPop
            | OpCode::ForIter | OpCode::SetupExcept | OpCode::SetupFinally => {
                let t = ins.operand as usize;
                if t <= n { targeted[t] = true; }
            }
            // Unwind::Goto resumes at the instruction after UnwindFinally.
            OpCode::UnwindFinally => targeted[k + 1] = true,
            _ => {}
        }
    }

    const MAX_WINDOW: usize = 8;
    let mut i = 0;
    while i + 1 < n {
        if src[i].opcode != OpCode::LoadAttr { i += 1; continue; }
        // Scan the run of relocatable single-push arg loads after the LoadAttr.
        let mut j = i + 1;
        while j < n
            && j - i - 1 < MAX_WINDOW
            && !targeted[j]
            && matches!(src[j].opcode, OpCode::LoadConst | OpCode::LoadName | OpCode::LoadTrue | OpCode::LoadFalse | OpCode::LoadNone)
        {
            j += 1;
        }
        if j >= n || src[j].opcode != OpCode::Call || targeted[j] { i += 1; continue; }
        // Every arg must be exactly one whitelisted push, else stack layout breaks.
        let raw = src[j].operand as usize;
        if (raw & 0xFF) + 2 * ((raw >> 8) & 0xFF) != j - i - 1 { i += 1; continue; }
        out[i..(j - 1)].copy_from_slice(&src[(i + 1)..j]);
        out[j - 1] = Instruction { opcode: OpCode::CallMethod, operand: src[i].operand };
        out[j] = Instruction { opcode: OpCode::CallMethodArgs, operand: src[j].operand };
        i = j + 1;
    }
    out
}
