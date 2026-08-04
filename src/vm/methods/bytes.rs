/*
Built-in methods for `bytes` receivers. Arity is checked by the dispatcher.
*/

use super::prelude::*;

// `bytes.decode([encoding[, errors]])`. 'strict' raises on invalid UTF-8; 'ignore'/'replace' recover.
pub fn decode(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    if let Some(arg) = pos.first() {
        let enc = val_to_str(vm, *arg)?;
        if !matches!(enc.as_str(), "utf-8" | "utf8" | "ascii") {
            return Err(cold_value("unsupported encoding (expected 'utf-8' or 'ascii')"));
        }
    }
    let errors = match pos.get(1) {
        Some(a) => val_to_str(vm, *a)?,
        None => alloc::string::String::from("strict"),
    };
    let text = match errors.as_str() {
        "strict" => alloc::string::String::from_utf8(buf)
            .map_err(|_| cold_value("invalid UTF-8 in bytes.decode()"))?,
        "ignore" => decode_recover(&buf, false),
        "replace" => decode_recover(&buf, true),
        _ => return Err(cold_value("unknown error handler (expected 'strict', 'ignore', or 'replace')")),
    };
    let v = vm.heap.alloc(HeapObj::Str(text))?;
    vm.push(v); Ok(())
}

// Decode dropping invalid bytes (replace=false) or substituting U+FFFD (replace=true).
fn decode_recover(buf: &[u8], replace: bool) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(buf.len());
    let mut i = 0;
    while i < buf.len() {
        match core::str::from_utf8(&buf[i..]) {
            Ok(s) => { out.push_str(s); break; }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(core::str::from_utf8(&buf[i..i + valid]).unwrap());
                if replace { out.push('\u{FFFD}'); }
                match e.error_len() {
                    Some(n) => i += valid + n,
                    None => break, // incomplete sequence at the end
                }
            }
        }
    }
    out
}

// `bytes.hex()`, lowercase hex of every byte. No separator.
pub fn hex(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let mut out = alloc::string::String::with_capacity(buf.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in &buf {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    let v = vm.heap.alloc(HeapObj::Str(out))?;
    vm.push(v); Ok(())
}

// bytes-only; strings go through `string::startswith`.
pub fn startswith(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let prefix = recv_bytes(vm, pos[0])?;
    vm.push(Val::bool(buf.starts_with(&prefix)));
    Ok(())
}

pub fn endswith(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let suffix = recv_bytes(vm, pos[0])?;
    vm.push(Val::bool(buf.ends_with(&suffix)));
    Ok(())
}

/* Shared search for find/index; `raise` turns a miss into ValueError. Empty needle matches at 0 (CPython); windows(0) would panic. */
fn find_impl(vm: &mut VM, recv: Val, pos: &[Val], raise: bool) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let sub = recv_bytes(vm, pos[0])?;
    let idx = if sub.is_empty() { Some(0) } else {
        buf.windows(sub.len()).position(|w| w == sub.as_slice())
    };
    let idx = match idx {
        Some(i) => i as i64,
        None if raise => return Err(cold_value("subsection not found")),
        None => -1,
    };
    vm.push(Val::int(idx));
    Ok(())
}
pub fn find(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, false) }
pub fn index(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, true) }

pub fn count(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let sub = recv_bytes(vm, pos[0])?;
    if sub.is_empty() {
        vm.push(Val::int(buf.len() as i64 + 1));
        return Ok(());
    }
    let mut n = 0i64;
    let mut i = 0usize;
    while i + sub.len() <= buf.len() {
        if buf[i..i + sub.len()] == sub[..] { n += 1; i += sub.len(); }
        else { i += 1; }
    }
    vm.push(Val::int(n));
    Ok(())
}

