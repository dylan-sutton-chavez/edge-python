use crate::s;

use alloc::{vec, vec::Vec};

use super::super::VM;
use super::super::types::*;
use crate::parser::{OpCode, SSAChunk};

/* A range element as a Val, promoting magnitudes beyond the 47-bit inline range to LongInt. */
pub(crate) fn range_int(heap: &mut HeapPool, i: i64) -> Result<Val, VmErr> {
    if (Val::INT_MIN..=Val::INT_MAX).contains(&i) { Ok(Val::int(i)) }
    else { heap.alloc(HeapObj::LongInt(i as i128)) }
}

// Lazy walker for short-circuit builtins; Vec variant copies because list/set/dict can't stream without a mutable heap borrow.
pub(crate) enum IterCursor {
    Range { cur: i64, end: i64, step: i64 },
    Vec { items: Vec<Val>, idx: usize },
    Bytes { bytes: Vec<u8>, idx: usize },
    StrChars { chars: Vec<char>, idx: usize },
}

impl IterCursor {
    // Next value; allocates only for StrChars. Err on alloc failure, Ok(None) on exhaustion.
    pub fn next(&mut self, heap: &mut HeapPool) -> Result<Option<Val>, VmErr> {
        match self {
            Self::Range { cur, end, step } => {
                let (c, e, s) = (*cur, *end, *step);
                let live = if s > 0 { c < e } else if s < 0 { c > e } else { false };
                if !live { return Ok(None); }
                // Clamp past the i64 edge so the next call ends the range, never overflows.
                *cur = c.checked_add(s).unwrap_or(if s > 0 { i64::MAX } else { i64::MIN });
                Ok(Some(range_int(heap, c)?))
            }
            Self::Vec { items, idx } => {
                if *idx >= items.len() { return Ok(None); }
                let v = items[*idx];
                *idx += 1;
                Ok(Some(v))
            }
            Self::Bytes { bytes, idx } => {
                if *idx >= bytes.len() { return Ok(None); }
                let b = bytes[*idx];
                *idx += 1;
                Ok(Some(Val::int(b as i64)))
            }
            Self::StrChars { chars, idx } => {
                if *idx >= chars.len() { return Ok(None); }
                let mut s = alloc::string::String::new();
                s.push(chars[*idx]);
                *idx += 1;
                Ok(Some(heap.alloc(HeapObj::Str(s))?))
            }
        }
    }
}

impl<'a> VM<'a> {

    pub fn call_len(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let o = self.pop()?;
        // instance `__len__` takes precedence over built-in length rules.
        if let Some(r) = self.try_call_dunder(o, "__len__", &[], chunk, slots)? {
            let n = if r.is_int() { r.as_int() as i128 }
            else if let Some(i) = crate::vm::types::as_i128(r, &self.heap) { i }
            else { return Err(cold_type("__len__ must return int")); };
            if n < 0 { return Err(cold_value("__len__() should return >= 0")); }
            let v = self.int_to_val(Some(n))?;
            self.push(v);
            return Ok(());
        }
        let n: i64 = if o.is_heap() { match self.heap.get(o) {
            HeapObj::Str(s) => s.chars().count() as i64,
            HeapObj::Bytes(b) => b.len() as i64,
            HeapObj::List(v) => v.borrow().len() as i64,
            HeapObj::Tuple(v) => v.len() as i64,
            HeapObj::Dict(v) => v.borrow().len() as i64,
            HeapObj::Set(v) => v.borrow().len() as i64,
            HeapObj::FrozenSet(v) => v.len() as i64,
            HeapObj::Range(s,e,st) => {
                let (s, e, st) = (*s as i128, *e as i128, *st as i128);
                if st == 0 { return Err(cold_value("range() step cannot be zero")); }
                (((e - s + st - st.signum()) / st).max(0)) as i64
            }
            _ => return Err(cold_type("object has no len()")),
        }} else { return Err(cold_type("object has no len()")); };
        self.push(Val::int(n)); Ok(())
    }

    pub fn call_sorted(&mut self, reverse: bool, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let o = self.pop()?;
        let mut items = self.extract_iter(o, false)?;
        self.sort_by_lt(&mut items, chunk, slots)?;
        if reverse { items.reverse(); }
        self.alloc_and_push_list(items)
    }

