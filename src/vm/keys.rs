use super::VM;
use super::types::*;
use crate::parser::SSAChunk;
use crate::parser::types::OpCode;

use alloc::{rc::Rc, vec::Vec};
use core::cell::RefCell;

/* A set the VM probes, the frozen one never changes under a probe. */
enum Table { Set(Rc<RefCell<ValSet>>), Frozen(Rc<ValSet>) }

impl Table {
    fn candidates(&self, h: u64) -> Vec<Val> {
        match self { Table::Set(rc) => rc.borrow().candidates(h), Table::Frozen(s) => s.candidates(h) }
    }
    fn is_rich(&self) -> bool {
        match self { Table::Set(rc) => rc.borrow().is_rich(), Table::Frozen(s) => s.is_rich() }
    }
}

impl<'a> VM<'a> {
    /* Hash of key `k`, a user `__hash__` runs for it and for the items of a tuple holding one. */
    pub(crate) fn key_hash(&mut self, k: Val, chunk: &SSAChunk) -> Result<u64, VmErr> {
        self.require_hashable(k)?;
        if !is_rich_key(k, &self.heap) { return Ok(hash_val_with_heap(k, &self.heap)); }
        let items = match self.heap.get(k) {
            HeapObj::Tuple(items) => items.clone(),
            HeapObj::FrozenSet(s) => return Ok(hash_set_parts(s.iter_hashed().map(|(h, _)| h))),
            _ => {
                // The int `hash(k)` gives hashes like the same int key, so `A(3)` and `3` share a probe.
                self.push(k);
                self.call_hash(chunk)?;
                let n = self.pop()?;
                return Ok(hash_val_with_heap(n, &self.heap));
            }
        };
        let mut parts = Vec::with_capacity(items.len());
        for e in items { parts.push(self.key_hash(e, chunk)?); }
        Ok(hash_tuple_parts(&parts))
    }

    /* Hash a set probe takes, a set looks up as the frozenset it equals. */
    fn probe_hash(&mut self, k: Val, chunk: &SSAChunk) -> Result<u64, VmErr> {
        if matches!(self.heap.try_get(k), Some(HeapObj::Set(_))) { return Ok(hash_val_with_heap(k, &self.heap)); }
        self.key_hash(k, chunk)
    }

    /* True when `d` is a dict whose lookup of `k` has to run user code. */
    #[inline]
    pub(crate) fn dict_needs_vm(&self, d: Val, k: Val) -> bool {
        d.is_heap() && matches!(self.heap.get(d), HeapObj::Dict(rc) if rc.borrow().is_rich() || is_rich_key(k, &self.heap))
    }

    /* `a == b`, the content compare first and the user dunders only where it cannot settle. */
    pub(crate) fn values_eq(&mut self, a: Val, b: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        match eq_checked(a, b, &self.heap) {
            Some(r) => Ok(r),
            None => self.with_roots([a, b], |vm| vm.rich_eq(a, b, chunk, 0)),
        }
    }

    /* Equality of a container member, identity first as in `x is e or x == e`. */
    pub(crate) fn member_eq(&mut self, a: Val, b: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        if a.0 == b.0 { return Ok(true); }
        self.values_eq(a, b, chunk)
    }

