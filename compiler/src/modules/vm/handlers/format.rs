/*
f-string format spec, PEP 3101 subset: `[[fill]align][sign][#][0][width][,][.precision][type]`.
Types: d/b/o/x/X (ints), f/F/e/E/g/G (floats), % (percent), s (str), c (codepoint).
Returns Err(msg); caller raises ValueError.
*/

use alloc::string::{String, ToString};
use crate::modules::vm::types::{Val, HeapObj, HeapPool, VmErr, cold_value, fabs, ffloor, flog10, fsignum, ftrunc, num_as_f64};

// `%c`/`{:c}` out-of-range raises OverflowError; other format errors are ValueError.
pub const C_RANGE_ERR: &str = "%c arg not in range(0x110000)";

pub fn fmt_err(m: &'static str) -> VmErr {
    if m == C_RANGE_ERR { VmErr::Raised(crate::s!("OverflowError: ", str m)) } else { cold_value(m) }
}

pub fn format_value(v: Val, spec: &str, heap: &HeapPool) -> Result<String, &'static str> {
    if spec.is_empty() {
        return Ok(display_inline(v, heap));
    }
    let parsed = parse_spec(spec)?;
    // Cap against the heap budget; precision also below core::fmt's u16 abort threshold (panics at >= 65535).
    const PRECISION_MAX: usize = 65_000;
    if parsed.width > heap.limit() { return Err("format width exceeds limit"); }
    if parsed.precision.is_some_and(|p| p > heap.limit().min(PRECISION_MAX)) { return Err("format precision exceeds limit"); }
    apply(v, &parsed, heap)
}

#[derive(Default, Clone)]
struct Spec {
    fill: char,
    align: Option<u8>, // b'<' b'>' b'^' b'='
    sign: u8, // 0, b'+', b'-', b' '
    alt: bool, // '#' alternate form, emits base prefix for b/o/x/X
    zero_pad: bool,
    width: usize,
    sep: u8, // 0, b',' or b'_' digit-group separator
    precision: Option<usize>,
    ty: u8, // 0 means default
}

fn parse_spec(spec: &str) -> Result<Spec, &'static str> {
    let bytes = spec.as_bytes();
    let mut s = Spec { fill: ' ', ..Spec::default() };
    let mut i = 0;

    /* fill+align is "<char><align>" only when char #2 is one of `<>^=`. */
    if bytes.len() >= 2 && matches!(bytes[1], b'<' | b'>' | b'^' | b'=') {
        s.fill = bytes[0] as char;
        s.align = Some(bytes[1]);
        i = 2;
    } else if !bytes.is_empty() && matches!(bytes[0], b'<' | b'>' | b'^' | b'=') {
        s.align = Some(bytes[0]);
        i = 1;
    }

    if i < bytes.len() && matches!(bytes[i], b'+' | b'-' | b' ') {
        s.sign = bytes[i];
        i += 1;
    }

    // '#' alternate form, opt in to base prefix on b/o/x/X.
    if i < bytes.len() && bytes[i] == b'#' { s.alt = true; i += 1; }

    if i < bytes.len() && bytes[i] == b'0' {
        s.zero_pad = true;
        if s.align.is_none() {
            s.align = Some(b'=');
            s.fill = '0';
        }
        i += 1;
    }

    let w_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
    if i > w_start {
        s.width = spec[w_start..i].parse().map_err(|_| "invalid width in format spec")?;
    }

    // ',' or '_' digit-group separator; '_' groups hex/oct/bin by four.
    if i < bytes.len() && matches!(bytes[i], b',' | b'_') {
        s.sep = bytes[i];
        i += 1;
    }

    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let p_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
        if i == p_start { return Err("missing precision in format spec"); }
        s.precision = Some(spec[p_start..i].parse().map_err(|_| "invalid precision in format spec")?);
    }

    if i < bytes.len() {
        if !bytes[i].is_ascii() { return Err("invalid type in format spec"); }
        s.ty = bytes[i];
        i += 1;
    }

    if i != bytes.len() { return Err("trailing characters in format spec"); }
    Ok(s)
}