    /* sorted(iterable, key=fn, reverse=False) — delegates to call_sorted when key is absent. */
    pub fn call_sorted_with_key(&mut self, key: Option<Val>, reverse: bool, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let key = match key {
            Some(k) if !k.is_none() => k,
            _ => return self.call_sorted(reverse, chunk, slots),
        };
        let o = self.pop()?;
        let items = self.extract_iter(o, false)?;
        let mut sorted = self.sort_by_key(items, key, chunk, slots)?;
        if reverse { sorted.reverse(); }
        self.alloc_and_push_list(sorted)
    }

    /* list.sort(key=fn, reverse=False) in-place. Snapshots list before key calls so heap borrow ends before exec_call. */
    pub fn call_list_sort_keyed(&mut self, recv: Val, key: Option<Val>, reverse: bool, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let items = match self.heap.get(recv) {
            HeapObj::List(rc) => rc.borrow().clone(),
            _ => return Err(cold_type("sort: receiver is not a list")),
        };
        let mut result = if let Some(k) = key.filter(|k| !k.is_none()) {
            self.sort_by_key(items, k, chunk, slots)?
        } else {
            let mut s = items;
            self.sort_by_lt(&mut s, chunk, slots)?;
            s
        };
        if reverse { result.reverse(); }
        let rc = match self.heap.get(recv) {
            HeapObj::List(rc) => rc.clone(),
            _ => return Err(cold_type("sort: receiver is not a list")),
        };
        *rc.borrow_mut() = result;
        self.mark_impure();
        self.push(Val::none());
        Ok(())
    }

