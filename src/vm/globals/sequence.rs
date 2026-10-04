use crate::s;

use alloc::{vec, vec::Vec};

use super::super::VM;
use super::super::types::*;
use crate::parser::{OpCode, SSAChunk};


impl<'a> VM<'a> {

    /* Pushes a builtin iterator named `name` over `frame`. */
    pub(crate) fn push_iterator(&mut self, frame: IterFrame, name: &'static str) -> Result<(), VmErr> {
        let it = self.heap.alloc(HeapObj::Iter(alloc::rc::Rc::new(core::cell::RefCell::new(frame)), name))?;
        self.push(it);
        Ok(())
    }

    /* Pushes an iterator over items computed up front. */
    fn push_items(&mut self, items: Vec<Val>, name: &'static str) -> Result<(), VmErr> {
        self.push_iterator(IterFrame::Seq { items: items.into(), idx: 0 }, name)
    }

    /* The next item of builtin iterator `it`, None once it is spent. */
    pub(crate) fn iter_step(&mut self, it: Val) -> Result<Option<Val>, VmErr> {
        let Some(HeapObj::Iter(frame, _)) = self.heap.try_get(it) else { return Ok(None); };
        let frame = frame.clone();
        frame.borrow_mut().next_item(&mut self.heap)
    }

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
        let n = self.builtin_len(o)?;
        let v = self.int_to_val(Some(n))?;
        self.push(v); Ok(())
    }

    /* `len(o)` of a builtin container, shared with the plugin ABI. */
    pub(crate) fn builtin_len(&mut self, o: Val) -> Result<i128, VmErr> {
        let ascii = self.heap.str_is_ascii(o);
        Ok(match self.heap.try_get(o) {
            Some(HeapObj::Str(s)) => (if ascii { s.len() } else { s.chars().count() }) as i128,
            Some(HeapObj::Bytes(b)) => b.len() as i128,
            Some(HeapObj::List(v)) => v.borrow().len() as i128,
            Some(HeapObj::Tuple(v)) => v.len() as i128,
            Some(HeapObj::Dict(v)) => v.borrow().len() as i128,
            Some(HeapObj::Set(v)) => v.borrow().len() as i128,
            Some(HeapObj::FrozenSet(v)) => v.len() as i128,
            Some(&HeapObj::Range(_, _, 0)) => return Err(cold_value("range() step cannot be zero")),
            Some(&HeapObj::Range(s, e, st)) => crate::vm::eq::range_len(s, e, st),
            _ => return Err(cold_type("object has no len()")),
        })
    }

    pub fn call_sorted(&mut self, reverse: bool, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let o = self.pop()?;
        let mut items = self.extract_iter(o)?;
        self.sort_by_lt(&mut items, reverse, chunk, slots)?;
        self.alloc_and_push_list(items)
    }

    /* sorted(iterable, key=fn, reverse=False), delegates to call_sorted when key is absent. */
    pub fn call_sorted_with_key(&mut self, key: Option<Val>, reverse: bool, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let key = match key {
            Some(k) if !k.is_none() => k,
            _ => return self.call_sorted(reverse, chunk, slots),
        };
        let o = self.pop()?;
        let items = self.extract_iter(o)?;
        let sorted = self.sort_by_key(items, key, reverse, chunk, slots)?;
        self.alloc_and_push_list(sorted)
    }

    /* list.sort(key=fn, reverse=False) in-place. Snapshots list before key calls so heap borrow ends before exec_call. */
    pub fn call_list_sort_keyed(&mut self, recv: Val, key: Option<Val>, reverse: bool, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let items = match self.heap.get(recv) {
            HeapObj::List(rc) => rc.borrow().clone(),
            _ => return Err(cold_type("sort: receiver is not a list")),
        };
        let result = if let Some(k) = key.filter(|k| !k.is_none()) {
            self.sort_by_key(items, k, reverse, chunk, slots)?
        } else {
            let mut s = items;
            self.sort_by_lt(&mut s, reverse, chunk, slots)?;
            s
        };
        let rc = match self.heap.get(recv) {
            HeapObj::List(rc) => rc.clone(),
            _ => return Err(cold_type("sort: receiver is not a list")),
        };
        self.heap.growing(&mut *rc.borrow_mut(), |v| *v = result);
        self.mark_impure();
        self.push(Val::none());
        Ok(())
    }

    /* Decorate-sort-undecorate, applies key fn to each item, sorts by resulting keys, returns reordered items. */
    fn sort_by_key(&mut self, items: Vec<Val>, key: Val, reverse: bool, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<Vec<Val>, VmErr> {
        let keys = self.call_rows(key, core::slice::from_ref(&items), items.len(), chunk, slots)?;
        // Root both keys and items because a `__lt__` comparison can run user code that GCs.
        let order = self.sorted_order(&keys, &items, reverse, chunk, slots)?;
        Ok(order.into_iter().map(|i| items[i]).collect())
    }

    /* Root the operands (comparators can run GC-triggering user code), stable-sort `keys` via `sort_lt`, and return the index permutation. `extra_roots` keeps caller-only values (e.g. the items in a keyed sort) alive across comparisons. First comparison error wins, later comparisons degrade to Equal. */
    fn sorted_order(&mut self, keys: &[Val], extra_roots: &[Val], reverse: bool, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Vec<usize>, VmErr> {
        if let Some(order) = plain_order(keys, reverse, &self.heap) { return Ok(order); }
        let roots_base = self.temp_roots.len();
        // Only an instance key runs user code that could collect.
        if self.any_instance(keys) { self.temp_roots.extend(keys.iter().chain(extra_roots).copied()); }
        let mut sort_err: Option<VmErr> = None;
        let order = Self::stable_sort_indices(keys.len(), |a, b| {
            if sort_err.is_some() { return core::cmp::Ordering::Equal; }
            // Descending compares flipped, so equal keys keep their order like an ascending sort.
            let (a, b) = if reverse { (b, a) } else { (a, b) };
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

    /* True when a comparison over `vals` can reach a user dunder. */
    pub(crate) fn any_instance(&self, vals: &[Val]) -> bool {
        vals.iter().any(|&v| matches!(self.heap.try_get(v), Some(HeapObj::Instance(..))))
    }

    // a < b via __lt__ when either side defines it, else the built-in comparison.
    #[inline]
    pub(crate) fn sort_lt(&mut self, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<bool, VmErr> {
        if let Some(r) = self.try_compare_dunder(OpCode::Lt, a, b, chunk, slots)? {
            return Ok(self.truthy(r));
        }
        self.values_lt(a, b, chunk, slots)
    }

    /* In-place sort dispatching `__lt__`, roots items since a comparison can run user code that GCs. */
    pub(crate) fn sort_by_lt(&mut self, items: &mut [Val], reverse: bool, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let snapshot = items.to_vec();
        let order = self.sorted_order(&snapshot, &[], reverse, chunk, slots)?;
        for (dst, &src) in order.iter().enumerate() { items[dst] = snapshot[src]; }
        Ok(())
    }

    /* Stable merge sort over `0..n`, unlike `slice::sort_by` it tolerates a non-total `cmp` (NaN keys) without aborting. */
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

    /* `reversed(seq)` over a sequence, its items read up front, a set or an iterator is not reversible. */
    pub fn call_reversed(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let name = match self.heap.try_get(o) {
            Some(HeapObj::List(_)) => "list_reverseiterator",
            Some(HeapObj::Range(..)) => "range_iterator",
            Some(HeapObj::Str(_) | HeapObj::Tuple(_) | HeapObj::Bytes(_) | HeapObj::Dict(_)) => "reversed",
            _ => return Err(VmErr::TypeMsg(s!("'", str self.type_name(o), "' object is not reversible"))),
        };
        let mut items = self.extract_iter(o)?;
        items.reverse();
        self.push_items(items, name)
    }

    pub fn call_enumerate(&mut self, op: u16) -> Result<(), VmErr> {
        let (positional, kw_flat) = self.parse_call_args(op)?;
        if positional.is_empty() || positional.len() > 2 {
            return Err(cold_type("enumerate() takes 1 or 2 positional arguments"));
        }
        // `start` is positional (`enumerate(xs, 5)`) or keyword (`enumerate(xs, start=5)`), default 0.
        let mut start = if positional.len() == 2 { positional[1] } else { Val::int(0) };
        for pair in kw_flat.as_chunks::<2>().0 {
            match self.kw_name(pair[0]) {
                Some("start") => start = pair[1],
                _ => return Err(cold_type("enumerate() got an unexpected keyword argument")),
            }
        }
        let start = match self.as_i128(start) {
            Some(n) => n,
            None => return Err(cold_type("enumerate() start must be an integer")),
        };
        let src = self.extract_iter(positional[0])?;
        let mut pairs: Vec<Val> = Vec::with_capacity(src.len());
        for (i, x) in src.into_iter().enumerate() {
            let idx = self.int_to_val(start.checked_add(i as i128))?;
            let t = self.heap.alloc(HeapObj::Tuple(vec![idx, x]))?;
            pairs.push(t);
        }
        self.push_items(pairs, "enumerate")
    }

    /* Pairs elements from N iterables into tuples, truncating to the shortest. */
    pub fn call_zip(&mut self, op: u16) -> Result<(), VmErr> {
        let mut iters: Vec<Vec<Val>> = Vec::with_capacity(op as usize);
        let mut vals = Vec::with_capacity(op as usize);
        for _ in 0..op { vals.push(self.pop()?); }
        vals.reverse();
        for v in vals { iters.push(self.extract_iter(v)?); }
        let len = iters.iter().map(|v| v.len()).min().unwrap_or(0);
        let mut pairs: Vec<Val> = Vec::with_capacity(len);
        for i in 0..len {
            let tuple: Vec<Val> = iters.iter().map(|v| v[i]).collect();
            let t = self.heap.alloc(HeapObj::Tuple(tuple))?;
            pairs.push(t);
        }
        self.push_items(pairs, "zip")
    }

    // TypeError for a non-iterable operand.
    fn not_iterable(&self, o: Val) -> VmErr {
        VmErr::TypeMsg(s!("'", str self.type_name(o), "' object is not iterable"))
    }

    /* Steps `o` lazily when a range or list, so `all`, `any` and `sum` stop early. */
    pub(in crate::vm) fn iter_cursor(&mut self, o: Val) -> Result<IterFrame, VmErr> {
        Ok(match self.heap.try_get(o) {
            Some(&HeapObj::Range(cur, end, step)) => IterFrame::Range { cur, end, step },
            Some(HeapObj::List(rc)) => IterFrame::List { rc: rc.clone(), idx: 0 },
            _ => IterFrame::Seq { items: self.extract_iter(o)?.into(), idx: 0 },
        })
    }

    /* Vec<Val> from any iterable (dict yields keys, str yields one-char strs, bytes yields ints, a generator runs to its end). */
    pub(crate) fn extract_iter(&mut self, o: Val) -> Result<Vec<Val>, VmErr> {
        if !o.is_heap() {
            return Err(self.not_iterable(o));
        }
        // An iterator yields what it has left, which spends it.
        if matches!(self.heap.get(o), HeapObj::Iter(..)) {
            let mut out = Vec::new();
            while let Some(v) = self.iter_step(o)? { out.push(v); }
            self.charge_steps(out.len())?;
            return Ok(out);
        }
        if matches!(self.heap.get(o), HeapObj::Coroutine(..)) {
            // Keep the coroutine and its yielded values rooted on the VM stack, each resume can allocate and trigger GC.
            self.push(o);
            let base = self.stack.len();
            loop {
                self.charge_step()?;
                let v = self.resume_coroutine(o)?;
                if !self.yielded { break; }
                self.yielded = false;
                self.push(v);
            }
            let out = self.stack.split_off(base.min(self.stack.len()));
            self.pop()?;
            return Ok(out);
        }
        // Snapshot the variant out so the &self borrow ends before any allocation.
        let snapshot = match self.heap.get(o) {
            HeapObj::List(v) => Some(v.borrow().clone()),
            HeapObj::Tuple(v) => Some(v.clone()),
            HeapObj::Set(v) => Some(v.borrow().iter().cloned().collect()),
            HeapObj::FrozenSet(v) => Some(v.iter().cloned().collect()),
            HeapObj::Range(s, e, st) => {
                let (mut cur, end, step) = (*s, *e, *st);
                // Materialised length is user-controlled, cap it against the memory left.
                let span = (end as i128 - cur as i128).unsigned_abs();
                let count = if step == 0 { 0 } else { span / (step as i128).unsigned_abs() };
                if count.saturating_mul(VAL_BYTES as u128) > self.heap.room() as u128 { return Err(cold_heap()); }
                let mut out = Vec::new();
                if step > 0 {
                    while cur < end {
                        out.push(self.heap.int(cur as i128)?);
                        match cur.checked_add(step) { Some(n) => cur = n, None => break }
                    }
                } else {
                    while cur > end {
                        out.push(self.heap.int(cur as i128)?);
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
            // Cost scales with element count, charge it so repeated materialisation stays bounded.
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

    /* `iter(x)` and `iter(f, sentinel)`, a list or range steps live and the rest up front. */
    pub fn call_iter(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 2 {
            let sentinel = self.pop()?;
            let callable = self.pop()?;
            // Collected values stay rooted, each call can run a collection.
            let items = self.with_roots([callable, sentinel], |vm| {
                let mut items: Vec<Val> = Vec::new();
                loop {
                    vm.charge_step()?; // bound the call loop against the op budget
                    vm.push(callable);
                    vm.exec_call(0, chunk, slots)?;
                    let v = vm.pop()?;
                    if eq_member(v, sentinel, &vm.heap) { break; }
                    vm.heap.reserve((items.len() + 1) * VAL_BYTES)?;
                    vm.temp_roots.push(v);
                    items.push(v);
                }
                Ok(items)
            })?;
            return self.push_items(items, "callable_iterator");
        }
        if argc != 1 { return Err(cold_type("iter() takes 1 or 2 arguments")); }
        let o = self.pop()?;
        let ascii = self.heap.str_is_ascii(o);
        let name = match self.heap.try_get(o) {
            Some(HeapObj::Iter(..) | HeapObj::Coroutine(..)) => { self.push(o); return Ok(()); }
            Some(HeapObj::Instance(..)) => {
                let it = self.try_call_dunder(o, "__iter__", &[], chunk, slots)?.ok_or_else(|| self.not_iterable(o))?;
                self.push(it);
                return Ok(());
            }
            Some(HeapObj::List(rc)) => { let rc = rc.clone(); return self.push_iterator(IterFrame::List { rc, idx: 0 }, "list_iterator"); }
            Some(&HeapObj::Range(cur, end, step)) => return self.push_iterator(IterFrame::Range { cur, end, step }, "range_iterator"),
            Some(HeapObj::Tuple(_)) => "tuple_iterator",
            Some(HeapObj::Str(_)) => if ascii { "str_ascii_iterator" } else { "str_iterator" },
            Some(HeapObj::Dict(_)) => "dict_keyiterator",
            Some(HeapObj::Set(_) | HeapObj::FrozenSet(_)) => "set_iterator",
            Some(HeapObj::Bytes(_)) => "bytes_iterator",
            _ => return Err(self.not_iterable(o)),
        };
        let items = self.extract_iter(o)?;
        self.push_items(items, name)
    }

    pub fn call_next(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 || argc > 2 { return Err(cold_type("next() takes 1 or 2 arguments")); }
        // `next(it, default)` returns the 2nd arg instead of raising StopIteration on exhaustion.
        let default = if argc == 2 { Some(self.pop()?) } else { None };
        let o = self.pop()?;
        if !o.is_heap() { return Err(cold_type("next() requires an iterator")); }
        // For a user iterator, dispatch __next__, mapping StopIteration to the optional default.
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
        if matches!(self.heap.get(o), HeapObj::Iter(..)) {
            return match (self.iter_step(o)?, default) {
                (Some(v), _) | (None, Some(v)) => { self.push(v); Ok(()) }
                (None, None) => Err(VmErr::Raised(s!("StopIteration"))),
            };
        }
        if !matches!(self.heap.get(o), HeapObj::Coroutine(..)) {
            return Err(VmErr::TypeMsg(s!("'", str self.type_name(o), "' object is not an iterator")));
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

    /* `map(fn, iter)`, its results computed up front. Re-enters `exec_call` per item so closures with captures see the caller frame. */
    pub fn call_map(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc < 2 { return Err(cold_type("map() must have at least two arguments")); }
        let mut args = self.pop_n(argc as usize)?;
        let fn_val = args.remove(0);
        // Materialise each iterable, the parallel walk stops at the shortest, like zip.
        let mut lists: Vec<Vec<Val>> = Vec::with_capacity(args.len());
        for it in args { lists.push(self.extract_iter(it)?); }
        let n = lists.iter().map(|l| l.len()).min().unwrap_or(0);
        let out = self.call_rows(fn_val, &lists, n, chunk, slots)?;
        self.push_items(out, "map")
    }

    /* `filter(pred, iter)`, computed up front, keeps truthy `pred(item)`, a None predicate keeps truthy items. */
    pub fn call_filter(&mut self, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let iterable = self.pop()?;
        let fn_val = self.pop()?;
        let items = self.extract_iter(iterable)?;
        let out: Vec<Val> = if fn_val.is_none() {
            items.into_iter().filter(|&v| self.truthy(v)).collect()
        } else {
            let verdicts = self.call_rows(fn_val, core::slice::from_ref(&items), items.len(), chunk, slots)?;
            items.into_iter().zip(verdicts).filter(|&(_, r)| self.truthy(r)).map(|(v, _)| v).collect()
        };
        self.push_items(out, "filter")
    }

    /* Short-circuit truthiness scan shared by `all`/`any`, stops at the first element whose truthiness equals `find`, pushing `find`. Pushes `!find` on exhaustion. */
    fn scan_truthy(&mut self, op: u16, find: bool, arity_err: &'static str) -> Result<(), VmErr> {
        if op != 1 { return Err(cold_type(arity_err)); }
        let o = self.pop()?;
        // Generators step lazily so evaluation stops at the deciding element (short-circuit).
        if o.is_heap() && matches!(self.heap.get(o), HeapObj::Coroutine(..)) {
            // Root the coroutine on the VM stack, each resume can allocate and trigger GC.
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
        while let Some(v) = cur.next_item(&mut self.heap)? {
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
        let items = self.extract_iter(o)?;
        self.alloc_and_push_list(items)
    }

    pub fn call_tuple(&mut self, argc: u16, chunk: &crate::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 { return self.alloc_and_push_tuple(Vec::new()); } // `tuple()` is the empty tuple.
        let o = self.pop()?;
        if let Some(items) = self.iter_to_vec_op(o, chunk, slots)? {
            return self.alloc_and_push_tuple(items);
        }
        let items = self.extract_iter(o)?;
        self.alloc_and_push_tuple(items)
    }

}

/* A key's kind to a comparison no user code decides, per tuple position. */
#[derive(PartialEq)]
enum Plain { Num, Str, Tuple(Vec<Plain>) }

fn plain_kind(v: Val, heap: &HeapPool, depth: usize) -> Option<Plain> {
    if v.is_int() || v.is_bool() || v.is_float() && !v.as_float().is_nan() { return Some(Plain::Num); }
    match heap.try_get(v)? {
        HeapObj::Str(_) => Some(Plain::Str),
        HeapObj::Tuple(items) if depth < 4 => items.iter().map(|&x| plain_kind(x, heap, depth + 1)).collect::<Option<Vec<_>>>().map(Plain::Tuple),
        _ => None,
    }
}

fn num(v: Val) -> f64 { if v.is_float() { v.as_float() } else if v.is_bool() { v.as_bool() as i64 as f64 } else { v.as_int() as f64 } }

/* Two plain keys of one kind, ordered as Python orders them. */
fn plain_cmp(a: Val, b: Val, heap: &HeapPool) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if a.is_int() && b.is_int() { return a.as_int().cmp(&b.as_int()); }
    match (heap.try_get(a), heap.try_get(b)) {
        (Some(HeapObj::Str(x)), Some(HeapObj::Str(y))) => x.as_str().cmp(y.as_str()),
        (Some(HeapObj::Tuple(x)), Some(HeapObj::Tuple(y))) => {
            for (&p, &q) in x.iter().zip(y) {
                let o = plain_cmp(p, q, heap);
                if o != Ordering::Equal { return o; }
            }
            x.len().cmp(&y.len())
        }
        _ => num(a).partial_cmp(&num(b)).unwrap_or(Ordering::Equal),
    }
}

/* Stable order of keys compared without user code, None for dunders or mixes. */
fn plain_order(keys: &[Val], reverse: bool, heap: &HeapPool) -> Option<Vec<usize>> {
    let first = plain_kind(*keys.first()?, heap, 0)?;
    // Mixed kinds raise or need the full protocol, which the general path gives.
    for &k in &keys[1..] { if plain_kind(k, heap, 0)? != first { return None; } }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    match first {
        Plain::Num if keys.iter().all(|k| k.is_int()) => {
            let mut v: Vec<(i64, usize)> = keys.iter().map(|k| k.as_int()).zip(0..).collect();
            if reverse { v.sort_by_key(|e| core::cmp::Reverse(e.0)); } else { v.sort_by_key(|e| e.0); }
            return Some(v.into_iter().map(|e| e.1).collect());
        }
        Plain::Num => {
            let mut v: Vec<(f64, usize)> = keys.iter().map(|&k| num(k)).zip(0..).collect();
            // No NaN reaches here, and -0.0 equals 0.0 as it does in Python.
            let by = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(core::cmp::Ordering::Equal);
            if reverse { v.sort_by(|a, b| by(b.0, a.0)); } else { v.sort_by(|a, b| by(a.0, b.0)); }
            return Some(v.into_iter().map(|e| e.1).collect());
        }
        _ if reverse => order.sort_by(|&a, &b| plain_cmp(keys[b], keys[a], heap)),
        _ => order.sort_by(|&a, &b| plain_cmp(keys[a], keys[b], heap)),
    }
    Some(order)
}