fn apply(v: Val, s: &Spec, heap: &HeapPool) -> Result<String, &'static str> {
    let is_long = v.is_heap() && matches!(heap.get(v), HeapObj::LongInt(_));
    let is_int_like = v.is_int() || v.is_bool() || is_long;
    // Dispatch by type char: int types -> int formatter, float types coerce ints up, `s` only strings.
    match s.ty {
        0 | b's' => {
            if is_int_like || v.is_float() || v.is_none() {
                if s.ty == b's' { return Err("'s' format spec requires a string"); }
                // Precision on int -> float-fixed; thousands stays in int path to keep LongInt precision.
                if s.precision.is_some() && !is_int_like {
                    return format_float(v, s, b'f', heap);
                }
                if s.sep != 0 && is_int_like {
                    let mut s2 = s.clone();
                    s2.ty = b'd';
                    return format_int(v, &s2, heap);
                }
                if s.precision.is_some() {
                    return format_float(v, s, b'f', heap);
                }
                if v.is_int() {
                    return Ok(pad_numeric(s, &itoa_str(v.as_int())));
                }
                if is_long
                    && let HeapObj::LongInt(i) = heap.get(v) {
                    let mut buf = itoa::Buffer::new();
                    return Ok(pad_numeric(s, buf.format(*i)));
                }
                if v.is_float() {
                    return format_float(v, s, b'f', heap);
                }
            }
            let raw = display_inline(v, heap);
            let truncated = match s.precision {
                Some(p) => raw.chars().take(p).collect::<String>(),
                None => raw,
            };
            Ok(pad_string(s, &truncated))
        }
        // `n` is locale-aware decimal; paradigm has no locale, so alias to `d`.
        b'd' | b'b' | b'o' | b'x' | b'X' | b'n' => {
            if s.precision.is_some() { return Err("precision not allowed in integer format spec"); }
            let mut s2 = s.clone();
            if s2.ty == b'n' { s2.ty = b'd'; }
            format_int(v, &s2, heap)
        }
        b'f' | b'F' => format_float(v, s, s.ty, heap),
        b'e' | b'E' | b'g' | b'G' => format_float(v, s, s.ty, heap),
        b'%' => format_percent(v, s, heap),
        b'c' => format_char(v, s),
        _ => Err("unknown format type"),
    }
}

fn format_percent(v: Val, s: &Spec, heap: &HeapPool) -> Result<String, &'static str> {
    let f = require_float(v, heap)? * 100.0;
    let prec = s.precision.unwrap_or(6);
    let body = if f.is_nan() { "nan".to_string() }
    else if f.is_infinite() { if f.is_sign_negative() { "-inf".into() } else { "inf".into() } }
    else { fixed(f.abs(), prec) };
    Ok(signed_pad(s, f.is_sign_negative() && !f.is_nan(), "", &alloc::format!("{body}%")))
}

fn format_char(v: Val, s: &Spec) -> Result<String, &'static str> {
    if !v.is_int() { return Err("'c' format spec requires an integer"); }
    let i = v.as_int();
    if !(0..=0x10FFFF).contains(&i) { return Err(C_RANGE_ERR); }
    let ch = char::from_u32(i as u32).ok_or("'c' format spec arg not a valid char")?;
    // 'c' is numeric for alignment: it defaults to right-align like ints.
    Ok(pad_aligned(s, &ch.to_string(), 0))
}

fn format_int(v: Val, s: &Spec, heap: &HeapPool) -> Result<String, &'static str> {
    let (neg, mag) = int_to_decimal_parts(v, heap)?;
    let (digits, prefix): (String, &'static str) = match s.ty {
        b'd' => (mag, ""),
        b'b' => (decimal_to_radix(&mag, 2), if s.alt { "0b" } else { "" }),
        b'o' => (decimal_to_radix(&mag, 8), if s.alt { "0o" } else { "" }),
        b'x' => (decimal_to_radix(&mag, 16), if s.alt { "0x" } else { "" }),
        b'X' => (decimal_to_radix(&mag, 16).to_uppercase(), if s.alt { "0X" } else { "" }),
        _ => unreachable!(),
    };
    let body = if s.sep != 0 { add_grouped(&digits, group_size(s.ty, s.sep), s.sep) } else { digits };
    Ok(signed_pad(s, neg, prefix, &body))
}