    fn rich_eq(&mut self, a: Val, b: Val, chunk: &SSAChunk, depth: usize) -> Result<bool, VmErr> {
        if depth > EQ_DEPTH_MAX { return Ok(a.0 == b.0); }
        let inst = |vm: &Self, v: Val| matches!(vm.heap.try_get(v), Some(HeapObj::Instance(..)));
        if inst(self, a) || inst(self, b) {
            return Ok(match self.try_compare_dunder(OpCode::Eq, a, b, chunk)? { Some(r) => self.truthy(r), None => a.0 == b.0 });
        }
        let seqs = match (self.heap.try_get(a), self.heap.try_get(b)) {
            (Some(HeapObj::List(x)), Some(HeapObj::List(y))) => Some((x.borrow().clone(), y.borrow().clone())),
            (Some(HeapObj::Tuple(x)), Some(HeapObj::Tuple(y))) => Some((x.clone(), y.clone())),
            _ => None,
        };
        if let Some((xs, ys)) = seqs {
            if xs.len() != ys.len() { return Ok(false); }
            // A `__eq__` can shrink either list, so the items it walks stay rooted.
            return self.with_roots(xs.iter().chain(&ys).copied(), |vm| {
                for (&x, &y) in xs.iter().zip(&ys) {
                    if !vm.member_eq_d(x, y, chunk, depth + 1)? { return Ok(false); }
                }
                Ok(true)
            });
        }
        if let (Some(HeapObj::Dict(x)), Some(HeapObj::Dict(_))) = (self.heap.try_get(a), self.heap.try_get(b)) {
            let entries: Vec<(Val, Val)> = x.borrow().iter().collect();
            if entries.len() != self.dict_len(b) { return Ok(false); }
            return self.with_roots(entries.iter().flat_map(|&(k, v)| [k, v]), |vm| {
                for &(k, v) in &entries {
                    let Some(w) = vm.dict_get(b, k, chunk)? else { return Ok(false) };
                    if !vm.member_eq_d(v, w, chunk, depth + 1)? { return Ok(false); }
                }
                Ok(true)
            });
        }
        if self.is_set_like(a) && self.is_set_like(b) {
            let items: Vec<Val> = self.clone_set_items(a).map(|s| s.iter().copied().collect()).unwrap_or_default();
            if items.len() != self.clone_set_items(b).map_or(0, |s| s.len()) { return Ok(false); }
            return self.with_roots(items.clone(), |vm| {
                for &v in &items { if !vm.set_has(b, v, chunk)? { return Ok(false); } }
                Ok(true)
            });
        }
        Ok(eq_vals_with_heap(a, b, &self.heap))
    }

    fn member_eq_d(&mut self, a: Val, b: Val, chunk: &SSAChunk, depth: usize) -> Result<bool, VmErr> {
        if a.0 == b.0 { return Ok(true); }
        match eq_checked(a, b, &self.heap) { Some(r) => Ok(r), None => self.rich_eq(a, b, chunk, depth) }
    }

    /* `a < b`, sequences holding instances compare their items through the user dunders. */
    pub(crate) fn values_lt(&mut self, a: Val, b: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        match self.lt_vals(a, b) {
            Err(VmErr::TypeMsg(_)) if self.is_sequence(a) && self.is_sequence(b) => self.with_roots([a, b], |vm| vm.rich_lt(a, b, chunk, 0)),
            r => r,
        }
    }

    fn is_sequence(&self, v: Val) -> bool { matches!(self.heap.try_get(v), Some(HeapObj::List(_) | HeapObj::Tuple(_))) }

    fn rich_lt(&mut self, a: Val, b: Val, chunk: &SSAChunk, depth: usize) -> Result<bool, VmErr> {
        if depth > EQ_DEPTH_MAX { return Err(cold_depth()); }
        if let Some(r) = self.try_compare_dunder(OpCode::Lt, a, b, chunk)? { return Ok(self.truthy(r)); }
        let seqs = match (self.heap.try_get(a), self.heap.try_get(b)) {
            (Some(HeapObj::List(x)), Some(HeapObj::List(y))) => (x.borrow().clone(), y.borrow().clone()),
            (Some(HeapObj::Tuple(x)), Some(HeapObj::Tuple(y))) => (x.clone(), y.clone()),
            _ => return self.lt_vals(a, b),
        };
        let (xs, ys) = seqs;
        // The first pair that differs decides, as for the builtin sequences.
        self.with_roots(xs.iter().chain(&ys).copied(), |vm| {
            for (&x, &y) in xs.iter().zip(&ys) {
                if vm.member_eq_d(x, y, chunk, depth + 1)? { continue; }
                return vm.rich_lt(x, y, chunk, depth + 1);
            }
            Ok(xs.len() < ys.len())
        })
    }

    /* The candidate equal to `k`, identity first and the user `__eq__` for the rest. */
    fn matching(&mut self, cands: &[Val], k: Val, chunk: &SSAChunk) -> Result<Option<Val>, VmErr> {
        if let Some(&c) = cands.iter().find(|c| c.0 == k.0) { return Ok(Some(c)); }
        for &c in cands {
            if self.values_eq(c, k, chunk)? { return Ok(Some(c)); }
        }
        Ok(None)
    }

    fn dict_rc(&self, d: Val) -> Option<Rc<RefCell<DictMap>>> {
        match self.heap.try_get(d) { Some(HeapObj::Dict(rc)) => Some(rc.clone()), _ => None }
    }

    fn dict_len(&self, d: Val) -> usize { self.dict_rc(d).map_or(0, |rc| rc.borrow().len()) }

    /* True when a probe of `table` for `k` has to run user code. */
    fn rich_probe(&self, table_rich: bool, k: Val) -> bool { table_rich || is_rich_key(k, &self.heap) }