    /* Decorate-sort-undecorate: applies key fn to each item, sorts by resulting keys, returns reordered items. */
    fn sort_by_key(&mut self, items: Vec<Val>, key: Val, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<Vec<Val>, VmErr> {
        let mut keys: Vec<Val> = Vec::with_capacity(items.len());
        for &item in &items {
            self.push(key);
            self.push(item);
            self.exec_call(1, chunk, slots)?;
            keys.push(self.pop()?);
        }
        // Root both keys and items: a `__lt__` comparison can run user code that GCs.
        let order = self.sorted_order(&keys, &items, chunk, slots)?;
        Ok(order.into_iter().map(|i| items[i]).collect())
    }

    /* Root the operands (comparators can run GC-triggering user code), stable-sort `keys` via `sort_lt`, and return the index permutation. `extra_roots` keeps caller-only values (e.g. the items in a keyed sort) alive across comparisons. First comparison error wins; later comparisons degrade to Equal. */
    fn sorted_order(&mut self, keys: &[Val], extra_roots: &[Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<Vec<usize>, VmErr> {
        let roots_base = self.temp_roots.len();
        for &v in keys.iter().chain(extra_roots.iter()) { self.temp_roots.push(v); }
        let mut sort_err: Option<VmErr> = None;
        let order = Self::stable_sort_indices(keys.len(), |a, b| {
            if sort_err.is_some() { return core::cmp::Ordering::Equal; }
            match self.sort_lt(keys[a], keys[b], chunk, slots) {
                Ok(true) => core::cmp::Ordering::Less,
                Ok(false) => match self.sort_lt(keys[b], keys[a], chunk, slots) {
                    Ok(true) => core::cmp::Ordering::Greater,
                    Ok(false) => core::cmp::Ordering::Equal,
                    Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
                },
                Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
            }
        });
        self.temp_roots.truncate(roots_base);
        if let Some(e) = sort_err { return Err(e); }
        Ok(order)
    }

    // a < b via __lt__ when either side defines it, else the built-in comparison.
    pub(crate) fn sort_lt(&mut self, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if let Some(r) = self.try_compare_dunder(OpCode::Lt, a, b, chunk, slots)? {
            return Ok(self.truthy(r));
        }
        self.lt_vals(a, b)
    }

    /* In-place sort dispatching `__lt__`; roots items since a comparison can run user code that GCs. */
    pub(crate) fn sort_by_lt(&mut self, items: &mut [Val], chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let snapshot = items.to_vec();
        let order = self.sorted_order(&snapshot, &[], chunk, slots)?;
        for (dst, &src) in order.iter().enumerate() { items[dst] = snapshot[src]; }
        Ok(())
    }

    /* Stable merge sort over `0..n`; unlike `slice::sort_by` it tolerates a non-total `cmp` (NaN keys) without aborting. */
    fn stable_sort_indices<F>(n: usize, mut cmp: F) -> Vec<usize>
    where F: FnMut(usize, usize) -> core::cmp::Ordering {
        let mut idx: Vec<usize> = (0..n).collect();
        if n < 2 { return idx; }
        let mut buf = idx.clone();
        let mut width = 1;
        while width < n {
            let mut lo = 0;
            while lo < n {
                let mid = (lo + width).min(n);
                let hi = (lo + 2 * width).min(n);
                let (mut a, mut b, mut k) = (lo, mid, lo);
                while a < mid && b < hi {
                    // Take the right run only on a strict Less, so equal keys keep input order (stable).
                    if cmp(idx[b], idx[a]) == core::cmp::Ordering::Less {
                        buf[k] = idx[b]; b += 1;
                    } else {
                        buf[k] = idx[a]; a += 1;
                    }
                    k += 1;
                }
                while a < mid { buf[k] = idx[a]; a += 1; k += 1; }
                while b < hi { buf[k] = idx[b]; b += 1; k += 1; }
                lo += 2 * width;
            }
            core::mem::swap(&mut idx, &mut buf);
            width *= 2;
        }
        idx
    }

    pub fn call_reversed(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if !o.is_heap() { return Err(cold_type("reversed() requires a sequence")); }
        let mut items = if let HeapObj::Str(s) = self.heap.get(o) {
            let s = s.clone();
            self.str_to_char_vals(&s)?
        } else {
            self.extract_iter(o, true)?
        };
        items.reverse();
        self.alloc_and_push_list(items)
    }

    pub fn call_enumerate(&mut self, op: u16) -> Result<(), VmErr> {
        let (positional, kw_flat, _np, _nk) = self.parse_call_args(op)?;
        if positional.is_empty() || positional.len() > 2 {
            return Err(cold_type("enumerate() takes 1 or 2 positional arguments"));
        }
        // `start` is positional (`enumerate(xs, 5)`) or keyword (`enumerate(xs, start=5)`); default 0.
        let mut start = if positional.len() == 2 { positional[1] } else { Val::int(0) };
        for pair in kw_flat.chunks_exact(2) {
            match self.kw_name(pair[0]) {
                Some("start") => start = pair[1],
                _ => return Err(cold_type("enumerate() got an unexpected keyword argument")),
            }
        }
        let start = match self.as_i128(start) {
            Some(n) => n,
            None => return Err(cold_type("enumerate() start must be an integer")),
        };
        let src = self.extract_iter(positional[0], false)?;
        let mut pairs: Vec<Val> = Vec::with_capacity(src.len());
        for (i, x) in src.into_iter().enumerate() {
            let idx = self.int_to_val(start.checked_add(i as i128))?;
            let t = self.heap.alloc(HeapObj::Tuple(vec![idx, x]))?;
            pairs.push(t);
        }
        self.alloc_and_push_list(pairs)
    }

    /* Pairs elements from N iterables into tuples, truncating to the shortest. */
    pub fn call_zip(&mut self, op: u16) -> Result<(), VmErr> {
        let mut iters: Vec<Vec<Val>> = Vec::with_capacity(op as usize);
        let mut vals = Vec::with_capacity(op as usize);
        for _ in 0..op { vals.push(self.pop()?); }
        vals.reverse();
        for v in vals { iters.push(self.extract_iter(v, false)?); }
        let len = iters.iter().map(|v| v.len()).min().unwrap_or(0);
        let mut pairs: Vec<Val> = Vec::with_capacity(len);
        for i in 0..len {
            let tuple: Vec<Val> = iters.iter().map(|v| v[i]).collect();
            let t = self.heap.alloc(HeapObj::Tuple(tuple))?;
            pairs.push(t);
        }
        self.alloc_and_push_list(pairs)
    }

    // TypeError for a non-iterable operand.
    fn not_iterable(&self, o: Val) -> VmErr {
        VmErr::TypeMsg(s!("'", str self.type_name(o), "' object is not iterable"))
    }

    /* Build an IterCursor so short-circuit builtins (e.g. `all(range(10**6))`) stop at the first hit instead of pre-materialising. TypeError on non-iterables. */
    pub(in crate::vm) fn iter_cursor(&self, o: Val) -> Result<IterCursor, VmErr> {
        if !o.is_heap() {
            return Err(self.not_iterable(o));
        }
        Ok(match self.heap.get(o) {
            HeapObj::Range(s, e, st) => IterCursor::Range { cur: *s, end: *e, step: *st },
            HeapObj::Bytes(b) => IterCursor::Bytes { bytes: b.clone(), idx: 0 },  // Vec<u8> clone
            HeapObj::Str(s) => IterCursor::StrChars { chars: s.chars().collect(), idx: 0 },
            HeapObj::List(v) => IterCursor::Vec { items: v.borrow().clone(), idx: 0 },
            HeapObj::Tuple(v) => IterCursor::Vec { items: v.clone(), idx: 0 },
            HeapObj::Set(v) => IterCursor::Vec { items: v.borrow().iter().cloned().collect(), idx: 0 },
            HeapObj::FrozenSet(v) => IterCursor::Vec { items: v.iter().cloned().collect(), idx: 0 },
            HeapObj::Dict(d) => IterCursor::Vec { items: d.borrow().keys().collect(), idx: 0 },
            _ => return Err(self.not_iterable(o)),
        })
    }

    /* Vec<Val> from any iterable (dict yields keys, str yields one-char strs, bytes yields ints). `include_range = false` lets callers reject Range. */
    pub(in crate::vm) fn extract_iter(&mut self, o: Val, include_range: bool) -> Result<Vec<Val>, VmErr> {
        if !o.is_heap() {
            return Err(self.not_iterable(o));
        }
        // Snapshot the variant out so the &self borrow ends before any allocation.
        let snapshot = match self.heap.get(o) {
            HeapObj::List(v) => Some(v.borrow().clone()),
            HeapObj::Tuple(v) => Some(v.clone()),
            HeapObj::Set(v) => Some(v.borrow().iter().cloned().collect()),
            HeapObj::FrozenSet(v) => Some(v.iter().cloned().collect()),
            HeapObj::Range(s, e, st) if include_range => {
                let (mut cur, end, step) = (*s, *e, *st);
                // Materialised length is user-controlled; cap it against the heap budget.
                let span = (end as i128 - cur as i128).unsigned_abs();
                let count = if step == 0 { 0 } else { span / (step as i128).unsigned_abs() };
                if count > self.heap.limit() as u128 { return Err(cold_heap()); }
                let mut out = Vec::new();
                if step > 0 {
                    while cur < end {
                        out.push(range_int(&mut self.heap, cur)?);
                        match cur.checked_add(step) { Some(n) => cur = n, None => break }
                    }
                } else {
                    while cur > end {
                        out.push(range_int(&mut self.heap, cur)?);
                        match cur.checked_add(step) { Some(n) => cur = n, None => break }
                    }
                }
                Some(out)
            }
            HeapObj::Dict(d) => Some(d.borrow().keys().collect()),
            HeapObj::Bytes(b) => Some(b.iter().map(|&x| Val::int(x as i64)).collect()),
            HeapObj::Str(_) => None, // handled below, needs heap allocation
            _ => return Err(self.not_iterable(o)),
        };
        if let Some(v) = snapshot {
            // Cost scales with element count; charge it so repeated materialisation stays bounded.
            self.charge_steps(v.len())?;
            return Ok(v);
        }
        // Str path materialises one-char heap strings via the existing helper.
        if let HeapObj::Str(s) = self.heap.get(o) {
            let s = s.clone();
            return self.str_to_char_vals(&s);
        }
        unreachable!()
    }

    /* Flatten any iterable to a fresh `Vec<Val>`, shared input path for iter/map/filter so all three accept the same set of sources. */
    pub(crate) fn iter_to_vec_general(&mut self, o: Val) -> Result<Vec<Val>, VmErr> {
        if !o.is_heap() {
            return Err(self.not_iterable(o));
        }
        if let HeapObj::Str(s) = self.heap.get(o) {
            let s = s.clone();
            return self.str_to_char_vals(&s);
        }
        if let HeapObj::Bytes(b) = self.heap.get(o) {
            // bytes iterates as ints (Python semantics; same as bytes[i]).
            return Ok(b.iter().map(|&byte| Val::int(byte as i64)).collect());
        }
        if let HeapObj::Dict(rc) = self.heap.get(o) {
            return Ok(rc.borrow().keys().collect());
        }
        self.extract_iter(o, true)
    }

    /* `iter(x)`, eager flatten into a fresh List drained front-to-back by `next()`. Original isn't touched. Mirrors the universal ABI's `Op::Iter`. The 2-arg form `iter(callable, sentinel)` calls `callable()` until it returns `sentinel`, eagerly. */
    pub fn call_iter(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 2 {
            let sentinel = self.pop()?;
            let callable = self.pop()?;
            let mut items: Vec<Val> = Vec::new();
            loop {
                self.charge_step()?; // bound the call loop against the op budget
                self.push(callable);
                self.exec_call(0, chunk, slots)?;
                let v = self.pop()?;
                if eq_vals_with_heap(v, sentinel, &self.heap) { break; }
                if items.len() >= self.heap.limit() { return Err(cold_heap()); }
                items.push(v);
            }
            return self.alloc_and_push_list(items);
        }
        if argc != 1 { return Err(cold_type("iter() takes 1 or 2 arguments")); }
        let o = self.pop()?;
        let items = self.iter_to_vec_general(o)?;
        self.alloc_and_push_list(items)
    }

    pub fn call_next(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 || argc > 2 { return Err(cold_type("next() takes 1 or 2 arguments")); }
        // `next(it, default)`: the 2nd arg is returned instead of raising StopIteration on exhaustion.
        let default = if argc == 2 { Some(self.pop()?) } else { None };
        let o = self.pop()?;
        if !o.is_heap() { return Err(cold_type("next() requires an iterator")); }
        // User iterator: dispatch __next__, mapping StopIteration to the optional default.
        if matches!(self.heap.get(o), HeapObj::Instance(..)) {
            return match self.try_call_dunder(o, "__next__", &[], chunk, slots) {
                Ok(Some(v)) => { self.push(v); Ok(()) }
                Ok(None) => Err(cold_type("next() requires an iterator")),
                Err(VmErr::Raised(m)) if default.is_some() && (m == "StopIteration" || m.starts_with("StopIteration")) => {
                    self.push(default.unwrap()); Ok(())
                }
                Err(e) => Err(e),
            };
        }
        // List path mirrors the ABI's IterNext op so script `next()` and host `Op::IterNext` match.
        if let HeapObj::List(rc) = self.heap.get(o) {
            let rc = rc.clone();
            let mut v = rc.borrow_mut();
            if v.is_empty() {
                drop(v);
                return match default { Some(d) => { self.push(d); Ok(()) }, None => Err(VmErr::Raised(s!("StopIteration"))) };
            }
            let item = v.remove(0);
            drop(v);
            self.push(item);
            return Ok(());
        }
        if !matches!(self.heap.get(o), HeapObj::Coroutine(..)) {
            return Err(cold_type("next() requires an iterator"));
        }
        self.push(o); // root across resume's GC
        let result = self.resume_coroutine(o)?;
        if self.yielded {
            self.yielded = false;
            *self.stack.last_mut().unwrap() = result; // leave one result, not two
            Ok(())
        } else {
            self.pop()?; // drop the rooted coroutine
            match default { Some(d) => { self.push(d); Ok(()) }, None => Err(VmErr::Raised(s!("StopIteration"))) }
        }
    }

    /* `map(fn, iter)`, eager; returns a list. Re-enters `exec_call` per item so closures with captures see the caller's chunk/slots. */
    pub fn call_map(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc < 2 { return Err(cold_type("map() must have at least two arguments")); }
        let mut args = self.pop_n(argc as usize)?;
        let fn_val = args.remove(0);
        // Materialise each iterable; the parallel walk stops at the shortest, like zip.
        let mut lists: Vec<Vec<Val>> = Vec::with_capacity(args.len());
        for it in args { lists.push(self.iter_to_vec_general(it)?); }
        let n = lists.iter().map(|l| l.len()).min().unwrap_or(0);
        let arity = lists.len() as u16;
        let mut out: Vec<Val> = Vec::with_capacity(n);
        for i in 0..n {
            self.push(fn_val);
            for l in &lists { self.push(l[i]); }
            self.exec_call(arity, chunk, slots)?;
            out.push(self.pop()?);
        }
        self.alloc_and_push_list(out)
    }

    /* `filter(pred, iter)`, eager; keeps truthy `pred(item)`. Same call-shape as `map`. `pred=None` falls back to Python's identity-truthy filter. */
    pub fn call_filter(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let iterable = self.pop()?;
        let fn_val = self.pop()?;
        let items = self.iter_to_vec_general(iterable)?;
        let mut out: Vec<Val> = Vec::new();
        for item in items {
            let keep = if fn_val.is_none() {
                self.truthy(item)
            } else {
                self.push(fn_val);
                self.push(item);
                self.exec_call(1, chunk, slots)?;
                let r = self.pop()?;
                self.truthy(r)
            };
            if keep { out.push(item); }
        }
        self.alloc_and_push_list(out)
    }

    /* Short-circuit truthiness scan shared by `all`/`any`: stops at the first element whose truthiness equals `find`, pushing `find`; pushes `!find` on exhaustion. */
    fn scan_truthy(&mut self, op: u16, find: bool, arity_err: &'static str) -> Result<(), VmErr> {
        if op != 1 { return Err(cold_type(arity_err)); }
        let o = self.pop()?;
        // Generators step lazily so evaluation stops at the deciding element (short-circuit).
        if o.is_heap() && matches!(self.heap.get(o), HeapObj::Coroutine(..)) {
            // Root the coroutine on the VM stack; each resume can allocate and trigger GC.
            self.push(o);
            let decided = loop {
                self.charge_step()?;
                let v = self.resume_coroutine(o)?;
                if !self.yielded { break None; }
                self.yielded = false;
                if self.truthy(v) == find { break Some(find); }
            };
            self.pop()?;
            self.push(Val::bool(decided.unwrap_or(!find)));
            return Ok(());
        }
        let mut cur = self.iter_cursor(o)?;
        while let Some(v) = cur.next(&mut self.heap)? {
            self.charge_step()?; // native iteration over a huge range must charge the op-budget
            if self.truthy(v) == find {
                self.push(Val::bool(find));
                return Ok(());
            }
        }
        self.push(Val::bool(!find));
        Ok(())
    }

    pub fn call_all(&mut self, op: u16) -> Result<(), VmErr> { self.scan_truthy(op, false, "all() takes exactly 1 argument") }

    pub fn call_any(&mut self, op: u16) -> Result<(), VmErr> { self.scan_truthy(op, true, "any() takes exactly 1 argument") }

    // Materialise an iterable to a list, strings -> chars, ranges eager, coroutines drained.
    pub fn call_list(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 { return self.alloc_and_push_list(Vec::new()); } // `list()` is the empty list.
        let o = self.pop()?;
        // user-defined iterable wins over the built-in dispatch.
        if let Some(items) = self.iter_to_vec_op(o, chunk, slots)? {
            return self.alloc_and_push_list(items);
        }
        if o.is_heap() {
            match self.heap.get(o) {
                HeapObj::Str(s) => {
                    let s = s.clone();
                    let items = self.str_to_char_vals(&s)?;
                    return self.alloc_and_push_list(items);
                }
                HeapObj::Coroutine(..) => {
                    // Keep the coroutine and its yielded values rooted on the VM stack; each resume can allocate and trigger GC.
                    self.push(o);
                    let base = self.stack.len();
                    loop {
                        self.charge_step()?;
                        let v = self.resume_coroutine(o)?;
                        if !self.yielded { break; }
                        self.yielded = false;
                        self.push(v);
                    }
                    // A shorter stack must not panic split_off; clamp.
                    let out = self.stack.split_off(base.min(self.stack.len()));
                    self.pop()?; // drop the rooted coroutine
                    return self.alloc_and_push_list(out);
                }
                _ => {}
            }
        }
        let items = self.extract_iter(o, true)?;
        self.alloc_and_push_list(items)
    }

    pub fn call_tuple(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 { return self.alloc_and_push_tuple(Vec::new()); } // `tuple()` is the empty tuple.
        let o = self.pop()?;
        if let Some(items) = self.iter_to_vec_op(o, chunk, slots)? {
            return self.alloc_and_push_tuple(items);
        }
        let items = self.extract_iter(o, true)?;
        self.alloc_and_push_tuple(items)
    }

}
