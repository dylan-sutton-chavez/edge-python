use crate::s;
use alloc::{string::{String, ToString}, vec, vec::Vec};

use super::VM;
use super::types::*;
use super::builtins::sequence::range_int;

impl<'a> VM<'a> {

    // Interned keyword name behind a Val, or None when not a string.
    pub(crate) fn kw_name(&self, k: Val) -> Option<&str> {
        match self.heap.try_get(k) {
            Some(HeapObj::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /* Byte offset of the last propagating error, or None on success / before `run()`. */
    pub fn error_pos(&self) -> Option<usize> { self.error_byte_pos.map(|p| p as usize) }

    /* Intended process exit code when the last uncaught error is `SystemExit` with an integer (or absent/None) argument. `None` means "not a plain SystemExit", so the host renders a normal traceback; a non-int argument also yields `None` so its message surfaces as an error. */
    pub fn system_exit_code(&self) -> Option<i64> {
        let exc = self.pending.exc_val?;
        let HeapObj::ExcInstance(name, args) = self.heap.get(exc) else { return None; };
        if name != "SystemExit" { return None; }
        match args.first() {
            None => Some(0),
            Some(a) if a.is_none() => Some(0),
            Some(a) if a.is_int() => Some(a.as_int()),
            _ => None,
        }
    }

    pub fn call_stack_frames(&self) -> &[CallFrame] { &self.call_stack }
    pub fn function_names_ref(&self) -> &[String] { &self.function_names }

    /// Host-provided wall clock (ns); without one, `sleep` advances a deterministic virtual clock.
    pub fn set_time_hook(&mut self, hook: fn() -> u64) { self.time_hook = Some(hook); }
    pub(crate) fn now_ns(&self) -> u64 {
        match self.time_hook { Some(h) => h(), None => self.virtual_clock_ns }
    }

    pub fn heap_usage(&self) -> usize { self.heap.usage() }
    pub fn cache_stats(&self) -> (usize, usize) { (self.templates.count(), self.chunk.instructions.len()) }

    // Stack helpers.

    #[inline] pub(crate) fn push(&mut self, v: Val) { self.stack.push(v); }

    #[inline] pub(crate) fn pop(&mut self) -> Result<Val, VmErr> {
        self.stack.pop().ok_or(cold_runtime("stack underflow"))
    }
    #[inline] pub(crate) fn pop2(&mut self) -> Result<(Val, Val), VmErr> {
        let b = self.pop()?; let a = self.pop()?; Ok((a, b))
    }
    #[inline] pub(crate) fn pop_n(&mut self, n: usize) -> Result<Vec<Val>, VmErr> {
        let at = self.stack.len().checked_sub(n).ok_or(cold_runtime("stack underflow"))?;
        Ok(self.stack.split_off(at))
    }

    /* Materialise an iterable into Vec<Val> for `*args` positional spread. */
    pub(crate) fn iter_to_vec_for_spread(&mut self, v: Val) -> Result<Vec<Val>, VmErr> {
        if !v.is_heap() { return Err(VmErr::Type("argument after * must be an iterable")); }
        Ok(match self.heap.get(v) {
            HeapObj::List(rc) => rc.borrow().clone(),
            HeapObj::Tuple(t) => t.clone(),
            HeapObj::Set(rc) => rc.borrow().iter().cloned().collect(),
            HeapObj::Range(s, e, st) => {
                let (s, e, st) = (*s, *e, *st);
                if st == 0 { return Err(VmErr::Value("range() arg 3 must not be zero")); }
                // Spreading a huge range would build a giant arg vec; cap against the heap budget.
                let count = (e as i128 - s as i128).unsigned_abs() / (st as i128).unsigned_abs();
                if count > self.heap.limit() as u128 { return Err(VmErr::Heap); }
                let mut out = Vec::new();
                let mut i = s;
                // range_int promotes past the 47-bit inline range; checked_add ends at the i64 edge.
                if st > 0 { while i < e { out.push(range_int(&mut self.heap, i)?); match i.checked_add(st) { Some(n) => i = n, None => break } } }
                else { while i > e { out.push(range_int(&mut self.heap, i)?); match i.checked_add(st) { Some(n) => i = n, None => break } } }
                out
            }
            _ => return Err(VmErr::Type("argument after * must be an iterable")),
        })
    }

    /* Materialise a mapping into (key_str, value) pairs for `**kwargs` spread. */
    pub(crate) fn mapping_to_kw_pairs(&self, v: Val) -> Result<Vec<(Val, Val)>, VmErr> {
        if !v.is_heap() {
            return Err(VmErr::Type("argument after ** must be a mapping"));
        }
        match self.heap.get(v) {
            HeapObj::Dict(rc) => {
                let entries: Vec<(Val, Val)> = rc.borrow().iter().collect();
                for (k, _) in &entries {
                    if !k.is_heap() || !matches!(self.heap.get(*k), HeapObj::Str(_)) {
                        return Err(VmErr::Type("keywords must be strings"));
                    }
                }
                Ok(entries)
            }
            _ => Err(VmErr::Type("argument after ** must be a mapping")),
        }
    }

