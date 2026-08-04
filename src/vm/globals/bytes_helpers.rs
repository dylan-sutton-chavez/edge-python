use crate::s;

use alloc::{string::String, vec::Vec};

use super::super::VM;
use super::super::types::*;

impl<'a> VM<'a> {

    /* `bytes_fromhex(s)`, hex string -> bytes; tolerates whitespace, errors on odd length / non-hex. */
    pub fn call_bytes_fromhex(&mut self) -> Result<(), VmErr> {
        let v = self.pop()?;
        let s = self.str_of(v, "bytes_fromhex() argument must be a string")?;
        let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if !cleaned.len().is_multiple_of(2) {
            return Err(cold_value("non-hexadecimal number or odd length"));
        }
        let mut out = Vec::with_capacity(cleaned.len() / 2);
        let bytes = cleaned.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let hi = (bytes[i] as char).to_digit(16).ok_or(cold_value("non-hexadecimal digit found"))?;
            let lo = (bytes[i + 1] as char).to_digit(16).ok_or(cold_value("non-hexadecimal digit found"))?;
            out.push(((hi << 4) | lo) as u8);
            i += 2;
        }
        let v = self.heap.alloc(HeapObj::Bytes(out))?;
        self.push(v); Ok(())
    }

    /* `int_from_bytes(b, byteorder)`, unsigned bytes -> int ("big"/"little"). Up to 8 bytes (u64); values above 47-bit Val auto-promote to LongInt. */
    pub fn call_int_from_bytes(&mut self) -> Result<(), VmErr> {
        let order = self.pop()?;
        let v = self.pop()?;
        let buf = match self.heap.try_get(v) {
            Some(HeapObj::Bytes(b)) => b.clone(),
            _ => return Err(cold_type("int_from_bytes() first arg must be bytes")),
        };
        let order_s = self.str_of(order, "int_from_bytes() byteorder must be 'big' or 'little'")?;
        if buf.len() > 8 { return Err(cold_overflow()); }
        let big = match order_s.as_str() {
            "big" => true,
            "little" => false,
            _ => return Err(cold_value("byteorder must be 'big' or 'little'")),
        };
        let mut acc: u64 = 0;
        if big {
            for &b in &buf { acc = (acc << 8) | b as u64; }
        } else {
            for (i, &b) in buf.iter().enumerate() { acc |= (b as u64) << (i * 8); }
        }
        // u64 always fits in i128; int_to_val picks inline-Val or LongInt storage.
        let val = self.int_to_val(Some(acc as i128))?;
        self.push(val);
        Ok(())
    }

    // `int_to_bytes(n, length, byteorder)`, non-negative int -> bytes of `length`; errors if it overflows.
    pub fn call_int_to_bytes(&mut self) -> Result<(), VmErr> {
        let order = self.pop()?;
        let length = self.pop()?;
        let n = self.pop()?;
        let n_i128 = self.as_i128(n).ok_or(cold_type("int_to_bytes() value must be an int"))?;
        if !length.is_int() { return Err(cold_type("int_to_bytes() length must be an int")); }
        let length = length.as_int() as usize;
        if length > 8 { return Err(cold_value("int_to_bytes() length must be <= 8")); }
        if n_i128 < 0 { return Err(cold_value("int_to_bytes() requires a non-negative int")); }
        let order_s = self.str_of(order, "int_to_bytes() byteorder must be 'big' or 'little'")?;
        let big = match order_s.as_str() {
            "big" => true,
            "little" => false,
            _ => return Err(cold_value("byteorder must be 'big' or 'little'")),
        };
        // Value must fit in `length` bytes (u64::MAX for length=8). Length cap makes the comparison overflow-free in u128.
        if n_i128 as u128 >= (1u128 << (length * 8).min(127)) {
            return Err(cold_overflow());
        }
        let val = n_i128 as u64;
        let mut out = Vec::with_capacity(length);
        if big {
            for i in (0..length).rev() { out.push((val >> (i * 8) & 0xff) as u8); }
        } else {
            for i in 0..length { out.push((val >> (i * 8) & 0xff) as u8); }
        }
        let v = self.heap.alloc(HeapObj::Bytes(out))?;
        self.push(v); Ok(())
    }

    // `import_module(name)`, fetch an already-imported module by alias; returns its `HeapObj::Module` Val.
    pub fn call_import_module(&mut self) -> Result<(), VmErr> {
        let spec = self.pop()?;
        if !spec.is_heap() {
            return Err(cold_type("import_module() argument must be a string"));
        }
        let name = match self.heap.get(spec) {
            HeapObj::Str(s) => s.clone(),
            _ => return Err(cold_type("import_module() argument must be a string")),
        };
        // Parser stores top-level bindings as both `name` and `name_0`, try both so the user's alias matches.
        let val = self.globals.get(&name)
            .or_else(|| self.globals.get(&s!(str &name, "_0")))
            .copied()
            .ok_or_else(|| VmErr::Name(s!("module '", str &name, "' not imported in this scope")))?;
        if !val.is_heap() || !matches!(self.heap.get(val), HeapObj::Module(..)) {
            return Err(VmErr::TypeMsg(s!("'", str &name, "' is not a module")));
        }
        self.push(val); Ok(())
    }
}
