use core::cell::RefCell;
use alloc::{rc::Rc, string::String, vec::Vec};

use super::super::VM;
use super::super::types::*;

impl<'a> VM<'a> {

    /* Heap-alloc `s` and push the resulting Val. Used by builtins that produce string results. */
    pub(crate) fn alloc_and_push_str(&mut self, s: String) -> Result<(), VmErr> {
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }

    /* Allocate a List from items and push. Centralises the Rc::new(RefCell::new(items)) construction inlined. */
    pub(crate) fn alloc_list(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))
    }

    /* Allocate a List, push it, return Ok. */
    pub(crate) fn alloc_and_push_list(&mut self, items: Vec<Val>) -> Result<(), VmErr> {
        let v = self.alloc_list(items)?;
        self.push(v); Ok(())
    }

    // Allocate a Set from `items` (deduped by content) and push. Mirrors `alloc_and_push_list`.
    pub(crate) fn alloc_and_push_set(&mut self, items: Vec<Val>) -> Result<(), VmErr> {
        let v = self.alloc_set(items)?;
        self.push(v); Ok(())
    }

    /* Allocate a Dict from a DictMap and push. */
    pub(crate) fn alloc_and_push_dict(&mut self, dm: DictMap) -> Result<(), VmErr> {
        let v = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?;
        self.push(v); Ok(())
    }

    /* Allocate a Tuple and push. */
    pub(crate) fn alloc_and_push_tuple(&mut self, items: Vec<Val>) -> Result<(), VmErr> {
        let v = self.tuple_from_items(items)?;
        self.push(v); Ok(())
    }

    fn alloc_set(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        let mut set = ValSet::with_capacity(items.len());
        for v in items { set.insert(v, &self.heap); }
        self.heap.alloc(HeapObj::Set(Rc::new(RefCell::new(set))))
    }

    // Build a tuple Val from items. Shared by the VM and the plugin ABI.
    pub(crate) fn tuple_from_items(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        self.heap.alloc(HeapObj::Tuple(items))
    }

    // Build a set Val from items, rejecting unhashable elements first.
    pub(crate) fn set_from_items(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        for v in &items { self.require_hashable(*v)?; }
        self.alloc_set(items)
    }

    // Build a frozenset Val from items, rejecting unhashable elements first.
    pub(crate) fn frozenset_from_items(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        for v in &items { self.require_hashable(*v)?; }
        let mut set = ValSet::with_capacity(items.len());
        for v in items { set.insert(v, &self.heap); }
        self.heap.alloc(HeapObj::FrozenSet(Rc::new(set)))
    }

    pub fn build_set(&mut self, op: u16) -> Result<(), VmErr> {
        let items = self.pop_n(op as usize)?;
        let val = self.set_from_items(items)?;
        self.push(val); Ok(())
    }

    pub fn build_slice(&mut self, op: u16) -> Result<(), VmErr> {
        let step = if op == 3 { self.pop()? } else { Val::none() };
        let stop = self.pop()?;
        let start = self.pop()?;
        let val = self.heap.alloc(HeapObj::Slice(start, stop, step))?;
        self.push(val); Ok(())
    }

    pub fn unpack_ex(&mut self, op: u16) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(cold_type("cannot unpack non-iterable")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v) => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            // Range materialises to its ints, with the same budget cap as `*` spread.
            HeapObj::Range(..) => self.iter_to_vec_for_spread(obj)?,
            HeapObj::Str(s) => {
                let s = s.clone();
                self.str_to_char_vals(&s)?
            }
            _ => return Err(cold_type("cannot unpack non-iterable")),
        };
        let before = (op >> 8) as usize;
        let after = (op & 0xFF) as usize;
        if items.len() < before + after {
            return Err(cold_value("not enough values to unpack"));
        }
        let mid = items.len() - after;
        for &v in items[mid..].iter().rev() { self.push(v); }
        let star = self.alloc_list(items[before..mid].to_vec())?;
        self.push(star);
        for &v in items[..before].iter().rev() { self.push(v); }
        Ok(())
    }

    // Operand packs kw<<8 | pos; keep counts distinct.
    pub fn call_dict(&mut self, op: u16) -> Result<(), VmErr> {
        let pos = (op & 0xFF) as usize;
        let kw = (op >> 8) as usize;
        if pos > 1 {
            return Err(cold_type("dict expected at most 1 argument"));
        }
        // Keyword pairs sit above the positional source.
        let kw_flat = self.pop_n(kw * 2)?;
        let mut dm = if pos == 1 {
            let src = self.pop()?;
            self.dict_from_source(src)?
        } else {
            DictMap::with_capacity(kw)
        };
        for pair in kw_flat.chunks(2) { dm.insert(pair[0], pair[1], &self.heap); }
        self.alloc_and_push_dict(dm)
    }

    // dict(mapping) copies; dict(iterable) builds from pairs.
    fn dict_from_source(&mut self, src: Val) -> Result<DictMap, VmErr> {
        if let Some(HeapObj::Dict(rc)) = self.heap.try_get(src) {
            let pairs: Vec<(Val, Val)> = rc.borrow().iter().collect();
            return Ok(DictMap::from_pairs(pairs, &self.heap));
        }
        let items = self.extract_iter(src, true)?;
        let mut dm = DictMap::with_capacity(items.len());
        for item in items {
            let pair = self.extract_iter(item, true)?;
            if pair.len() != 2 {
                return Err(cold_value("dictionary update sequence element has length != 2"));
            }
            self.require_hashable(pair[0])?;
            dm.insert(pair[0], pair[1], &self.heap);
        }
        Ok(dm)
    }

    pub fn call_set(&mut self, op: u16) -> Result<(), VmErr> {
        if op == 0 {
            let val = self.alloc_set(Vec::new())?;
            self.push(val);
        } else {
            let o = self.pop()?;
            let src = self.extract_iter(o, true)?;
            let val = self.alloc_set(src)?;
            self.push(val);
        }
        Ok(())
    }

    /* `frozenset()` | `frozenset(iter)`, construct an immutable, hashable set from an iterable. Without args returns the empty frozenset. */
    pub fn call_frozenset(&mut self, argc: u16) -> Result<(), VmErr> {
        let args = self.pop_n(argc as usize)?;
        let items: Vec<Val> = match args.len() {
            0 => Vec::new(),
            1 => self.iter_to_vec_general(args[0])?,
            _ => return Err(cold_type("frozenset() takes 0 or 1 argument")),
        };
        let v = self.frozenset_from_items(items)?;
        self.push(v); Ok(())
    }

    /* `bytes()`, empty, `n` zero bytes, iter of ints (0..=255), or `(str, encoding)`. Encodings limited to utf-8/utf8/ascii; unknown ones error so mismatches aren't silent. */
    pub fn call_bytes(&mut self, argc: u16) -> Result<(), VmErr> {
        let args = self.pop_n(argc as usize)?;
        let buf: Vec<u8> = match args.len() {
            0 => Vec::new(),
            1 => {
                let a = args[0];
                if a.is_int() {
                    let n = a.as_int();
                    if n < 0 { return Err(cold_value("negative count")); }
                    // Length is user-controlled; cap it against the heap budget so a huge count errors instead of aborting in the allocator.
                    if n as usize > self.heap.limit() { return Err(cold_heap()); }
                    alloc::vec![0u8; n as usize]
                } else if a.is_heap() {
                    if let HeapObj::Bytes(b) = self.heap.get(a) {
                        b.clone()
                    } else {
                        let items = self.iter_to_vec_general(a)?;
                        let mut out = Vec::with_capacity(items.len());
                        for v in items {
                            if !v.is_int() {
                                return Err(cold_type("bytes() iterable must contain ints"));
                            }
                            let n = v.as_int();
                            if !(0..=255).contains(&n) {
                                return Err(cold_value("bytes must be in range(0, 256)"));
                            }
                            out.push(n as u8);
                        }
                        out
                    }
                } else {
                    return Err(cold_type("bytes() requires an int, an iterable of ints, or (str, encoding)"));
                }
            }
            2 => {
                // `bytes(s, "utf-8")`, string encoding form.
                let (s, enc) = (args[0], args[1]);
                let Some(HeapObj::Str(text)) = self.heap.try_get(s).cloned() else {
                    return Err(cold_type("bytes() first argument must be a string when encoding is given"));
                };
                let Some(HeapObj::Str(encoding)) = self.heap.try_get(enc) else {
                    return Err(cold_type("bytes() encoding must be a string"));
                };
                match encoding.as_str() {
                    "utf-8" | "utf8" => text.into_bytes(),
                    "ascii" => {
                        if !text.is_ascii() {
                            return Err(cold_value("'ascii' codec can't encode non-ASCII characters"));
                        }
                        text.into_bytes()
                    }
                    _ => return Err(cold_value("unsupported encoding (expected 'utf-8' or 'ascii')")),
                }
            }
            _ => return Err(cold_type("bytes() takes at most 2 arguments")),
        };
        let v = self.heap.alloc(HeapObj::Bytes(buf))?;
        self.push(v); Ok(())
    }
}