pub fn replace(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let old = recv_bytes(vm, pos[0])?;
    let new = recv_bytes(vm, pos[1])?;
    if old.is_empty() {
        // CPython inserts `new` before each byte and once at the end.
        let mut out: Vec<u8> = Vec::with_capacity(new.len() * (buf.len() + 1) + buf.len());
        out.extend_from_slice(&new);
        for &b in &buf { out.push(b); out.extend_from_slice(&new); }
        let v = vm.heap.alloc(HeapObj::Bytes(out))?;
        vm.push(v); return Ok(());
    }
    let mut out: Vec<u8> = Vec::with_capacity(buf.len());
    let mut i = 0usize;
    while i < buf.len() {
        if i + old.len() <= buf.len() && buf[i..i + old.len()] == old[..] {
            out.extend_from_slice(&new); i += old.len();
        } else {
            out.push(buf[i]); i += 1;
        }
    }
    let v = vm.heap.alloc(HeapObj::Bytes(out))?;
    vm.push(v); Ok(())
}

pub fn split(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let sep = recv_bytes(vm, pos[0])?;
    if sep.is_empty() { return Err(cold_value("empty separator")); }
    let mut parts: Vec<Val> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + sep.len() <= buf.len() {
        if buf[i..i + sep.len()] == sep[..] {
            parts.push(vm.heap.alloc(HeapObj::Bytes(buf[start..i].to_vec()))?);
            i += sep.len(); start = i;
        } else { i += 1; }
    }
    parts.push(vm.heap.alloc(HeapObj::Bytes(buf[start..].to_vec()))?);
    vm.alloc_and_push_list(parts)
}

// `bytes.fromhex(s)` classmethod: parse pairs of hex digits, skipping ASCII whitespace.
pub fn fromhex(vm: &mut VM, _recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = val_to_str(vm, pos[0])?;
    let mut out: Vec<u8> = Vec::new();
    let mut hi: Option<u8> = None;
    for c in s.chars() {
        if c.is_whitespace() { continue; }
        let d = c.to_digit(16).ok_or(cold_value("non-hexadecimal number found in fromhex() arg"))? as u8;
        match hi { None => hi = Some(d), Some(h) => { out.push((h << 4) | d); hi = None; } }
    }
    if hi.is_some() { return Err(cold_value("non-hexadecimal number found in fromhex() arg")); }
    let v = vm.heap.alloc(HeapObj::Bytes(out))?;
    vm.push(v); Ok(())
}

pub fn lower(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let mut buf = recv_bytes(vm, recv)?;
    buf.make_ascii_lowercase();
    let v = vm.heap.alloc(HeapObj::Bytes(buf))?; vm.push(v); Ok(())
}

pub fn upper(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let mut buf = recv_bytes(vm, recv)?;
    buf.make_ascii_uppercase();
    let v = vm.heap.alloc(HeapObj::Bytes(buf))?; vm.push(v); Ok(())
}

// bytes strip: ASCII whitespace, or any byte in the optional argument.
fn bstrip(vm: &mut VM, recv: Val, pos: &[Val], left: bool, right: bool) -> Result<(), VmErr> {
    let buf = recv_bytes(vm, recv)?;
    let chars = match pos.first() { Some(&a) => Some(recv_bytes(vm, a)?), None => None };
    // Python's bytes whitespace set includes the vertical tab (0x0b), which Rust omits.
    let strip = |b: u8| -> bool { match &chars { Some(set) => set.contains(&b), None => b.is_ascii_whitespace() || b == 0x0b } };
    let mut s = 0usize;
    let mut e = buf.len();
    if left { while s < e && strip(buf[s]) { s += 1; } }
    if right { while e > s && strip(buf[e - 1]) { e -= 1; } }
    let v = vm.heap.alloc(HeapObj::Bytes(buf[s..e].to_vec()))?; vm.push(v); Ok(())
}
pub fn strip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { bstrip(vm, recv, pos, true, true) }
pub fn lstrip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { bstrip(vm, recv, pos, true, false) }
pub fn rstrip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { bstrip(vm, recv, pos, false, true) }

pub fn join(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let sep = recv_bytes(vm, recv)?;
    let items = extract_sequence(vm, pos[0], "can only join an iterable of bytes")?;
    let mut out: Vec<u8> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if i > 0 { out.extend_from_slice(&sep); }
        out.extend_from_slice(&recv_bytes(vm, *it)?);
    }
    let v = vm.heap.alloc(HeapObj::Bytes(out))?; vm.push(v); Ok(())
}
