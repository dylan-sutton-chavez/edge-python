use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use wasm_pdk::*;

struct Item {
    code: u8,
    count: usize,
}

// Byte width of a format code, None for an unknown code (the single source of the legal-code set).
fn width(code: u8) -> Option<usize> {
    Some(match code {
        b'x' | b'b' | b'B' | b'?' => 1,
        b'h' | b'H' => 2,
        b'i' | b'I' | b'f' => 4,
        b'q' | b'Q' | b'd' => 8,
        _ => return None,
    })
}

// (big_endian, items), rejects unknown codes and dangling counts.
fn parse_fmt(fmt: &str) -> Result<(bool, Vec<Item>)> {
    let bytes = fmt.as_bytes();
    let mut i = 0;
    let big = match bytes.first() {
        Some(b'<') | Some(b'=') => { i = 1; false }
        Some(b'>') | Some(b'!') => { i = 1; true }
        _ => false,
    };
    let mut items = Vec::new();
    while i < bytes.len() {
        let mut count: usize = 0;
        let mut has_count = false;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            count = count * 10 + (bytes[i] - b'0') as usize;
            has_count = true;
            i += 1;
            if count > 0xFFFF { return Err(Error::Value(String::from("repeat count too large"))); }
        }
        let code = *bytes.get(i).ok_or_else(|| Error::Value(String::from("format ends with a repeat count")))?;
        i += 1;
        if width(code).is_none() {
            return Err(Error::Value(format!("bad char in struct format: '{}'", code as char)));
        }
        items.push(Item { code, count: if has_count { count } else { 1 } });
    }
    Ok((big, items))
}

fn total_size(items: &[Item]) -> usize {
    // Codes are validated by parse_fmt, so width is always Some here.
    items.iter().map(|it| it.count * width(it.code).unwrap()).sum()
}

fn value_count(items: &[Item]) -> usize {
    items.iter().filter(|it| it.code != b'x').map(|it| it.count).sum()
}

// Range-checked int for one code, then endian-ordered bytes.
fn put_int(out: &mut Vec<u8>, big: bool, code: u8, i: i128) -> Result<()> {
    let (min, max): (i128, i128) = match code {
        b'b' => (i8::MIN as i128, i8::MAX as i128),
        b'B' => (0, u8::MAX as i128),
        b'h' => (i16::MIN as i128, i16::MAX as i128),
        b'H' => (0, u16::MAX as i128),
        b'i' => (i32::MIN as i128, i32::MAX as i128),
        b'I' => (0, u32::MAX as i128),
        b'q' => (i64::MIN as i128, i64::MAX as i128),
        _ => (0, u64::MAX as i128),
    };
    if !(min..=max).contains(&i) {
        return Err(Error::Value(format!("argument out of range for '{}'", code as char)));
    }
    let w = width(code).unwrap();
    let full = if big { i.to_be_bytes() } else { i.to_le_bytes() };
    // i128 buffer, value bytes sit at the tail for BE, at the head for LE.
    let slice = if big { &full[16 - w..] } else { &full[..w] };
    out.extend_from_slice(slice);
    Ok(())
}

fn put(out: &mut Vec<u8>, big: bool, code: u8, v: &Value) -> Result<()> {
    match (code, v) {
        (b'?', Value::Bool(b)) => out.push(*b as u8),
        (b'?', Value::Int(i)) => out.push((*i != 0) as u8),
        (b'f', Value::Float(f)) => out.extend_from_slice(&if big { (*f as f32).to_be_bytes() } else { (*f as f32).to_le_bytes() }),
        (b'f', Value::Int(i)) => out.extend_from_slice(&if big { (*i as f32).to_be_bytes() } else { (*i as f32).to_le_bytes() }),
        (b'd', Value::Float(f)) => out.extend_from_slice(&if big { f.to_be_bytes() } else { f.to_le_bytes() }),
        (b'd', Value::Int(i)) => out.extend_from_slice(&if big { (*i as f64).to_be_bytes() } else { (*i as f64).to_le_bytes() }),
        (b'f' | b'd' | b'?', other) => return Err(Error::Type(format!("expected a number for '{}', got {:?}", code as char, other))),
        (_, Value::Int(i)) => put_int(out, big, code, *i)?,
        (_, other) => return Err(Error::Type(format!("required argument is not an integer for '{}', got {:?}", code as char, other))),
    }
    Ok(())
}

fn take(big: bool, code: u8, chunk: &[u8]) -> Value {
    // Widen through a zero/sign-filled i128 buffer, mirroring `put_int`.
    let int_of = |signed: bool| -> i128 {
        let mut full = if signed && (if big { chunk[0] & 0x80 != 0 } else { chunk[chunk.len() - 1] & 0x80 != 0 }) {
            [0xFFu8; 16]
        } else {
            [0u8; 16]
        };
        if big { full[16 - chunk.len()..].copy_from_slice(chunk); } else { full[..chunk.len()].copy_from_slice(chunk); }
        if big { i128::from_be_bytes(full) } else { i128::from_le_bytes(full) }
    };
    match code {
        b'?' => Value::Bool(chunk[0] != 0),
        b'f' => {
            let arr: [u8; 4] = chunk.try_into().unwrap();
            Value::Float((if big { f32::from_be_bytes(arr) } else { f32::from_le_bytes(arr) }) as f64)
        }
        b'd' => {
            let arr: [u8; 8] = chunk.try_into().unwrap();
            Value::Float(if big { f64::from_be_bytes(arr) } else { f64::from_le_bytes(arr) })
        }
        b'b' | b'h' | b'i' | b'q' => Value::Int(int_of(true)),
        _ => Value::Int(int_of(false)),
    }
}

/* Format-string packing into fixed-width `bytes`, byte-order prefix, repeat counts, one width code per value. No alignment padding, `unpack` returns a list. */
#[plugin_fn]
fn pack(fmt: String, values: Args) -> Result<Bytes> {
    let (big, items) = parse_fmt(&fmt)?;
    let needed = value_count(&items);
    if values.len() != needed {
        return Err(Error::Value(format!("pack expected {} items for format '{}', got {}", needed, fmt, values.len())));
    }
    let mut out = Vec::with_capacity(total_size(&items));
    let mut vi = 0;
    for it in &items {
        for _ in 0..it.count {
            if it.code == b'x' { out.push(0); continue; }
            let v: Value = values.get(vi).unwrap()?;
            vi += 1;
            put(&mut out, big, it.code, &v)?;
        }
    }
    Ok(Bytes(out))
}

#[plugin_fn]
fn unpack(fmt: String, data: Bytes) -> Result<Vec<Value>> {
    let (big, items) = parse_fmt(&fmt)?;
    let total = total_size(&items);
    if data.len() != total {
        return Err(Error::Value(format!("unpack requires a buffer of {} bytes, got {}", total, data.len())));
    }
    let mut out = Vec::with_capacity(value_count(&items));
    let mut pos = 0;
    for it in &items {
        for _ in 0..it.count {
            let w = width(it.code).unwrap();
            let chunk = &data[pos..pos + w];
            pos += w;
            if it.code == b'x' { continue; }
            out.push(take(big, it.code, chunk));
        }
    }
    Ok(out)
}

#[plugin_fn]
fn calcsize(fmt: String) -> Result<i64> {
    let (_, items) = parse_fmt(&fmt)?;
    Ok(total_size(&items) as i64)
}