fn format_float(v: Val, s: &Spec, ty: u8, heap: &HeapPool) -> Result<String, &'static str> {
    let f = require_float(v, heap)?;
    let prec = s.precision.unwrap_or(6);

    /* NaN/inf go through unchanged (emits "nan"/"inf" before padding). */
    if f.is_nan() {
        let body = if ty == b'F' { "NAN" } else { "nan" };
        return Ok(pad_string(s, body));
    }
    if f.is_infinite() {
        return Ok(signed_pad(s, f.is_sign_negative(), "", if ty == b'F' { "INF" } else { "inf" }));
    }

    let mag = f.abs();
    let body = match ty {
        b'f' | b'F' => fixed(mag, prec),
        // e/g delegate to Rust's f64 formatter; round-half-to-even applies only to `f`.
        b'e' => format_with_e(mag, prec, false),
        b'E' => format_with_e(mag, prec, true),
        // `g/G`: pick `e` for very small/large, `f` otherwise.
        b'g' | b'G' => {
            let upper = ty == b'G';
            let exp = if mag == 0.0 { 0 } else { ffloor(flog10(mag)) as i32 };
            // rule: -4 <= exp < precision uses fixed; else scientific.
            let p = prec.max(1);
            if exp < -4 || exp >= p as i32 {
                format_with_e(mag, p.saturating_sub(1), upper)
            } else {
                let dec = (p as i32 - 1 - exp).max(0) as usize;
                let out = fixed(mag, dec);
                if upper { out.to_uppercase() } else { out }
            }
        }
        _ => unreachable!(),
    };
    let body = if s.sep != 0 { add_thousands_float(&body, s.sep) } else { body };
    Ok(signed_pad(s, f.is_sign_negative(), "", &body))
}

fn format_with_e(mag: f64, prec: usize, upper: bool) -> String {
    // Rust emits "3.14e0"; expects "e+00", inject the sign and pad exponent to >=2 digits.
    let raw = alloc::format!("{:.*e}", prec, mag);
    let (mant, exp_str) = raw.split_once('e').unwrap_or((raw.as_str(), "0"));
    let (esign, edigs) = if let Some(rest) = exp_str.strip_prefix('-') { ('-', rest) }
    else { ('+', exp_str) };
    let mut out = String::new();
    out.push_str(mant);
    out.push(if upper { 'E' } else { 'e' });
    out.push(esign);
    if edigs.len() < 2 { out.push('0'); }
    out.push_str(edigs);
    out
}

fn require_float(v: Val, heap: &HeapPool) -> Result<f64, &'static str> {
    num_as_f64(v, heap).ok_or("format spec requires a number")
}

fn int_to_decimal_parts(v: Val, heap: &HeapPool) -> Result<(bool, String), &'static str> {
    if v.is_int() {
        let i = v.as_int();
        let neg = i < 0;
        let mut b = itoa::Buffer::new();
        let mag = b.format(i.unsigned_abs()).to_string();
        return Ok((neg, mag));
    }
    if v.is_bool() { return Ok((false, itoa_str(v.as_bool() as i64))); }
    if v.is_heap() && let HeapObj::LongInt(i) = heap.get(v) {
        let neg = *i < 0;
        let mut b = itoa::Buffer::new();
        // unsigned_abs handles i128::MIN: returns 2^127 in u128.
        let mag = b.format(i.unsigned_abs()).to_string();
        return Ok((neg, mag));
    }
    Err("format spec requires an integer")
}

fn decimal_to_radix(mag: &str, radix: u32) -> String {
    // Non-neg decimal string -> `radix`. Parses into u128 to cover LongInt
    // magnitudes (up to 2^127). Bool/inline-int values still fit trivially.
    if mag == "0" { return String::from("0"); }
    let mut n: u128 = mag.parse().unwrap_or(0);
    let mut out = String::new();
    while n > 0 {
        let d = (n % radix as u128) as u32;
        out.push(core::char::from_digit(d, radix).unwrap());
        n /= radix as u128;
    }
    out.chars().rev().collect()
}

// '_' groups binary/octal/hex every four digits; decimal and ',' every three.
fn group_size(ty: u8, sep: u8) -> usize {
    if sep == b'_' && matches!(ty, b'b' | b'o' | b'x' | b'X') { 4 } else { 3 }
}

fn add_grouped(digits: &str, group: usize, sep: u8) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / group);
    let chars: alloc::vec::Vec<char> = digits.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(group) { out.push(sep as char); }
        out.push(c);
    }
    out
}

