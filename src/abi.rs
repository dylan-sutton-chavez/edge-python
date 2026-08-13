use alloc::{string::String, vec::Vec};

pub use wasm_abi::{nan_box, WireValue, EDGE_ABI_VERSION, MAX_WIRE_DEPTH, TAG_INVALID};

/* Sealed op codes / tags / error kinds with a `from_u32` reverse map from one variant list, so the two can't drift. Values mirror `wasm_abi::*` byte-for-byte per docs/content/reference/abi.md, extend host modules via new Op values, never new imports. */
macro_rules! abi_enum {
    ($name:ident { $($variant:ident = $value:path),+ $(,)? }) => {
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u32)]
        pub enum $name { $($variant = $value),+ }
        impl $name {
            pub fn from_u32(v: u32) -> Option<Self> {
                match v { $($value => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

/* Op codes (sealed), values mirror `wasm_abi::op::*`. */
abi_enum!(Op {
    Call = wasm_abi::op::CALL,
    GetAttr = wasm_abi::op::GET_ATTR,
    SetAttr = wasm_abi::op::SET_ATTR,
    GetItem = wasm_abi::op::GET_ITEM,
    SetItem = wasm_abi::op::SET_ITEM,
    Len = wasm_abi::op::LEN,
    Iter = wasm_abi::op::ITER,
    IterNext = wasm_abi::op::ITER_NEXT,
    NewDict = wasm_abi::op::NEW_DICT,
    NewList = wasm_abi::op::NEW_LIST,
    TypeOf = wasm_abi::op::TYPE_OF,
    NewTuple = wasm_abi::op::NEW_TUPLE,
    NewSet = wasm_abi::op::NEW_SET,
    NewFrozenSet = wasm_abi::op::NEW_FROZENSET,
});

/* Tags (sealed), values mirror `wasm_abi::tag::*`. */
abi_enum!(Tag {
    None = wasm_abi::tag::NONE,
    Bool = wasm_abi::tag::BOOL,
    Int = wasm_abi::tag::INT,
    Float = wasm_abi::tag::FLOAT,
    // UTF-8 bytes, encoder builds a str, decoder returns its bytes.
    Bytes = wasm_abi::tag::BYTES,
    // Opaque bytes, no UTF-8 validation, maps to Python `bytes`.
    Raw = wasm_abi::tag::RAW,
    // TLV composites, payloads defined in `wasm_abi::WireValue`.
    List = wasm_abi::tag::LIST,
    Dict = wasm_abi::tag::DICT,
});

/* Error kinds (sealed), values mirror `wasm_abi::error_kind::*`. */
abi_enum!(ErrorKind {
    Type = wasm_abi::error_kind::TYPE,
    Value = wasm_abi::error_kind::VALUE,
    Runtime = wasm_abi::error_kind::RUNTIME,
    Attribute = wasm_abi::error_kind::ATTRIBUTE,
    Index = wasm_abi::error_kind::INDEX,
    Key = wasm_abi::error_kind::KEY,
    Custom = wasm_abi::error_kind::CUSTOM,
});

/* Handle table */

// Handle slot, rc=0 = free. Exposed handle = index+1 (0 reserved as invalid).
struct HandleSlot {
    // Raw Val bits, opaque to the ABI, encode/decode go through classify_*.
    val: u64,
    rc: u32,
}

// Refcounted handle -> Val-bits map, cleared between script runs.
pub struct HandleTable {
    slots: Vec<HandleSlot>,
    free_list: Vec<u32>,
}

impl Default for HandleTable {
    fn default() -> Self { Self::new() }
}

impl HandleTable {
    pub const fn new() -> Self {
        Self { slots: Vec::new(), free_list: Vec::new() }
    }

    // Reset to empty state. Called by the host between runs.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.free_list.clear();
    }

    // Register a value. Returns a fresh handle (rc=1).
    pub fn put(&mut self, val: u64) -> u32 {
        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = HandleSlot { val, rc: 1 };
            idx + 1
        } else {
            self.slots.push(HandleSlot { val, rc: 1 });
            self.slots.len() as u32
        }
    }

    // Look up a value by handle, or `None` if invalid / freed.
    pub fn get(&self, h: u32) -> Option<u64> {
        if h == 0 { return None; }
        self.slots.get((h - 1) as usize)
            .filter(|s| s.rc > 0)
            .map(|s| s.val)
    }

    // Decrements rc, frees slot at 0. Safe against double-release.
    pub fn release(&mut self, h: u32) {
        if h == 0 { return; }
        let idx = (h - 1) as usize;
        if let Some(slot) = self.slots.get_mut(idx)
            && slot.rc > 0
        {
            slot.rc -= 1;
            if slot.rc == 0 { self.free_list.push(idx as u32); }
        }
    }
}

/* Error stash */

// Single-slot error stash, populated by dispatch failures / edge_throw, drained by edge_take_error.
#[derive(Default)]
pub struct ErrorStash(Option<(u32, String)>);

impl ErrorStash {
    pub const fn new() -> Self { Self(None) }
    pub fn clear(&mut self) { self.0 = None; }

    // Replace any pending error with `(kind, msg)`.
    pub fn set(&mut self, kind: u32, msg: String) {
        self.0 = Some((kind, msg));
    }

    // Stash a typed error.
    pub fn set_typed(&mut self, kind: ErrorKind, msg: String) {
        self.0 = Some((kind as u32, msg));
    }

    // Take the error if present.
    pub fn take(&mut self) -> Option<(u32, String)> { self.0.take() }

    // Peeks without consuming, lets edge_take_error retry on buffer-too-small.
    pub fn peek(&self) -> Option<(u32, &str)> {
        self.0.as_ref().map(|(k, m)| (*k, m.as_str()))
    }
}

/* Primitive codec helpers */

// edge_encode outcome, Direct (Val bits), AllocStr / AllocBytes / AllocLongInt (host alloc), Composite (host builds recursively), or Invalid.
pub enum EncodeRequest<'a> {
    Direct(u64),
    AllocStr(&'a str),
    AllocBytes(&'a [u8]),
    AllocLongInt(i128),
    Composite(WireValue),
    Invalid,
}

// Inline range for Val::int (47-bit signed), values outside go to HeapObj::LongInt.
const INLINE_INT_MIN: i128 = -0x0000_8000_0000_0000i64 as i128;
const INLINE_INT_MAX: i128 =  0x0000_7FFF_FFFF_FFFFi64 as i128;

// Val bits for an inline int, `None` when the value needs a LongInt.
pub fn inline_int_bits(i: i128) -> Option<u64> {
    use nan_box::*;
    (INLINE_INT_MIN..=INLINE_INT_MAX).contains(&i).then_some(TAG_INT | ((i as i64) as u64 & INT_PAYLOAD_MASK))
}

// Maps (tag, bytes) to EncodeRequest using the sealed `nan_box` layout.
pub fn classify_encode(tag: u32, bytes: &[u8]) -> EncodeRequest<'_> {
    use nan_box::*;

    match Tag::from_u32(tag) {
        Some(Tag::None) => EncodeRequest::Direct(TAG_NONE),
        Some(Tag::Bool) => {
            let b = !bytes.is_empty() && bytes[0] != 0;
            EncodeRequest::Direct(if b { TAG_TRUE } else { TAG_FALSE })
        }
        Some(Tag::Int) => {
            // Wire format is 16 bytes (i128) covering Edge Python's full int range.
            if bytes.len() != 16 { return EncodeRequest::Invalid; }
            let mut buf = [0u8; 16];
            buf.copy_from_slice(bytes);
            let i = i128::from_le_bytes(buf);
            // Fits in 47-bit inline range -> emit as Val::int directly, else heap-alloc LongInt.
            match inline_int_bits(i) {
                Some(bits) => EncodeRequest::Direct(bits),
                None => EncodeRequest::AllocLongInt(i),
            }
        }
        Some(Tag::Float) => {
            if bytes.len() != 8 { return EncodeRequest::Invalid; }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(bytes);
            EncodeRequest::Direct(f64::from_le_bytes(buf).to_bits())
        }
        Some(Tag::Bytes) => match core::str::from_utf8(bytes) {
            Ok(s) => EncodeRequest::AllocStr(s),
            Err(_) => EncodeRequest::Invalid,
        },
        // Raw bytes bypass UTF-8 validation and become a Python `bytes`.
        Some(Tag::Raw) => EncodeRequest::AllocBytes(bytes),
        Some(Tag::List) | Some(Tag::Dict) => match WireValue::decode_body(tag, bytes) {
            Some(w) => EncodeRequest::Composite(w),
            None => EncodeRequest::Invalid,
        },
        None => EncodeRequest::Invalid,
    }
}

// edge_decode outcome, Primitive (ready bytes), Heap (host materializes), or Invalid.
pub enum DecodeBits {
    Primitive { tag: u32, bytes: PrimitiveBytes },
    Heap,
    Invalid,
}

pub enum PrimitiveBytes {
    None,
    Bool(u8),
    Eight([u8; 8]),
    Sixteen([u8; 16]),
}

// Classifies Val bits, Heap routes the host to HeapPool.
pub fn classify_decode(val_bits: u64) -> DecodeBits {
    use nan_box::*;

    // Any non-QNAN-tagged pattern is a float.
    if (val_bits & QNAN) != QNAN {
        return DecodeBits::Primitive {
            tag: Tag::Float as u32,
            bytes: PrimitiveBytes::Eight(f64::from_bits(val_bits).to_le_bytes()),
        };
    }
    // Int, QNAN|SIGN with payload. Sign-extend the 47-bit payload to i128 (wire width).
    if (val_bits & (QNAN | SIGN)) == TAG_INT {
        let raw = (val_bits & INT_PAYLOAD_MASK) as i64;
        let sign_extended_i64 = (raw << 16) >> 16;
        let as_i128 = sign_extended_i64 as i128;
        return DecodeBits::Primitive {
            tag: Tag::Int as u32,
            bytes: PrimitiveBytes::Sixteen(as_i128.to_le_bytes()),
        };
    }
    // Singletons and heap handles.
    let lower = val_bits & 0xF;
    if (val_bits & QNAN) == QNAN && (val_bits & SIGN) == 0 {
        if val_bits == TAG_NONE {
            return DecodeBits::Primitive {
                tag: Tag::None as u32, bytes: PrimitiveBytes::None,
            };
        }
        if val_bits == TAG_TRUE {
            return DecodeBits::Primitive {
                tag: Tag::Bool as u32, bytes: PrimitiveBytes::Bool(1),
            };
        }
        if val_bits == TAG_FALSE {
            return DecodeBits::Primitive {
                tag: Tag::Bool as u32, bytes: PrimitiveBytes::Bool(0),
            };
        }
        if lower >= 4 {
            return DecodeBits::Heap;
        }
    }
    DecodeBits::Invalid
}
