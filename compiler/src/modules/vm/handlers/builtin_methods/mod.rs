/*
Builtin-method descriptor table + dispatcher. Bodies live in per-type files (string/bytes/list/dict/set) as `pub fn`.
Descriptor: name + fn ptr + mutating flag + arity range; dispatcher checks arity uniformly.
*/

mod prelude;
pub mod bytes;
pub mod dict;
pub mod list;
pub mod numeric;
pub mod set;
pub mod string;

use prelude::{VM, Val, VmErr, cold_type};
use crate::s;
use alloc::string::String;

pub type MethodFn = fn(&mut VM, Val, &[Val]) -> Result<(), VmErr>;

pub struct MethodDesc {
    pub ty: &'static str, // receiver type ("str"/"bytes"/"list"/"dict"/"set"); drives lookup.
    pub name: &'static str,
    pub func: MethodFn,
    pub mutating: bool,
    pub min_args: u8,
    pub max_args: u8, // 255 = unbounded (variadic).
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinMethodId(u8);

impl BuiltinMethodId {
    #[inline] pub fn name(self) -> &'static str { ALL_METHODS[self.0 as usize].name }
    pub(crate) fn raw(self) -> u8 { self.0 }
    /* Bounds-checked decode for snapshot restore. */
    pub(crate) fn from_raw(i: u8) -> Option<Self> {
        ((i as usize) < ALL_METHODS.len()).then_some(Self(i))
    }
}

// Builds `ALL_METHODS` grouped by receiver type. `ro`/`rw` = read-only/mutating; `min..max` is the arity (max 255 = variadic).
macro_rules! methods {
    ( $( $ty:literal { $( $name:literal => $func:path, $m:ident, $min:literal .. $max:literal );* $(;)? } )* ) => {
        &[ $( $( MethodDesc {
            ty: $ty, name: $name, func: $func,
            mutating: methods!(@m $m), min_args: $min, max_args: $max,
        } ),* ),* ]
    };
    (@m ro) => { false };
    (@m rw) => { true };
}

// Lookup scans by (ty, name), so order is irrelevant; group however reads best.
pub static ALL_METHODS: &[MethodDesc] = methods! {
    "str" {
        "encode" => string::encode, ro, 0..1;
        "upper" => string::upper, ro, 0..0;
        "lower" => string::lower, ro, 0..0;
        "strip" => string::strip, ro, 0..1;
        "capitalize" => string::capitalize, ro, 0..0;
        "title" => string::title, ro, 0..0;
        "lstrip" => string::lstrip, ro, 0..1;
        "rstrip" => string::rstrip, ro, 0..1;
        "isdigit" => string::isdigit, ro, 0..0;
        "isalpha" => string::isalpha, ro, 0..0;
        "isalnum" => string::isalnum, ro, 0..0;
        "startswith" => string::startswith, ro, 1..3;
        "endswith" => string::endswith, ro, 1..3;
        "find" => string::find, ro, 1..3;
        "count" => string::count, ro, 1..3;
        "split" => string::split, ro, 0..2;
        "join" => string::join, ro, 1..1;
        "replace" => string::replace, ro, 2..3;
        "removeprefix" => string::removeprefix, ro, 1..1;
        "removesuffix" => string::removesuffix, ro, 1..1;
        "splitlines" => string::splitlines, ro, 0..0;
        "partition" => string::partition, ro, 1..1;
        "rpartition" => string::rpartition, ro, 1..1;
        "center" => string::center, ro, 1..2;
        "zfill" => string::zfill, ro, 1..1;
        "rsplit" => string::rsplit, ro, 0..2;
        "casefold" => string::casefold, ro, 0..0;
        "swapcase" => string::swapcase, ro, 0..0;
        "ljust" => string::ljust, ro, 1..2;
        "rjust" => string::rjust, ro, 1..2;
        "expandtabs" => string::expandtabs, ro, 0..1;
        "rfind" => string::rfind, ro, 1..3;
        "index" => string::index, ro, 1..3;
        "rindex" => string::rindex, ro, 1..3;
        "isspace" => string::isspace, ro, 0..0;
        "isupper" => string::isupper, ro, 0..0;
        "islower" => string::islower, ro, 0..0;
        "istitle" => string::istitle, ro, 0..0;
        "format" => string::format, ro, 0..255;
    }
    "bytes" {
        "decode" => bytes::decode, ro, 0..2;
        "hex" => bytes::hex, ro, 0..0;
        "startswith" => bytes::startswith, ro, 1..1;
        "endswith" => bytes::endswith, ro, 1..1;
        "find" => bytes::find, ro, 1..1;
        "index" => bytes::index, ro, 1..1;
        "count" => bytes::count, ro, 1..1;
        "replace" => bytes::replace, ro, 2..2;
        "split" => bytes::split, ro, 1..1;
        "lower" => bytes::lower, ro, 0..0;
        "upper" => bytes::upper, ro, 0..0;
        "strip" => bytes::strip, ro, 0..1;
        "lstrip" => bytes::lstrip, ro, 0..1;
        "rstrip" => bytes::rstrip, ro, 0..1;
        "join" => bytes::join, ro, 1..1;
        "fromhex" => bytes::fromhex, ro, 1..1;
    }
    "list" {
        "index" => list::index, ro, 1..3;
        "count" => list::count, ro, 1..1;
        "copy" => list::copy, ro, 0..0;
        "append" => list::append, rw, 1..1;
        "clear" => list::clear, rw, 0..0;
        "reverse" => list::reverse, rw, 0..0;
        "extend" => list::extend, rw, 1..1;
        "insert" => list::insert, rw, 2..2;
        "remove" => list::remove, rw, 1..1;
        "pop" => list::pop, rw, 0..1;
        "sort" => list::sort, rw, 0..0;
        "__next__" => list::next_method, rw, 0..0;
    }
    "dict" {
        "keys" => dict::keys, ro, 0..0;
        "values" => dict::values, ro, 0..0;
        "items" => dict::items, ro, 0..0;
        "copy" => dict::copy, ro, 0..0;
        "popitem" => dict::popitem, rw, 0..0;
        "get" => dict::get, ro, 1..2;
        "update" => dict::update, rw, 1..1;
        "pop" => dict::pop, rw, 1..2;
        "setdefault" => dict::setdefault, rw, 1..2;
        "fromkeys" => dict::fromkeys, ro, 1..2;
    }
    "set" {
        "add" => set::add, rw, 1..1;
        "remove" => set::remove, rw, 1..1;
        "discard" => set::discard, rw, 1..1;
        "pop" => set::pop, rw, 0..0;
        "clear" => set::clear, rw, 0..0;
        "update" => set::update, rw, 0..255;
        "copy" => set::copy, ro, 0..0;
        "union" => set::union, ro, 0..255;
        "intersection" => set::intersection, ro, 0..255;
        "difference" => set::difference, ro, 0..255;
        "symmetric_difference" => set::symmetric_difference, ro, 1..1;
        "intersection_update" => set::intersection_update, rw, 0..255;
        "difference_update" => set::difference_update, rw, 0..255;
        "symmetric_difference_update" => set::symmetric_difference_update, rw, 1..1;
        "issubset" => set::issubset, ro, 1..1;
        "issuperset" => set::issuperset, ro, 1..1;
        "isdisjoint" => set::isdisjoint, ro, 1..1;
    }
    "int" {
        "bit_length" => numeric::bit_length, ro, 0..0;
        "bit_count" => numeric::bit_count, ro, 0..0;
        "to_bytes" => numeric::to_bytes, ro, 0..2;
        "from_bytes" => numeric::from_bytes, ro, 1..2;
    }
    "float" {
        "is_integer" => numeric::is_integer, ro, 0..0;
    }
    // Appended group: keeps snapshot method ids stable.
    "dict" {
        "clear" => dict::clear, rw, 0..0;
    }
};

#[inline]
pub(crate) fn dispatch_method(vm: &mut VM, id: BuiltinMethodId, recv: Val, pos: &[Val], kw: &[Val]) -> Result<(), VmErr> {
    let m = &ALL_METHODS[id.0 as usize];
    if !kw.is_empty() {
        // `dict.update(**kwargs)` is the one builtin method taking keywords: pack them into a dict and append as a positional, which `dict::update` already merges.
        if m.ty == "dict" && m.name == "update" 
            && let Some(kwd) = VM::pack_kw_dict(&mut vm.heap, kw)? {
                let mut p = alloc::vec::Vec::with_capacity(pos.len() + 1);
                p.extend_from_slice(pos);
                p.push(kwd);
                let result = (m.func)(vm, recv, &p);
                if result.is_ok() { vm.mark_impure(); }
                return result;
            }

        return Err(cold_type("builtin method takes no keyword arguments"));
    }
    let n = pos.len();
    if n < m.min_args as usize || (m.max_args != 255 && n > m.max_args as usize) {
        return Err(arity_error(m.name, m.min_args, m.max_args, n));
    }
    let result = (m.func)(vm, recv, pos);
    if m.mutating && result.is_ok() {
        vm.mark_impure();
    }
    result
}

pub fn lookup_method(ty: &str, attr: &str) -> Option<BuiltinMethodId> {
    // Scan by (ty, name); order-independent, so new methods can be appended anywhere.
    ALL_METHODS
        .iter()
        .position(|m| m.ty == ty && m.name == attr)
        .map(|i| BuiltinMethodId(i as u8))
}

#[cold]
fn arity_error(name: &str, min: u8, max: u8, got: usize) -> VmErr {
    let msg: String = if min == max {
        s!(str name, "() takes ", int min as i64, " arg(s), got ", int got as i64)
    } else if max == 255 {
        s!(str name, "() takes at least ", int min as i64, ", got ", int got as i64)
    } else {
        s!(str name, "() takes ", int min as i64, "..", int max as i64, " args, got ", int got as i64)
    };
    VmErr::TypeMsg(msg)
}