fn add_thousands_float(s: &str, sep: u8) -> String {
    /* Insert separators only in the integer portion (before `.` / `e`). */
    let split = s.find(['.', 'e', 'E']);
    let (int_part, rest) = match split {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let mut out = add_grouped(int_part, 3, sep);
    out.push_str(rest);
    out
}

fn sign_char(neg: bool, sign_flag: u8) -> Option<char> {
    if neg { Some('-') }
    else if sign_flag == b'+' { Some('+') }
    else if sign_flag == b' ' { Some(' ') }
    else { None }
}

/* Sign char + optional radix prefix + body, then numeric-aligned padding. */
fn signed_pad(s: &Spec, neg: bool, prefix: &str, body: &str) -> String {
    let sign_ch = sign_char(neg, s.sign);
    let mut left = String::new();
    if let Some(c) = sign_ch { left.push(c); }
    left.push_str(prefix);
    left.push_str(body);
    pad_aligned(s, &left, sign_ch.map(|_| 1).unwrap_or(0) + prefix.len())
}

fn pad_numeric(s: &Spec, body: &str) -> String {
    let (neg, mag) = if let Some(rest) = body.strip_prefix('-') { (true, rest) } else { (false, body) };
    signed_pad(s, neg, "", mag)
}

fn pad_string(s: &Spec, body: &str) -> String {
    let len = body.chars().count();
    if len >= s.width { return body.to_string(); }
    let pad = s.width - len;
    // Strings can't use '=' alignment; a zero-fill default becomes left-align.
    let align = match s.align.unwrap_or(b'<') { b'=' => b'<', a => a };
    pad_with(body, pad, align, s.fill, 0)
}

fn pad_aligned(s: &Spec, body: &str, sign_prefix_len: usize) -> String {
    let len = body.chars().count();
    if len >= s.width { return body.to_string(); }
    let pad = s.width - len;
    let align = s.align.unwrap_or(b'>');
    pad_with(body, pad, align, s.fill, sign_prefix_len)
}

fn pad_with(body: &str, pad: usize, align: u8, fill: char, sign_prefix_len: usize) -> String {
    let mut out = String::new();
    match align {
        b'<' => {
            out.push_str(body);
            for _ in 0..pad { out.push(fill); }
        }
        b'>' => {
            for _ in 0..pad { out.push(fill); }
            out.push_str(body);
        }
        b'^' => {
            let l = pad / 2;
            let r = pad - l;
            for _ in 0..l { out.push(fill); }
            out.push_str(body);
            for _ in 0..r { out.push(fill); }
        }
        b'=' => {
            /* Sign-aware: insert padding between the sign/prefix and the digits. */
            let mut chars = body.chars();
            for _ in 0..sign_prefix_len {
                if let Some(c) = chars.next() { out.push(c); }
            }
            for _ in 0..pad { out.push(fill); }
            out.push_str(chars.as_str());
        }
        _ => unreachable!(),
    }
    out
}

fn fixed(mag: f64, prec: usize) -> String {
    // Round-half-to-even to `prec` decimals; renders manually to avoid `alloc::format!`'s %f.
    let scale = pow10(prec);
    let scaled = mag * scale;
    let rounded = if fabs(scaled - ftrunc(scaled)) == 0.5 {
        let f = ftrunc(scaled);
        if (f as i64) % 2 == 0 { f } else if scaled > 0.0 { f + 1.0 } else { f - 1.0 }
    } else {
        ftrunc(scaled + 0.5 * fsignum(scaled))
    };
    let dec = u128_to_dec(fabs(rounded) as u128);
    if prec == 0 { return dec; }
    /* Pad on the left so we can insert a `.` exactly `prec` chars from the end. */
    let needed = prec + 1;
    let padded = if dec.len() < needed {
        let mut s = String::with_capacity(needed);
        for _ in 0..(needed - dec.len()) { s.push('0'); }
        s.push_str(&dec); s
    } else { dec };
    let dot = padded.len() - prec;
    let mut out = String::with_capacity(padded.len() + 1);
    out.push_str(&padded[..dot]);
    out.push('.');
    out.push_str(&padded[dot..]);
    out
}

fn pow10(n: usize) -> f64 {
    let mut r = 1.0f64;
    for _ in 0..n { r *= 10.0; }
    r
}
fn itoa_str(i: i64) -> String {
    let mut b = itoa::Buffer::new(); b.format(i).to_string()
}
fn u128_to_dec(n: u128) -> String {
    if n == 0 { return String::from("0"); }
    let mut out = String::new();
    let mut x = n;
    while x > 0 { out.push((b'0' + (x % 10) as u8) as char); x /= 10; }
    out.chars().rev().collect()
}

/* Plain `str()` rendering inlined here so the hot path doesn't borrow from VM. */
pub fn display_inline(v: Val, heap: &HeapPool) -> String {
    if v.is_int() {
        let mut b = itoa::Buffer::new();
        return b.format(v.as_int()).to_string();
    }
    if v.is_bool() { return (if v.as_bool() { "True" } else { "False" }).to_string(); }
    if v.is_none() { return String::from("None"); }
    if v.is_float() { return crate::util::fstr::format_f64(v.as_float()); }
    if v.is_heap() {
        match heap.get(v) {
            HeapObj::Str(s) => return s.clone(),
            HeapObj::LongInt(i) => {
                let mut b = itoa::Buffer::new();
                return b.format(*i).to_string();
            }
            _ => {}
        }
    }
    /* Fall back to nothing, caller should use VM::display for full coverage. */
    String::new()
}