    /* Entry of `k` in the dict, a `__eq__` that reshaped the dict sends the probe round again. */
    fn dict_slot(&mut self, rc: &Rc<RefCell<DictMap>>, k: Val, h: u64, chunk: &SSAChunk) -> Result<Option<usize>, VmErr> {
        loop {
            let cands: Vec<(usize, Val)> = { let m = rc.borrow(); m.candidates(h).into_iter().map(|i| (i, m.key_at(i))).collect() };
            let keys: Vec<Val> = cands.iter().map(|c| c.1).collect();
            let Some(hit) = self.matching(&keys, k, chunk)? else { return Ok(None) };
            let i = cands.iter().find(|c| c.1.0 == hit.0).map_or(0, |c| c.0);
            if rc.borrow().key_at(i).0 == hit.0 { return Ok(Some(i)); }
            self.charge_step()?;
        }
    }

    /* `d[k]` lookup, None on a miss. */
    pub(crate) fn dict_get(&mut self, d: Val, k: Val, chunk: &SSAChunk) -> Result<Option<Val>, VmErr> {
        match self.heap.try_get(d) {
            Some(HeapObj::Dict(rc)) if !self.rich_probe(rc.borrow().is_rich(), k) => return Ok(rc.borrow().get(&k, &self.heap).copied()),
            Some(HeapObj::Dict(_)) => {}
            _ => return Ok(None),
        }
        let Some(rc) = self.dict_rc(d) else { return Ok(None) };
        self.with_roots([d, k], |vm| {
            let h = vm.key_hash(k, chunk)?;
            Ok(vm.dict_slot(&rc, k, h, chunk)?.map(|i| rc.borrow().value_at(i)))
        })
    }

    /* `d[k] = v`. */
    pub(crate) fn dict_set(&mut self, d: Val, k: Val, v: Val, chunk: &SSAChunk) -> Result<(), VmErr> {
        self.dict_set_with(d, k, v, None, chunk)
    }

    /* `d[k] = v` with the hash `k` already has when it comes from another dict or set. */
    fn dict_set_with(&mut self, d: Val, k: Val, v: Val, known: Option<u64>, chunk: &SSAChunk) -> Result<(), VmErr> {
        match self.heap.try_get(d) {
            Some(HeapObj::Dict(rc)) if !self.rich_probe(rc.borrow().is_rich(), k) => {
                self.require_hashable(k)?;
                self.heap.growing(&mut *rc.borrow_mut(), |d| d.insert(k, v, &self.heap));
                return Ok(());
            }
            Some(HeapObj::Dict(_)) => {}
            _ => return Ok(()),
        }
        let Some(rc) = self.dict_rc(d) else { return Ok(()) };
        self.with_roots([d, k, v], |vm| {
            let h = match known { Some(h) => h, None => vm.key_hash(k, chunk)? };
            match vm.dict_slot(&rc, k, h, chunk)? {
                Some(i) => rc.borrow_mut().set_value_at(i, v),
                None => { let rich = is_rich_key(k, &vm.heap); vm.heap.growing(&mut *rc.borrow_mut(), |d| d.push_hashed(k, v, h, rich)); }
            }
            Ok(())
        })
    }

    /* `del d[k]` and `d.pop(k)`, the removed value or None on a miss. */
    pub(crate) fn dict_del(&mut self, d: Val, k: Val, chunk: &SSAChunk) -> Result<Option<Val>, VmErr> {
        match self.heap.try_get(d) {
            Some(HeapObj::Dict(rc)) if !self.rich_probe(rc.borrow().is_rich(), k) => return Ok(rc.borrow_mut().remove(&k, &self.heap)),
            Some(HeapObj::Dict(_)) => {}
            _ => return Ok(None),
        }
        let Some(rc) = self.dict_rc(d) else { return Ok(None) };
        self.with_roots([d, k], |vm| {
            let h = vm.key_hash(k, chunk)?;
            Ok(vm.dict_slot(&rc, k, h, chunk)?.map(|i| rc.borrow_mut().remove_at(i)))
        })
    }

    fn table(&self, s: Val) -> Option<Table> {
        match self.heap.try_get(s) {
            Some(HeapObj::Set(rc)) => Some(Table::Set(rc.clone())),
            Some(HeapObj::FrozenSet(rc)) => Some(Table::Frozen(rc.clone())),
            _ => None,
        }
    }

