use super::types::{Val, HeapObj, HeapPool, VmErr, EQ_DEPTH_MAX};
use crate::parser::{SSAChunk, Value};

use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};

use super::{lower::Code, sites::Site};

/* One chunk's caches, each running frame holds one and returns it here. */
#[derive(Default)]
pub struct CachePool {
    /* The chunk's lowered code, which every frame running it shares. */
    pub code: Option<Rc<Code>>,
    /* The chunk's constants as values, made once and kept for good. */
    pub consts: Option<Box<[Val]>>,
    /* Set once a frame ran the chunk, its next call lowers it. */
    pub ran: bool,
    /* Back-edges and resumes its unlowered frames took, a resume past `HOT_LOOP` lowers it. */
    pub heat: u32,
    active: u32,
    // Boxed, so handing a cache to a frame and back moves one pointer.
    main: Option<Box<OpcodeCache>>,
    /* Only a recursion fills it, so a plain call never allocates here. */
    #[allow(clippy::vec_box)]
    spare: Vec<Box<OpcodeCache>>,
}

impl CachePool {
    pub fn take(&mut self, len: usize) -> Box<OpcodeCache> {
        self.active += 1;
        self.main.take().or_else(|| self.spare.pop()).unwrap_or_else(|| Box::new(OpcodeCache::new(len)))
    }

    /* The outermost frame returning keeps one cache and frees the rest. */
    pub fn put(&mut self, cache: Box<OpcodeCache>) {
        self.active -= 1;
        if self.active == 0 {
            self.spare.clear();
            self.main = Some(cache);
        } else if self.main.is_none() {
            self.main = Some(cache);
        } else {
            self.spare.push(cache);
        }
    }

    pub fn caches(&self) -> impl Iterator<Item = &OpcodeCache> { self.main.iter().chain(&self.spare).map(|c| &**c) }
}

/* A chunk's constants as values, each string interned. */
pub(crate) fn const_vals(chunk: &SSAChunk, heap: &mut HeapPool) -> Result<Box<[Val]>, VmErr> {
    let mut out = Vec::with_capacity(chunk.constants.len());
    for c in &chunk.constants {
        out.push(match c {
            // A wide literal that fits inline demotes so hash and eq stay in sync with the short form.
            Value::Int(i) => heap.int(*i as i128)?,
            Value::LongInt(i) => heap.int(*i)?,
            Value::Float(f) => Val::float(*f),
            Value::Bool(b) => Val::bool(*b),
            Value::None => Val::none(),
            Value::Str(s) => heap.intern_str(s)?,
            Value::Bytes(b) => heap.alloc(HeapObj::Bytes(b.clone()))?,
        });
    }
    Ok(out.into_boxed_slice())
}

pub struct OpcodeCache {
    len: usize,
    /* Attribute and operator sites by ip, made on the first one learned. */
    sites: Vec<Site>,
}

impl OpcodeCache {
    pub fn new(len: usize) -> Self { Self { len, sites: Vec::new() } }

    /* The site at `ip`. */
    #[inline]
    pub(crate) fn site(&self, ip: usize) -> Site { self.sites.get(ip).copied().unwrap_or_default() }

    pub(crate) fn set_site(&mut self, ip: usize, site: Site) {
        if self.sites.is_empty() { self.sites = vec![Site::Empty; self.len]; }
        if let Some(s) = self.sites.get_mut(ip) { *s = site; }
    }

    pub(crate) fn site_roots(&self) -> impl Iterator<Item = Val> + '_ { self.sites.iter().flat_map(|s| s.roots()) }
}

// Template memoization for pure functions.

fn args_match(e: &TplEntry, args: &[Val], owner: Val, h: u64, heap: &HeapPool) -> bool {
    e.hash == h
    && e.owner.0 == owner.0
    && e.args.len() == args.len()
    && e.args.iter().zip(args).all(|(&a, &b)| key_eq(a, b, heap, 0))
}

// `owner` is the function when it has defaults, so other defaults never share a result.
struct TplEntry { args: Vec<Val>, owner: Val, result: Val, hash: u64 }

fn mix(h: u64, x: u64) -> u64 { (h ^ x).wrapping_mul(0x100000001b3) }

fn hash_args(args: &[Val], heap: &HeapPool) -> u64 {
    args.iter().fold(0xcbf29ce484222325, |h, &v| mix(h, if v.is_heap() { key_hash(v, heap, 0) } else { v.0 }))
}

/* Strings, bytes and tuples hash by content, everything else by its bits. */
#[inline]
fn key_hash(v: Val, heap: &HeapPool, depth: usize) -> u64 {
    let fold = |seed: u64, bytes: &[u8]| bytes.iter().fold(seed, |h, &b| mix(h, b as u64));
    if !v.is_heap() || depth > EQ_DEPTH_MAX { return v.0; }
    match heap.try_get(v) {
        Some(HeapObj::Str(s)) => fold(1, s.as_bytes()),
        Some(HeapObj::Bytes(b)) => fold(2, b),
        Some(HeapObj::Tuple(items)) => items.iter().fold(3, |h, &x| mix(h, key_hash(x, heap, depth + 1))),
        _ => v.0,
    }
}

