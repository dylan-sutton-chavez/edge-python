use core::cmp::Ordering;

use super::{DictMap, HeapObj, HeapPool, Val, ValSet, as_i128, as_long_int};

/* 2^127 exactly, the saturation guard for f64-to-i128 casts. */
const TWO_POW_127: f64 = 170141183460469231731687303715884105728.0;

pub(crate) fn eq_seq(a: &[Val], b: &[Val], eq: impl Fn(Val,Val)->bool) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x,y)| eq(*x,*y))
}
pub(crate) fn eq_dict(a: &DictMap, b: &DictMap, heap: &HeapPool, eq: impl Fn(Val,Val)->bool) -> bool {
    a.len() == b.len() && a.iter().all(|(k,v)| b.get(&k, heap).is_some_and(|&v2| eq(v,v2)))
}
/* Content set-equality, same size and every element of `a` content-matches one in `b`. */
pub(crate) fn eq_set(a: &ValSet, b: &ValSet, eq: impl Fn(Val,Val)->bool) -> bool {
    a.len() == b.len() && a.iter().all(|&x| b.iter().any(|&y| eq(x, y)))
}

/* Recursion cap so self-referential containers fall back instead of overflowing the stack. */
const EQ_DEPTH_MAX: usize = 100;

pub fn eq_vals_with_heap(a: Val, b: Val, heap: &HeapPool) -> bool {
    eq_vals_depth(a, b, heap, 0)
}

/* Content hash, consistent with eq_vals_with_heap. Values that compare equal hash equal (numeric unified). */
pub fn hash_val_with_heap(v: Val, heap: &HeapPool) -> u64 {
    hash_depth(v, heap, 0)
}
fn hash_depth(v: Val, heap: &HeapPool, depth: usize) -> u64 {
    use core::hash::Hasher;
    let mut h = crate::util::hash::FxHasher::default();
    // Numeric unification, int / bool / integral-float / in-range LongInt all hash as the same i64.
    if v.is_int() { h.write_i64(v.as_int()); return h.finish(); }
    if v.is_bool() { h.write_i64(v.as_bool() as i64); return h.finish(); }
    if v.is_float() {
        let f = v.as_float();
        // Integral floats hash exactly as the equal int or LongInt, so equal keys probe identically.
        match float_exact_i128(f) {
            Some(i) => write_i128(&mut h, i),
            None => h.write_u64(f.to_bits()),
        }
        return h.finish();
    }
    if !v.is_heap() || depth > EQ_DEPTH_MAX { h.write_u64(v.0); return h.finish(); }
    match heap.get(v) {
        HeapObj::LongInt(i) => write_i128(&mut h, *i),
        HeapObj::Str(s) => { h.write_u8(1); h.write(s.as_bytes()); }
        HeapObj::Bytes(b) => { h.write_u8(2); h.write(b); }
        HeapObj::Tuple(t) => { h.write_u8(3); h.write_usize(t.len()); for &e in t { h.write_u64(hash_depth(e, heap, depth + 1)); } }
        // Order-independent so equal frozensets (any order/handle) hash equal.
        HeapObj::FrozenSet(s) => { h.write_u8(4); let acc = s.iter().fold(0u64, |a, &e| a.wrapping_add(hash_depth(e, heap, depth + 1))); h.write_u64(acc); }
        _ => h.write_u64(v.0),
    }
    h.finish()
}

/* i64-range values hash like the equal inline int, wider values hash their two halves. */
fn write_i128(h: &mut crate::util::hash::FxHasher, i: i128) {
    use core::hash::Hasher;
    match i64::try_from(i) {
        Ok(n) => h.write_i64(n),
        Err(_) => { h.write_u64(i as u64); h.write_u64((i >> 64) as u64); }
    }
}

/* Exact i128 view of an integral in-range f64, `as` saturates so the 2^127 bound is mandatory. */
pub(crate) fn float_exact_i128(f: f64) -> Option<i128> {
    if f.abs() < TWO_POW_127 && libm::trunc(f) == f { Some(f as i128) } else { None }
}