    /* The stored item equal to `k`, a `__eq__` that reshaped the set sends the probe round again. */
    fn set_slot(&mut self, t: &Table, k: Val, h: u64, chunk: &SSAChunk) -> Result<Option<Val>, VmErr> {
        loop {
            let Some(hit) = self.matching(&t.candidates(h), k, chunk)? else { return Ok(None) };
            if t.candidates(h).iter().any(|c| c.0 == hit.0) { return Ok(Some(hit)); }
            self.charge_step()?;
        }
    }

    /* `k in s` for a set or frozenset. */
    pub(crate) fn set_has(&mut self, s: Val, k: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        let fast = match self.heap.try_get(s) {
            Some(HeapObj::Set(rc)) if !self.rich_probe(rc.borrow().is_rich(), k) => Some(rc.borrow().contains(k, &self.heap)),
            Some(HeapObj::FrozenSet(fs)) if !self.rich_probe(fs.is_rich(), k) => Some(fs.contains(k, &self.heap)),
            _ => None,
        };
        if let Some(hit) = fast {
            if !hit { self.require_set_probe(k)?; }
            return Ok(hit);
        }
        let Some(t) = self.table(s) else { return Ok(false) };
        self.with_roots([s, k], |vm| {
            let h = vm.probe_hash(k, chunk)?;
            Ok(vm.set_slot(&t, k, h, chunk)?.is_some())
        })
    }

    /* `s.add(k)`, true when `k` was new. */
    pub(crate) fn set_add(&mut self, s: Val, k: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        self.set_add_with(s, k, None, chunk)
    }

    /* `s.add(k)` with the hash `k` already has when it comes from another set. */
    pub(crate) fn set_add_with(&mut self, s: Val, k: Val, known: Option<u64>, chunk: &SSAChunk) -> Result<bool, VmErr> {
        if let Some(HeapObj::Set(rc)) = self.heap.try_get(s) && !self.rich_probe(rc.borrow().is_rich(), k) {
            self.require_hashable(k)?;
            return Ok(self.heap.growing(&mut *rc.borrow_mut(), |t| t.insert(k, &self.heap)));
        }
        let Some(t @ Table::Set(_)) = self.table(s) else { return Ok(false) };
        self.with_roots([s, k], |vm| {
            let h = match known { Some(h) => h, None => vm.key_hash(k, chunk)? };
            if vm.set_slot(&t, k, h, chunk)?.is_some() { return Ok(false); }
            let rich = is_rich_key(k, &vm.heap);
            if let Table::Set(rc) = &t { vm.heap.growing(&mut *rc.borrow_mut(), |t| t.push_hashed(k, h, rich)); }
            Ok(true)
        })
    }

    /* `s.discard(k)`, true when `k` was there. */
    pub(crate) fn set_discard(&mut self, s: Val, k: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        if let Some(HeapObj::Set(rc)) = self.heap.try_get(s) && !self.rich_probe(rc.borrow().is_rich(), k) {
            let hit = rc.borrow_mut().remove(k, &self.heap);
            if !hit { self.require_set_probe(k)?; }
            return Ok(hit);
        }
        let Some(t @ Table::Set(_)) = self.table(s) else { return Ok(false) };
        self.with_roots([s, k], |vm| {
            let h = vm.probe_hash(k, chunk)?;
            let Some(hit) = vm.set_slot(&t, k, h, chunk)? else { return Ok(false) };
            Ok(match &t { Table::Set(rc) => rc.borrow_mut().remove_exact(hit, h), Table::Frozen(_) => false })
        })
    }

    /* A set of `items`, user-hashed ones placed by their own `__hash__` and `__eq__`. */
    pub(crate) fn valset_of(&mut self, items: &[Val], chunk: &SSAChunk) -> Result<ValSet, VmErr> {
        if !items.iter().any(|&v| is_rich_key(v, &self.heap)) {
            for &v in items.iter().filter(|v| v.is_heap()) { self.require_hashable(v)?; }
            return Ok(ValSet::from_vals(items, &self.heap));
        }
        self.with_roots(items.iter().copied(), |vm| {
            let mut s = ValSet::with_capacity(items.len());
            for &v in items {
                let h = vm.key_hash(v, chunk)?;
                if vm.matching(&s.candidates(h), v, chunk)?.is_none() {
                    let rich = is_rich_key(v, &vm.heap);
                    s.push_hashed(v, h, rich);
                }
            }
            Ok(s)
        })
    }