/* Strict equality, so `1`, `1.0` and `True` stay apart. */
#[inline]
fn key_eq(a: Val, b: Val, heap: &HeapPool, depth: usize) -> bool {
    if a.0 == b.0 { return true; }
    if !a.is_heap() || !b.is_heap() || depth > EQ_DEPTH_MAX { return false; }
    match (heap.try_get(a), heap.try_get(b)) {
        (Some(HeapObj::Str(x)), Some(HeapObj::Str(y))) => x == y,
        (Some(HeapObj::Bytes(x)), Some(HeapObj::Bytes(y))) => x == y,
        (Some(HeapObj::Tuple(x)), Some(HeapObj::Tuple(y))) => x.len() == y.len() && x.iter().zip(y).all(|(&p, &q)| key_eq(p, q, heap, depth + 1)),
        _ => false,
    }
}

/* Immutable all the way down, so nothing can change behind a cached result. */
pub(crate) fn deeply_immutable(v: Val, heap: &HeapPool, depth: usize) -> bool {
    if !v.is_heap() { return true; }
    // Post-call args aren't rooted, so the body may have freed one, a freed slot (None) is not memoizable.
    match heap.try_get(v) {
        Some(HeapObj::Str(_) | HeapObj::Bytes(_) | HeapObj::LongInt(_) | HeapObj::Range(..) | HeapObj::NativeFn(_) | HeapObj::Type(_)) => true,
        Some(HeapObj::Tuple(items)) => depth < EQ_DEPTH_MAX && items.iter().all(|&x| deeply_immutable(x, heap, depth + 1)),
        Some(HeapObj::FrozenSet(items)) => depth < EQ_DEPTH_MAX && items.iter().all(|&x| deeply_immutable(x, heap, depth + 1)),
        _ => false,
    }
}

/* Disable a fi's memo after this many consecutive lookup misses, the scan tax outweighs stale hope. */
const MISS_LIMIT: u64 = 256;

/* `meta` holds SEEN first-run marks, then the consecutive misses of each fi. */
const SEEN: usize = 32;

// Indexed by dense `fi`, Vec gives O(1) lookup with no HashMap monomorphization.
pub struct Templates { slots: Vec<Vec<TplEntry>>, meta: Vec<u64> }

impl Templates {
    pub fn new() -> Self { Self { slots: Vec::new(), meta: Vec::new() } }

    /* Drops every table, nothing to do when no call reached one. */
    pub fn clear(&mut self) { if !self.slots.is_empty() || !self.meta.is_empty() { *self = Self::new(); } }

    pub fn dead(&self, fi: usize) -> bool {
        self.meta.get(SEEN + fi).is_some_and(|&m| m >= MISS_LIMIT)
    }

    /* Counts calls the cache could not serve, a useful one resets the count. */
    fn note(&mut self, fi: usize, useful: bool) {
        if self.meta.len() <= SEEN + fi { self.meta.resize(SEEN + fi + 1, 0); }
        if useful { self.meta[SEEN + fi] = 0; } else { self.meta[SEEN + fi] += 1; }
    }

    pub fn lookup(&mut self, fi: usize, args: &[Val], owner: Val, heap: &HeapPool) -> Option<Val> {
        let entries = self.slots.get(fi)?;
        if entries.is_empty() || self.dead(fi) { return None; }
        let h = hash_args(args, heap);
        let hit = entries.iter()
            .find(|e| args_match(e, args, owner, h, heap))
            .map(|e| e.result);
        self.note(fi, hit.is_some());
        // Reclaim the dead table, entries would otherwise stay GC roots forever.
        if self.dead(fi) { self.slots[fi] = Vec::new(); }
        hit
    }

    /* The key hash on its second run, so a key that never repeats allocates nothing. */
    pub fn admit(&mut self, fi: usize, args: &[Val], owner: Val, heap: &HeapPool) -> Option<u64> {
        if self.dead(fi) || self.slots.get(fi).is_some_and(|v| v.len() >= 256) { return None; }
        let h = hash_args(args, heap);
        // Fibonacci hashing, so keys apart only in high bits like `0.0` and `-0.0` land apart.
        let mark = (h ^ owner.0 ^ fi as u64).wrapping_mul(0x9e3779b97f4a7c15);
        if self.meta.len() < SEEN { self.meta.resize(SEEN, 0); }
        let seen = &mut self.meta[(mark >> (64 - SEEN.ilog2())) as usize];
        // A first sighting counts as a miss, so unrepeated keys end the memo.
        if *seen != mark { *seen = mark; self.note(fi, false); return None; }
        self.note(fi, true);
        Some(h)
    }

    pub fn holds(&self, fi: usize) -> bool { self.slots.get(fi).is_some_and(|v| !v.is_empty()) }

    pub fn insert(&mut self, fi: usize, args: &[Val], owner: Val, result: Val, h: u64) {
        if self.slots.len() <= fi { self.slots.resize_with(fi + 1, Vec::new); }
        self.slots[fi].push(TplEntry { args: args.to_vec(), owner, result, hash: h });
    }

    pub fn mark_all(&self, heap: &mut HeapPool) {
        for slot in &self.slots {
            for e in slot {
                for &v in &e.args { heap.mark(v); }
                heap.mark(e.owner);
                heap.mark(e.result);
            }
        }
    }
}