/* Exact float vs wide-int ordering, None on NaN. The float converts to i128 since the wide int would round in f64. */
pub(crate) fn float_cmp_int(f: f64, i: i128) -> Option<Ordering> {
    if f.is_nan() { return None; }
    if let Some(fi) = float_exact_i128(f) { return Some(fi.cmp(&i)); }
    // Out-of-range floats exceed any i128, fractional in-range floats break a truncation tie by sign.
    if f.abs() >= TWO_POW_127 { return Some(if f > 0.0 { Ordering::Greater } else { Ordering::Less }); }
    let t = libm::trunc(f) as i128;
    Some(match t.cmp(&i) {
        Ordering::Equal => if f > 0.0 { Ordering::Greater } else { Ordering::Less },
        o => o,
    })
}

/* f64 view of any numeric Val (int/bool/float/LongInt), None for non-numerics. */
pub(crate) fn num_as_f64(v: Val, heap: &HeapPool) -> Option<f64> {
    if v.is_float() { Some(v.as_float()) }
    else if v.is_int() { Some(v.as_int() as f64) }
    else if v.is_bool() { Some(v.as_bool() as i64 as f64) }
    else if v.is_heap() { if let HeapObj::LongInt(i) = heap.get(v) { Some(*i as f64) } else { None } }
    else { None }
}

fn eq_vals_depth(a: Val, b: Val, heap: &HeapPool, depth: usize) -> bool {
    // Past the cap fall back to identity, cyclic structures terminate.
    if depth > EQ_DEPTH_MAX { return a.0 == b.0; }

    // Unify all int-flavoured pairs through i128 (LongInt, inline int, bool).
    if let (Some(ai), Some(bi)) = (as_i128(a, heap), as_i128(b, heap)) {
        return ai == bi;
    }

    if a.is_float() && let Some(bi) = as_long_int(b, heap) {
        return float_cmp_int(a.as_float(), bi) == Some(Ordering::Equal);
    }
    if b.is_float() && let Some(ai) = as_long_int(a, heap) {
        return float_cmp_int(b.as_float(), ai) == Some(Ordering::Equal);
    }

    // One side is a float here (all-integer handled above), compare numerically so float unifies with int/bool, e.g. `1.0 == True`.
    if let (Some(af), Some(bf)) = (num_as_f64(a, heap), num_as_f64(b, heap)) {
        return af == bf;
    }

    if !a.is_heap() || !b.is_heap() {
        return a.0 == b.0;
    }

    // A heap object equals itself, short-circuits self-referential containers before the element walk.
    if a.0 == b.0 { return true; }

    let d = depth + 1;
    match (heap.get(a), heap.get(b)) {
        (HeapObj::Str(x), HeapObj::Str(y)) => x == y,
        (HeapObj::Bytes(x), HeapObj::Bytes(y)) => x == y,
        (HeapObj::Tuple(x), HeapObj::Tuple(y)) => eq_seq(x, y, |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::List(x), HeapObj::List(y)) => eq_seq(&x.borrow(), &y.borrow(), |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::Set(x), HeapObj::Set(y)) => eq_set(&x.borrow(), &y.borrow(), |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::FrozenSet(x), HeapObj::FrozenSet(y)) => eq_set(x, y, |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::Set(x), HeapObj::FrozenSet(y)) => eq_set(&x.borrow(), y, |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::FrozenSet(x), HeapObj::Set(y)) => eq_set(x, &y.borrow(), |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::Dict(x), HeapObj::Dict(y)) => eq_dict(&x.borrow(), &y.borrow(), heap, |a,b| eq_vals_depth(a, b, heap, d)),
        (HeapObj::Type(x), HeapObj::Type(y)) => x == y, // by name, interning also makes `is` hold
        (HeapObj::Range(s1,e1,t1), HeapObj::Range(s2,e2,t2)) => {
            // Python semantics, equal length, then matching start/step only when non-empty.
            let (l1, l2) = (range_len(*s1,*e1,*t1), range_len(*s2,*e2,*t2));
            l1 == l2 && (l1 == 0 || (s1 == s2 && (l1 == 1 || t1 == t2)))
        }
        // Cross-type comparisons fall through to false. Notably `bytes == str` is False, even when the bytes are valid UTF-8 of the str.
        _ => false,
    }
}

/* Count of values range(start, stop, step) yields, step is never zero. */
fn range_len(s: i64, e: i64, t: i64) -> i128 {
    let (lo, hi, step) = if t > 0 { (s as i128, e as i128, t as i128) } else { (e as i128, s as i128, -(t as i128)) };
    if hi > lo { (hi - lo + step - 1) / step } else { 0 }
}