    /* A dict of `pairs` in order, a later equal key overwriting the value and keeping the first key. */
    pub(crate) fn dictmap_of(&mut self, pairs: Vec<(Val, Val)>, chunk: &SSAChunk) -> Result<DictMap, VmErr> {
        if !pairs.iter().any(|&(k, _)| is_rich_key(k, &self.heap)) {
            for &(k, _) in pairs.iter().filter(|p| p.0.is_heap()) { self.require_hashable(k)?; }
            return Ok(DictMap::from_pairs(pairs, &self.heap));
        }
        self.with_roots(pairs.iter().flat_map(|&(k, v)| [k, v]), |vm| {
            let mut m = DictMap::with_capacity(pairs.len());
            for &(k, v) in &pairs {
                let h = vm.key_hash(k, chunk)?;
                let cands: Vec<(usize, Val)> = m.candidates(h).into_iter().map(|i| (i, m.key_at(i))).collect();
                let keys: Vec<Val> = cands.iter().map(|c| c.1).collect();
                match vm.matching(&keys, k, chunk)? {
                    Some(hit) => { let i = cands.iter().find(|c| c.1.0 == hit.0).map_or(0, |c| c.0); m.set_value_at(i, v); }
                    None => { let rich = is_rich_key(k, &vm.heap); m.push_hashed(k, v, h, rich); }
                }
            }
            Ok(m)
        })
    }

    /* `a op b` for sets holding user-hashed items, probing with the hashes each set stored, `inplace` rewrites a mutable `a`. */
    pub(crate) fn rich_set_op(&mut self, a: Val, b: Val, op: OpCode, inplace: bool, chunk: &SSAChunk) -> Result<(), VmErr> {
        let (Some(ta), Some(tb)) = (self.clone_set_items(a), self.clone_set_items(b)) else { return Err(cold_runtime("set op on non-set operands")) };
        let roots: Vec<Val> = ta.iter().chain(tb.iter()).copied().chain([a, b]).collect();
        let out = self.with_roots(roots, |vm| -> Result<ValSet, VmErr> {
            let mut out = if op == OpCode::BitOr { ta.clone() } else { ValSet::with_capacity(ta.len()) };
            let xs: Vec<(u64, Val)> = ta.iter_hashed().collect();
            let ys: Vec<(u64, Val)> = tb.iter_hashed().collect();
            match op {
                OpCode::BitOr => for &(h, y) in &ys { vm.vs_insert(&mut out, h, y, chunk)?; },
                OpCode::BitAnd | OpCode::Sub => for &(h, x) in &xs {
                    if vm.vs_has(&tb, h, x, chunk)? == (op == OpCode::BitAnd) { out.push_hashed(x, h, is_rich_key(x, &vm.heap)); }
                },
                _ => {
                    for &(h, x) in &xs { if !vm.vs_has(&tb, h, x, chunk)? { out.push_hashed(x, h, is_rich_key(x, &vm.heap)); } }
                    for &(h, y) in &ys { if !vm.vs_has(&ta, h, y, chunk)? { out.push_hashed(y, h, is_rich_key(y, &vm.heap)); } }
                }
            }
            Ok(out)
        })?;
        if inplace && let Some(HeapObj::Set(rc)) = self.heap.try_get(a) {
            self.heap.growing(&mut *rc.borrow_mut(), |t| *t = out);
            self.push(a);
            return Ok(());
        }
        let v = if matches!(self.heap.try_get(a), Some(HeapObj::FrozenSet(_))) { self.heap.alloc(HeapObj::FrozenSet(Rc::new(out)))? }
            else { self.heap.alloc(HeapObj::Set(Rc::new(RefCell::new(out))))? };
        self.push(v);
        Ok(())
    }

