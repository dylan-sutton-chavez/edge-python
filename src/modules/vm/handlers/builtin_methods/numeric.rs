/*
Built-in methods for `int` / `float` receivers (and the `int.from_bytes` classmethod).
Arity is checked by the dispatcher.
*/

use super::prelude::*;
use crate::modules::vm::types::as_i128;

pub fn bit_length(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let n = as_i128(recv, &vm.heap).ok_or(cold_type("bit_length() requires an int"))?;
    let bits = 128 - n.unsigned_abs().leading_zeros();
    vm.push(Val::int(bits as i64)); Ok(())
}

pub fn bit_count(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let n = as_i128(recv, &vm.heap).ok_or(cold_type("bit_count() requires an int"))?;
    vm.push(Val::int(n.unsigned_abs().count_ones() as i64)); Ok(())
}

// `int.to_bytes(length=1, byteorder='big')`; unsigned (signed=False), errors if it doesn't fit.
pub fn to_bytes(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let n = as_i128(recv, &vm.heap).ok_or(cold_type("to_bytes() requires an int"))?;
    if n < 0 { return Err(cold_overflow_msg("can't convert negative int to unsigned")); }
    let length = match pos.first() { Some(v) if v.is_int() => v.as_int().max(0) as usize, None => 1, _ => return Err(cold_type("length must be an integer")) };
    // Length is user-controlled; cap it against the heap budget so a huge count errors instead of aborting in the allocator.
    if length > vm.heap.limit() { return Err(cold_heap()); }
    let big = byteorder_is_big(vm, pos.get(1))?;
    let mut v = n.unsigned_abs();
    let mut out = alloc::vec![0u8; length];
    for k in 0..length {
        let byte = (v & 0xff) as u8;
        let idx = if big { length - 1 - k } else { k };
        out[idx] = byte;
        v >>= 8;
    }
    if v != 0 { return Err(cold_overflow_msg("int too big to convert")); }
    let val = vm.heap.alloc(HeapObj::Bytes(out))?;
    vm.push(val); Ok(())
}

// `int.from_bytes(bytes, byteorder='big')` classmethod; unsigned.
pub fn from_bytes(vm: &mut VM, _recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, pos[0])?;
    let big = byteorder_is_big(vm, pos.get(1))?;
    let mut acc: i128 = 0;
    if big {
        for &b in &buf { acc = (acc << 8) | b as i128; }
    } else {
        for &b in buf.iter().rev() { acc = (acc << 8) | b as i128; }
    }
    let v = vm.int_to_val(Some(acc))?;
    vm.push(v); Ok(())
}

pub fn is_integer(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let f = if recv.is_float() { recv.as_float() } else { return Err(cold_type("is_integer() requires a float")); };
    vm.push(Val::bool(f.is_finite() && f == libm::trunc(f))); Ok(())
}

fn byteorder_is_big(vm: &VM, arg: Option<&Val>) -> Result<bool, VmErr> {
    match arg {
        None => Ok(true), // default is 'big'
        Some(&v) => match vm.heap.try_get(v) {
            Some(HeapObj::Str(s)) if s == "big" => Ok(true),
            Some(HeapObj::Str(s)) if s == "little" => Ok(false),
            _ => Err(cold_value("byteorder must be 'little' or 'big'")),
        },
    }
}

fn cold_overflow_msg(m: &'static str) -> VmErr {
    VmErr::Raised(crate::s!("OverflowError: ", str m))
}