    /* Seed slots with `undef()` so LoadName can detect unbound names via a u64 compare. */
    pub(crate) fn fill_builtins(&self, names: &[String]) -> Vec<Val> {
        let mut slots = vec![Val::undef(); names.len()];
        for (i, name) in names.iter().enumerate() {
            if let Some(v) = self.globals.get(name) {
                slots[i] = *v;
            }
        }
        slots
    }

    #[inline]
    pub(crate) fn checked_jump(&mut self, target: usize, limit: usize) -> Result<usize, VmErr> {
        // Sandbox-off skips the budget decrement; the bounds check still runs.
        if !self.sandbox_off {
            if self.budget == 0 { return Err(cold_budget()); }
            self.budget -= 1;
        }
        if target > limit { return Err(cold_runtime("jump target out of bounds")); }
        Ok(target)
    }

    pub(crate) fn str_to_char_vals(&mut self, s: &str) -> Result<Vec<Val>, VmErr> {
        // Per-char heap allocs scale with input; charge the budget so loops over this stay bounded.
        self.charge_steps(s.len())?;
        s.chars().map(|c| self.heap.alloc(HeapObj::Str(c.to_string()))).collect()
    }

    pub(crate) fn make_iter_frame(&mut self, obj: Val, chunk: &crate::modules::parser::SSAChunk, slots: &mut [Val]) -> Result<IterFrame, VmErr> {
        if !obj.is_heap() {
            return Err(VmErr::TypeMsg(s!("'", str self.type_name(obj), "' object is not iterable")));
        }
        // Instance `__iter__` produces a user-defined iterator that drives `ForIter` via `__next__`.
        if matches!(self.heap.get(obj), HeapObj::Instance(..))
            && let Some(iter) = self.try_call_dunder(obj, "__iter__", &[], chunk, slots)? {
            return Ok(IterFrame::UserDefined(iter));
        }
        Ok(match self.heap.get(obj) {
            HeapObj::Range(s, e, st) => IterFrame::Range { cur: *s, end: *e, step: *st },
            HeapObj::List(v) => IterFrame::Seq { items: v.borrow().clone(), idx: 0 },
            HeapObj::Tuple(v) => IterFrame::Seq { items: v.clone(), idx: 0 },
            HeapObj::Dict(p) => IterFrame::Seq { items: p.borrow().keys().collect(), idx: 0 },
            HeapObj::Set(s) => {
                let items: Vec<Val> = s.borrow().iter().cloned().collect();
                IterFrame::Seq { items, idx: 0 }
            },
            HeapObj::FrozenSet(s) => {
                let items: Vec<Val> = s.iter().cloned().collect();
                IterFrame::Seq { items, idx: 0 }
            },
            HeapObj::Str(s) => {
                let s = s.clone();
                let items = self.str_to_char_vals(&s)?;
                IterFrame::Seq { items, idx: 0 }
            },
            HeapObj::Bytes(b) => {
                // Bytes iteration yields ints, matching `iter()` and indexing.
                let items: Vec<Val> = b.iter().map(|&byte| Val::int(byte as i64)).collect();
                IterFrame::Seq { items, idx: 0 }
            },
            HeapObj::Coroutine(..) => return Ok(IterFrame::Coroutine(obj)),
            _ => return Err(VmErr::TypeMsg(s!("'", str self.type_name(obj), "' object is not iterable"))),
        })
    }

    pub(crate) fn exec_unpack_seq(&mut self, expected: usize) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(cold_type("cannot unpack non-sequence")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v) => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            HeapObj::Set(v) => v.borrow().iter().cloned().collect(),
            HeapObj::FrozenSet(v) => v.iter().cloned().collect(),
            // Range materialises to its ints, with the same budget cap as `*` spread.
            HeapObj::Range(..) => self.iter_to_vec_for_spread(obj)?,
            HeapObj::Str(s) => {
                let s = s.clone();
                let out = self.str_to_char_vals(&s)?;
                if out.len() > expected {
                    return Err(cold_value("too many values to unpack"));
                } else if out.len() < expected {
                    return Err(cold_value("not enough values to unpack"));
                }
                out
            },
            _ => return Err(cold_type("cannot unpack non-sequence")),
        };
        if items.len() > expected {
            return Err(cold_value("too many values to unpack"));
        } else if items.len() < expected {
            return Err(cold_value("not enough values to unpack"));
        }
        for item in items.into_iter().rev() { self.push(item); }
        Ok(())
    }

    /* Pick the first defined Phi source; if both are undef fall back to None. */
    pub(crate) fn exec_phi(op: u16, rip: usize, phi_map: &[usize], slots: &mut [Val], phi_sources: &[(u16, u16)]) {
        // Parse recovery can leave a Phi indexing past `slots` (sized to names.len()); index defensively.
        let Some(&(ia, ib)) = phi_map.get(rip).and_then(|&pi| phi_sources.get(pi)) else { return };
        let a = slots.get(ia as usize).copied().unwrap_or_else(Val::undef);
        let val = if !a.is_undef() { a }
        else { let b = slots.get(ib as usize).copied().unwrap_or_else(Val::undef); if !b.is_undef() { b } else { Val::none() } };
        if let Some(dst) = slots.get_mut(op as usize) { *dst = val; }
    }
}