    /* `a | b` for dicts, `b` wins and every key keeps the hash its dict stored. */
    pub(crate) fn rich_dict_merge(&mut self, a: Val, b: Val, chunk: &SSAChunk) -> Result<(), VmErr> {
        let (Some(ra), Some(rb)) = (self.dict_rc(a), self.dict_rc(b)) else { return Err(cold_runtime("dict merge on non-dict operands")) };
        let m = ra.borrow().clone();
        let out = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(m))))?;
        let entries: Vec<(Val, Val, u64)> = rb.borrow().iter_hashed().collect();
        self.with_roots([a, b, out], |vm| {
            for (k, v, h) in entries { vm.dict_set_with(out, k, v, Some(h), chunk)?; }
            Ok::<(), VmErr>(())
        })?;
        self.push(out);
        Ok(())
    }

    /* `{*src}` into set `acc`, a set source keeps the hashes it stored. */
    pub(crate) fn spread_into(&mut self, acc: Val, src: Val, chunk: &SSAChunk) -> Result<(), VmErr> {
        let hashed: Option<Vec<(u64, Val)>> = match self.heap.try_get(src) {
            Some(HeapObj::Set(rc)) => Some(rc.borrow().iter_hashed().collect()),
            Some(HeapObj::FrozenSet(rc)) => Some(rc.iter_hashed().collect()),
            _ => None,
        };
        let items: Vec<(Option<u64>, Val)> = match hashed {
            Some(h) => h.into_iter().map(|(h, v)| (Some(h), v)).collect(),
            None => {
                let plain = self.iterable_items(src, chunk)?;
                // Plain items into a plain set need no user code, so no roots either.
                if let Some(HeapObj::Set(rc)) = self.heap.try_get(acc) && !rc.borrow().is_rich() && !plain.iter().any(|&v| is_rich_key(v, &self.heap)) {
                    for &v in plain.iter().filter(|v| v.is_heap()) { self.require_hashable(v)?; }
                    self.heap.growing(&mut *rc.borrow_mut(), |s| for v in plain { s.insert(v, &self.heap); });
                    return Ok(());
                }
                plain.into_iter().map(|v| (None, v)).collect()
            }
        };
        self.with_roots(items.iter().map(|i| i.1).chain([acc, src]), |vm| {
            for &(h, v) in &items { vm.set_add_with(acc, v, h, chunk)?; }
            Ok(())
        })
    }

    /* `{**src}` merged into dict `acc`, later keys overwriting earlier values. */
    pub(crate) fn dict_spread_into(&mut self, acc: Val, src: Val, chunk: &SSAChunk) -> Result<(), VmErr> {
        let entries: Vec<(Val, Val, u64)> = match self.heap.try_get(src) {
            Some(HeapObj::Dict(rc)) => rc.borrow().iter_hashed().collect(),
            _ => return Err(cold_type("argument after ** must be a mapping")),
        };
        self.with_roots(entries.iter().flat_map(|&(k, v, _)| [k, v]).chain([acc, src]), |vm| {
            for &(k, v, h) in &entries { vm.dict_set_with(acc, k, v, Some(h), chunk)?; }
            Ok(())
        })
    }

    /* True when set algebra over `a` and `b` has to run user code. */
    pub(crate) fn sets_rich(&self, a: Val, b: Val) -> bool {
        [a, b].iter().any(|&v| self.table(v).is_some_and(|t| t.is_rich()))
    }

    /* Every item of set `a` is in set `b`. */
    pub(crate) fn set_within(&mut self, a: Val, b: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        let xs: Vec<Val> = self.clone_set_items(a).map(|s| s.iter().copied().collect()).unwrap_or_default();
        self.with_roots(xs.iter().copied().chain([a, b]), |vm| {
            for &x in &xs { if !vm.set_has(b, x, chunk)? { return Ok(false); } }
            Ok(true)
        })
    }

    /* Items of `v` with their hashes, a set or dict gives the ones it stored and anything else runs `__hash__`. */
    fn hashed_items(&mut self, v: Val, chunk: &SSAChunk) -> Result<Vec<(u64, Val)>, VmErr> {
        match self.heap.try_get(v) {
            Some(HeapObj::Set(rc)) => return Ok(rc.borrow().iter_hashed().collect()),
            Some(HeapObj::FrozenSet(rc)) => return Ok(rc.iter_hashed().collect()),
            Some(HeapObj::Dict(rc)) => return Ok(rc.borrow().iter_hashed().map(|(k, _, h)| (h, k)).collect()),
            _ => {}
        }
        let items = self.extract_iter(v)?;
        self.with_roots(items.iter().copied(), |vm| {
            let mut out = Vec::with_capacity(items.len());
            for &x in &items { out.push((vm.key_hash(x, chunk)?, x)); }
            Ok(out)
        })
    }

    /* Adds `v` stored under `h` to a set built here, unless an equal item is in. */
    fn vs_insert(&mut self, s: &mut ValSet, h: u64, v: Val, chunk: &SSAChunk) -> Result<(), VmErr> {
        if self.matching(&s.candidates(h), v, chunk)?.is_none() {
            let rich = is_rich_key(v, &self.heap);
            s.push_hashed(v, h, rich);
        }
        Ok(())
    }

    fn vs_has(&mut self, s: &ValSet, h: u64, v: Val, chunk: &SSAChunk) -> Result<bool, VmErr> {
        Ok(self.matching(&s.candidates(h), v, chunk)?.is_some())
    }

    /* A set built here from hashed items. */
    fn vs_from(&mut self, items: &[(u64, Val)], chunk: &SSAChunk) -> Result<ValSet, VmErr> {
        let mut s = ValSet::with_capacity(items.len());
        for &(h, v) in items { self.vs_insert(&mut s, h, v, chunk)?; }
        Ok(s)
    }

    /* True when a dict or set method over `recv` and `pos` meets a key that runs user code, false for any other receiver. */
    fn method_rich(&self, recv: Val, pos: &[Val]) -> bool {
        let recv_rich = match self.heap.try_get(recv) {
            Some(HeapObj::Dict(rc)) => rc.borrow().is_rich(),
            Some(HeapObj::Set(rc)) => rc.borrow().is_rich(),
            Some(HeapObj::Type(_)) => false,
            _ => return false,
        };
        let holds = |v: Val| match self.heap.try_get(v) {
            Some(HeapObj::List(rc)) => rc.borrow().iter().any(|&x| is_rich_key(x, &self.heap)),
            Some(HeapObj::Tuple(t)) => t.iter().any(|&x| is_rich_key(x, &self.heap)),
            Some(HeapObj::Set(rc)) => rc.borrow().is_rich(),
            Some(HeapObj::FrozenSet(rc)) => rc.is_rich(),
            Some(HeapObj::Dict(rc)) => rc.borrow().is_rich(),
            _ => false,
        };
        recv_rich || pos.iter().any(|&p| p.is_heap() && (is_rich_key(p, &self.heap) || holds(p)))
    }

    /* A dict or set method where user keys take part, false leaves the call to the plain method. */
    pub(crate) fn keyed_method(&mut self, id: crate::vm::methods::BuiltinMethodId, recv: Val, pos: &[Val], kw: &[Val], chunk: &SSAChunk) -> Result<bool, VmErr> {
        if !self.method_rich(recv, pos) { return Ok(false); }
        let (ty, name) = (id.ty(), id.name());
        if !matches!(ty, "dict" | "set") { return Ok(false); }
        if !kw.is_empty() && !(ty == "dict" && name == "update") { return Ok(false); }
        crate::vm::methods::method_frame(self, id, pos.len())?;
        let roots: Vec<Val> = pos.iter().chain(kw).copied().chain([recv]).collect();
        self.with_roots(roots, |vm| if ty == "dict" { vm.dict_method(name, recv, pos, kw, chunk) } else { vm.set_method(name, recv, pos, chunk) })
    }

    fn dict_method(&mut self, name: &str, recv: Val, pos: &[Val], kw: &[Val], chunk: &SSAChunk) -> Result<bool, VmErr> {
        let arg = |i: usize| pos.get(i).copied();
        let out = match name {
            "get" => self.dict_get(recv, pos[0], chunk)?.unwrap_or(arg(1).unwrap_or(Val::none())),
            "pop" => match (self.dict_del(recv, pos[0], chunk)?, arg(1)) {
                (Some(v), _) | (None, Some(v)) => v,
                (None, None) => return Err(VmErr::Raised(crate::s!("KeyError: ", str &self.repr(pos[0])))),
            },
            "setdefault" => match self.dict_get(recv, pos[0], chunk)? {
                Some(v) => v,
                None => { let d = arg(1).unwrap_or(Val::none()); self.dict_set(recv, pos[0], d, chunk)?; d }
            },
            "update" => {
                let mut pairs = Vec::new();
                for &src in pos { pairs.extend(self.pairs_of(src)?); }
                pairs.extend(kw.chunks(2).map(|p| (p[0], p[1])));
                for (k, v) in pairs { self.dict_set(recv, k, v, chunk)?; }
                Val::none()
            }
            "fromkeys" => {
                let value = arg(1).unwrap_or(Val::none());
                let keys = self.hashed_items(pos[0], chunk)?;
                let m = self.dictmap_of(keys.into_iter().map(|(_, k)| (k, value)).collect(), chunk)?;
                return self.alloc_and_push_dict(m).map(|_| true);
            }
            "copy" => {
                let m = match self.heap.try_get(recv) { Some(HeapObj::Dict(rc)) => rc.borrow().clone(), _ => return Ok(false) };
                return self.alloc_and_push_dict(m).map(|_| true);
            }
            "popitem" => {
                let Some(rc) = self.dict_rc(recv) else { return Ok(false) };
                let i = rc.borrow().last_index().ok_or_else(|| cold_key("popitem(): dictionary is empty"))?;
                let k = rc.borrow().key_at(i);
                let v = rc.borrow_mut().remove_at(i);
                return self.alloc_and_push_tuple(alloc::vec![k, v]).map(|_| true);
            }
            _ => return Ok(false),
        };
        self.push(out);
        Ok(true)
    }

    fn set_method(&mut self, name: &str, recv: Val, pos: &[Val], chunk: &SSAChunk) -> Result<bool, VmErr> {
        let Some(Table::Set(rc)) = self.table(recv) else { return Ok(false) };
        let mine: Vec<(u64, Val)> = rc.borrow().iter_hashed().collect();
        let mut args = Vec::with_capacity(pos.len());
        if !matches!(name, "add" | "remove" | "discard" | "pop" | "copy") {
            for &p in pos { args.push(self.hashed_items(p, chunk)?); }
        }
        let result = match name {
            "add" => { self.set_add(recv, pos[0], chunk)?; None }
            // A set never holds itself, and a miss on `remove` raises.
            "remove" | "discard" => {
                let hit = pos[0].0 != recv.0 && self.set_discard(recv, pos[0], chunk)?;
                if !hit && name == "remove" { return Err(VmErr::Raised("KeyError".into())); }
                None
            }
            "pop" => {
                let &(h, v) = mine.first().ok_or_else(|| cold_key("pop from an empty set"))?;
                rc.borrow_mut().remove_exact(v, h);
                self.push(v);
                return Ok(true);
            }
            "copy" => Some(rc.borrow().clone()),
            "update" | "union" => {
                let mut out = rc.borrow().clone();
                for items in &args { for &(h, v) in items { self.vs_insert(&mut out, h, v, chunk)?; } }
                if name == "union" { Some(out) } else { self.heap.growing(&mut *rc.borrow_mut(), |t| *t = out); None }
            }
            "intersection" | "intersection_update" | "difference" | "difference_update" => {
                let tables: Vec<ValSet> = args.iter().map(|items| self.vs_from(items, chunk)).collect::<Result<_, _>>()?;
                let keep_in = name.starts_with("intersection");
                let mut out = ValSet::with_capacity(mine.len());
                for &(h, v) in &mine {
                    let mut hits = 0;
                    for t in &tables { if self.vs_has(t, h, v, chunk)? { hits += 1; } }
                    let keep = if keep_in { hits == tables.len() } else { hits == 0 };
                    if keep { out.push_hashed(v, h, is_rich_key(v, &self.heap)); }
                }
                if name.ends_with("_update") { self.heap.growing(&mut *rc.borrow_mut(), |t| *t = out); None } else { Some(out) }
            }
            "symmetric_difference" | "symmetric_difference_update" => {
                let other = self.vs_from(&args[0], chunk)?;
                let lhs = rc.borrow().clone();
                let mut out = ValSet::with_capacity(mine.len());
                for &(h, v) in &mine { if !self.vs_has(&other, h, v, chunk)? { out.push_hashed(v, h, is_rich_key(v, &self.heap)); } }
                for (h, v) in other.iter_hashed().collect::<Vec<_>>() { if !self.vs_has(&lhs, h, v, chunk)? { out.push_hashed(v, h, is_rich_key(v, &self.heap)); } }
                if name.ends_with("_update") { self.heap.growing(&mut *rc.borrow_mut(), |t| *t = out); None } else { Some(out) }
            }
            "issubset" => {
                let other = self.vs_from(&args[0], chunk)?;
                let mut all = true;
                for &(h, v) in &mine { if !self.vs_has(&other, h, v, chunk)? { all = false; break; } }
                self.push(Val::bool(all));
                return Ok(true);
            }
            "issuperset" | "isdisjoint" => {
                let lhs = rc.borrow().clone();
                let mut hits = 0;
                for &(h, v) in &args[0] { if self.vs_has(&lhs, h, v, chunk)? { hits += 1; } }
                self.push(Val::bool(if name == "issuperset" { hits == args[0].len() } else { hits == 0 }));
                return Ok(true);
            }
            _ => return Ok(false),
        };
        match result {
            Some(s) => { let v = self.heap.alloc(HeapObj::Set(Rc::new(RefCell::new(s))))?; self.push(v); }
            None => self.push(Val::none()),
        }
        Ok(true)
    }
}
