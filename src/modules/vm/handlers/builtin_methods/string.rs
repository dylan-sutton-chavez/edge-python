/*
Built-in methods for `str` receivers. Arity is checked by the dispatcher.
*/

use super::prelude::*;

use core::iter;

// `str.encode([encoding])`, UTF-8/ASCII only; other names error to block silent mismatches.
pub fn encode(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    if let Some(arg) = pos.first() {
        let enc = val_to_str(vm, *arg)?;
        match enc.as_str() {
            "utf-8" | "utf8" => {}
            "ascii" if !s.is_ascii() => {
                return Err(cold_value("'ascii' codec can't encode non-ASCII characters"));
            }
            "ascii" => {}
            _ => return Err(cold_value("unsupported encoding (expected 'utf-8' or 'ascii')")),
        }
    }
    let v = vm.heap.alloc(HeapObj::Bytes(s.into_bytes()))?;
    vm.push(v); Ok(())
}

// str: zero-arg transforms `recv_str -> f -> push`.
macro_rules! str_transform {
    ($name:ident, $f:expr) => {
        pub fn $name(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
            let s = recv_str(vm, recv)?;
            vm.alloc_and_push_str($f(s.as_str()))
        }
    };
}
str_transform!(upper, |s: &str| s.to_uppercase());
str_transform!(lower, |s: &str| s.to_lowercase());
str_transform!(capitalize, capitalize_first);
str_transform!(title, title_case);

// str.strip / lstrip / rstrip: trim whitespace, or any char in the optional arg.
enum Trim { Both, Start, End }
fn strip_impl(vm: &mut VM, recv: Val, pos: &[Val], mode: Trim) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let out = if pos.is_empty() {
        match mode { Trim::Both => s.trim(), Trim::Start => s.trim_start(), Trim::End => s.trim_end() }.to_string()
    } else {
        let p = val_to_str(vm, pos[0])?;
        let f = |c: char| p.contains(c);
        match mode { Trim::Both => s.trim_matches(f), Trim::Start => s.trim_start_matches(f), Trim::End => s.trim_end_matches(f) }.to_string()
    };
    vm.alloc_and_push_str(out)
}
pub fn strip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { strip_impl(vm, recv, pos, Trim::Both) }
pub fn lstrip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { strip_impl(vm, recv, pos, Trim::Start) }
pub fn rstrip(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { strip_impl(vm, recv, pos, Trim::End) }

pub fn isdigit(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    // Unicode-aware (e.g. superscripts), not ASCII-only.
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_numeric())));
    Ok(())
}

pub fn isalpha(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())));
    Ok(())
}

pub fn isalnum(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())));
    Ok(())
}

// Collect the affix argument as one or more strings (str, or a tuple of str).
fn affixes(vm: &VM, v: Val) -> Result<Vec<String>, VmErr> {
    if let Some(HeapObj::Tuple(items)) = vm.heap.try_get(v) {
        let items = items.clone();
        let mut out = Vec::with_capacity(items.len());
        for it in items { out.push(val_to_str(vm, it)?); }
        Ok(out)
    } else {
        Ok(alloc::vec![val_to_str(vm, v)?])
    }
}

pub fn startswith(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let prefixes = affixes(vm, pos[0])?;
    // Optional start/stop window restricts where the prefix must begin.
    let (chars, start, stop) = slice_window(&s, pos, 1);
    let hay: String = chars[start..stop].iter().collect();
    vm.push(Val::bool(prefixes.iter().any(|p| hay.starts_with(p.as_str()))));
    Ok(())
}

pub fn endswith(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let suffixes = affixes(vm, pos[0])?;
    let (chars, start, stop) = slice_window(&s, pos, 1);
    let hay: String = chars[start..stop].iter().collect();
    vm.push(Val::bool(suffixes.iter().any(|p| hay.ends_with(p.as_str()))));
    Ok(())
}

// Optional char-index `start`/`stop` window (pos[1], pos[2]); negatives count from the end.
fn slice_window(s: &str, pos: &[Val], from: usize) -> (Vec<char>, usize, usize) {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let norm = |i: i64| -> usize { (if i < 0 { (n + i).max(0) } else { i.min(n) }) as usize };
    let start = pos.get(from).filter(|v| v.is_int()).map_or(0, |v| norm(v.as_int()));
    let stop = pos.get(from + 1).filter(|v| v.is_int()).map_or(n as usize, |v| norm(v.as_int()));
    (chars, start, stop.max(start))
}

fn find_impl(vm: &mut VM, recv: Val, pos: &[Val], last: bool, raise: bool) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let sub = val_to_str(vm, pos[0])?;
    let (chars, start, stop) = slice_window(&s, pos, 1);
    let hay: String = chars[start..stop].iter().collect();
    let local = if last { hay.rfind(sub.as_str()) } else { hay.find(sub.as_str()) };
    let idx = local.map(|b| (start + hay[..b].chars().count()) as i64).unwrap_or(-1);
    if raise && idx < 0 { return Err(cold_value("substring not found")); }
    vm.push(Val::int(idx));
    Ok(())
}
pub fn find(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, false, false) }
pub fn rfind(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, true, false) }
pub fn index(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, false, true) }
pub fn rindex(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { find_impl(vm, recv, pos, true, true) }

pub fn count(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let sub = val_to_str(vm, pos[0])?;
    let (chars, start, stop) = slice_window(&s, pos, 1);
    let hay: String = chars[start..stop].iter().collect();
    let c = if sub.is_empty() { hay.chars().count() + 1 } else { hay.matches(sub.as_str()).count() };
    vm.push(Val::int(c as i64));
    Ok(())
}

// Whitespace split capped at m, collapsing runs.
fn split_ws_max(s: &str, m: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        while i < n && chars[i].is_whitespace() { i += 1; }
        if i >= n { break; }
        if out.len() == m { out.push(chars[i..].iter().collect()); break; }
        let start = i;
        while i < n && !chars[i].is_whitespace() { i += 1; }
        out.push(chars[start..i].iter().collect());
    }
    out
}

// Right-to-left counterpart for rsplit(None, m).
fn rsplit_ws_max(s: &str, m: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = chars.len();
    let mut out = Vec::new();
    loop {
        while i > 0 && chars[i - 1].is_whitespace() { i -= 1; }
        if i == 0 { break; }
        if out.len() == m { out.push(chars[..i].iter().collect()); break; }
        let end = i;
        while i > 0 && !chars[i - 1].is_whitespace() { i -= 1; }
        out.push(chars[i..end].iter().collect());
    }
    out.reverse();
    out
}

// `str.split`/`rsplit([sep[, maxsplit]])`; `from_right` counts from the right and reverses sep-mode results.
fn split_impl(vm: &mut VM, recv: Val, pos: &[Val], from_right: bool) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    // Optional second arg: maxsplit (<0 means unlimited).
    let maxsplit: Option<usize> = match pos.get(1) {
        Some(n) if n.is_int() => { let m = n.as_int(); if m < 0 { None } else { Some(m as usize) } }
        Some(_) => return Err(cold_type("maxsplit must be an integer")),
        None => None,
    };
    let strs: Vec<String> = if pos.is_empty() || pos[0].is_none() {
        // No separator: split on runs of whitespace, dropping empties.
        match maxsplit {
            Some(m) => if from_right { rsplit_ws_max(&s, m) } else { split_ws_max(&s, m) },
            None => s.split_whitespace().map(String::from).collect(),
        }
    } else {
        let sep = val_to_str(vm, pos[0])?;
        if sep.is_empty() { return Err(cold_value("empty separator")); }
        match maxsplit {
            Some(m) if from_right => { let mut v: Vec<String> = s.rsplitn(m + 1, sep.as_str()).map(String::from).collect(); v.reverse(); v }
            Some(m) => s.splitn(m + 1, sep.as_str()).map(String::from).collect(),
            None => s.split(sep.as_str()).map(String::from).collect(),
        }
    };
    let parts: Vec<Val> = strs.into_iter().map(|p| vm.heap.alloc(HeapObj::Str(p))).collect::<Result<_, _>>()?;
    vm.alloc_and_push_list(parts)
}

pub fn split(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { split_impl(vm, recv, pos, false) }

pub fn join(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let sep = recv_str(vm, recv)?;
    let items = extract_sequence(vm, pos[0], "join() argument must be iterable")?;
    let mut parts: Vec<String> = Vec::with_capacity(items.len());
    for v in items { parts.push(val_to_str(vm, v)?); }
    vm.alloc_and_push_str(parts.join(sep.as_str()))
}

pub fn replace(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let old = val_to_str(vm, pos[0])?;
    let new = val_to_str(vm, pos[1])?;
    // Optional third arg: max replacements (<0 means all).
    let out = match pos.get(2) {
        Some(n) if n.is_int() && n.as_int() >= 0 => s.replacen(old.as_str(), new.as_str(), n.as_int() as usize),
        Some(n) if !n.is_int() => return Err(cold_type("replace count must be an integer")),
        _ => s.replace(old.as_str(), new.as_str()),
    };
    vm.alloc_and_push_str(out)
}

// `str.rsplit([sep[, maxsplit]])`: like split but counts from the right.
pub fn rsplit(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { split_impl(vm, recv, pos, true) }

// `str.format(*args)`: positional/auto/explicit-index fields with `{[idx][!r|!s][:spec]}`. Keyword fields aren't supported (the method dispatcher forbids kwargs).
pub fn format(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let tmpl = recv_str(vm, recv)?;
    let chars: Vec<char> = tmpl.chars().collect();
    let mut out = String::with_capacity(tmpl.len());
    let mut auto = 0usize;
    // Numbering mode: Some(true)=manual, Some(false)=auto.
    let mut manual: Option<bool> = None;
    let mut ci = 0;
    while ci < chars.len() {
        let c = chars[ci];
        if c == '{' {
            if chars.get(ci + 1) == Some(&'{') { out.push('{'); ci += 2; continue; }
            let mut j = ci + 1;
            let mut field = String::new();
            while j < chars.len() && chars[j] != '}' { field.push(chars[j]); j += 1; }
            if j >= chars.len() { return Err(cold_value("Single '{' encountered in format string")); }
            ci = j + 1;
            let (name_conv, spec) = match field.split_once(':') { Some((a, b)) => (a, b.to_string()), None => (field.as_str(), String::new()) };
            let (name, conv) = match name_conv.split_once('!') { Some((a, b)) => (a, Some(b)), None => (name_conv, None) };
            let val = if name.is_empty() {
                if manual == Some(true) { return Err(cold_value("cannot switch from manual field specification to automatic field numbering")); }
                manual = Some(false);
                let v = *pos.get(auto).ok_or(cold_index("Replacement index out of range"))?;
                auto += 1; v
            } else if let Ok(idx) = name.parse::<usize>() {
                if manual == Some(false) { return Err(cold_value("cannot switch from automatic field numbering to manual field specification")); }
                manual = Some(true);
                *pos.get(idx).ok_or(cold_index("Replacement index out of range"))?
            } else {
                return Err(cold_type("str.format() does not support keyword fields"));
            };
            // Conversion (!r/!s) renders to a string first, then the spec applies to that.
            let target = match conv {
                None => val,
                Some("r") => { let r = vm.repr(val); vm.heap.alloc(HeapObj::Str(r))? }
                Some("s") => { let d = vm.display(val); vm.heap.alloc(HeapObj::Str(d))? }
                Some(_) => return Err(cold_value("unknown conversion specifier")),
            };
            // Empty spec means str(x); use full display so containers render, not just scalars.
            let rendered = if spec.is_empty() {
                vm.display(target)
            } else {
                crate::modules::vm::handlers::format::format_value(target, &spec, &vm.heap).map_err(crate::modules::vm::handlers::format::fmt_err)?
            };
            out.push_str(&rendered);
        } else if c == '}' {
            if chars.get(ci + 1) == Some(&'}') { out.push('}'); ci += 2; continue; }
            return Err(cold_value("Single '}' encountered in format string"));
        } else {
            out.push(c);
            ci += 1;
        }
    }
    vm.alloc_and_push_str(out)
}

str_transform!(casefold, |s: &str| s.to_lowercase());
str_transform!(swapcase, |s: &str| s.chars().map(|c| if c.is_uppercase() { c.to_lowercase().to_string() } else if c.is_lowercase() { c.to_uppercase().to_string() } else { c.to_string() }).collect::<String>());

/* Parse a pad-method `(width[, fill])` arg pair; `width_err` names the method for the width TypeError. */
fn width_and_fill(vm: &mut VM, pos: &[Val], width_err: &'static str) -> Result<(usize, char), VmErr> {
    if !pos[0].is_int() { return Err(cold_type(width_err)); }
    let width = pos[0].as_int().max(0) as usize;
    // User-controlled width drives the output size; cap it so a huge value errors instead of aborting in the allocator.
    if width > vm.heap.limit() { return Err(cold_heap()); }
    let fill = if pos.len() > 1 {
        let f = val_to_str(vm, pos[1])?;
        let mut cs = f.chars();
        match (cs.next(), cs.next()) { (Some(c), None) => c, _ => return Err(cold_type("The fill character must be exactly one character long")) }
    } else { ' ' };
    Ok((width, fill))
}

// `str.ljust`/`rjust(width[, fill])`: pad to width in code points.
fn justify(vm: &mut VM, recv: Val, pos: &[Val], right: bool) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let (width, fill) = width_and_fill(vm, pos, "width must be an integer")?;
    let pad = width.saturating_sub(s.chars().count());
    let fills: String = iter::repeat_n(fill, pad).collect();
    let out = if right { fills + &s } else { s + &fills };
    vm.alloc_and_push_str(out)
}
pub fn ljust(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { justify(vm, recv, pos, false) }
pub fn rjust(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { justify(vm, recv, pos, true) }

pub fn expandtabs(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    // Holds tabsize in a C int; out-of-range is an OverflowError before any expansion.
    let ts = match pos.first() {
        Some(n) if n.is_int() => {
            let raw = n.as_int();
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&raw) { return Err(cold_overflow()); }
            raw.max(0) as usize
        }
        _ => 8,
    };
    let limit = vm.heap.limit();
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for c in s.chars() {
        match c {
            // A large tabsize can blow the column count past the heap budget; bail before materialising the spaces.
            '\t' => {
                let n = if ts == 0 { 0 } else { ts - (col % ts) };
                if col.saturating_add(n) > limit { return Err(cold_heap()); }
                for _ in 0..n { out.push(' '); }
                col += n;
            }
            '\n' | '\r' => { out.push(c); col = 0; }
            _ => { if col >= limit { return Err(cold_heap()); } out.push(c); col += 1; }
        }
    }
    vm.alloc_and_push_str(out)
}

// str predicates: all chars satisfy a class, and (for cased ones) at least one cased char exists.
pub fn isspace(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_whitespace())));
    Ok(())
}
pub fn isupper(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let cased = s.chars().any(|c| c.is_uppercase() || c.is_lowercase());
    vm.push(Val::bool(cased && !s.chars().any(|c| c.is_lowercase())));
    Ok(())
}
pub fn islower(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let cased = s.chars().any(|c| c.is_uppercase() || c.is_lowercase());
    vm.push(Val::bool(cased && !s.chars().any(|c| c.is_uppercase())));
    Ok(())
}
pub fn istitle(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    // Titlecased: every cased run starts uppercase and continues lowercase; needs >=1 cased char.
    let mut prev_cased = false;
    let mut any_cased = false;
    let mut ok = true;
    for c in s.chars() {
        if c.is_uppercase() {
            if prev_cased { ok = false; break; }
            prev_cased = true; any_cased = true;
        } else if c.is_lowercase() {
            if !prev_cased { ok = false; break; }
            prev_cased = true; any_cased = true;
        } else {
            prev_cased = false;
        }
    }
    vm.push(Val::bool(ok && any_cased));
    Ok(())
}

// `str.removeprefix` / `removesuffix`, strip if present, else return unchanged.
pub fn removeprefix(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let p = val_to_str(vm, pos[0])?;
    let out = s.strip_prefix(p.as_str()).map(|t| t.to_string()).unwrap_or(s);
    vm.alloc_and_push_str(out)
}

pub fn removesuffix(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let suf = val_to_str(vm, pos[0])?;
    let out = s.strip_suffix(suf.as_str()).map(|t| t.to_string()).unwrap_or(s);
    vm.alloc_and_push_str(out)
}

// `str.splitlines()`, split on \n / \r / \r\n, dropping the separator (keepends=False).
pub fn splitlines(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    // line boundaries: \n \r \r\n \v \f \x1c \x1d \x1e \x85.
    const BREAKS: &[char] = &['\n', '\r', '\u{0b}', '\u{0c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}'];
    let mut parts: Vec<Val> = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if BREAKS.contains(&c) {
            // \r\n counts as a single boundary.
            if c == '\r' && chars.peek() == Some(&'\n') { chars.next(); }
            parts.push(vm.heap.alloc(HeapObj::Str(core::mem::take(&mut cur)))?);
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        parts.push(vm.heap.alloc(HeapObj::Str(cur))?);
    }
    vm.alloc_and_push_list(parts)
}

// `str.partition` / `rpartition`, (head, sep, tail); on miss returns (s,"","") / ("","",s).
fn partition_impl(vm: &mut VM, recv: Val, pos: &[Val], from_right: bool) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let sep = val_to_str(vm, pos[0])?;
    if sep.is_empty() { return Err(cold_value("empty separator")); }
    let hit = if from_right { s.rfind(sep.as_str()) } else { s.find(sep.as_str()) };
    let (a, b, c): (String, String, String) = match hit {
        Some(i) => (s[..i].to_string(), sep.clone(), s[i + sep.len()..].to_string()),
        None if from_right => (String::new(), String::new(), s), // miss: original at the tail
        None => (s, String::new(), String::new()), // miss: original at the head
    };
    let av = vm.heap.alloc(HeapObj::Str(a))?;
    let bv = vm.heap.alloc(HeapObj::Str(b))?;
    let cv = vm.heap.alloc(HeapObj::Str(c))?;
    vm.alloc_and_push_tuple(vec![av, bv, cv])
}
pub fn partition(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { partition_impl(vm, recv, pos, false) }
pub fn rpartition(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> { partition_impl(vm, recv, pos, true) }

// str: padding.
pub fn center(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let s = recv_str(vm, recv)?;
    let (width, fill) = width_and_fill(vm, pos, "center() width must be an integer")?;
    // Padding measured in code points, not UTF-8 bytes (Unicode parity).
    let pad = width.saturating_sub(s.chars().count());
    let left = pad / 2;
    let right = pad - left;
    let out = fill.to_string().repeat(left) + &s + &fill.to_string().repeat(right);
    vm.alloc_and_push_str(out)
}

pub fn zfill(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    if !pos[0].is_int() { return Err(cold_type("zfill() requires an integer argument")); }
    let s = recv_str(vm, recv)?;
    let width = pos[0].as_int().max(0) as usize;
    // User-controlled width drives the output size; cap it so a huge value errors instead of aborting in the allocator.
    if width > vm.heap.limit() { return Err(cold_heap()); }
    let nchars = s.chars().count();
    let out = if nchars >= width {
        s
    } else {
        let pad = "0".repeat(width - nchars);
        if s.starts_with('+') || s.starts_with('-') {
            s[..1].to_string() + &pad + &s[1..]
        } else {
            pad + &s
        }
    };
    vm.alloc_and_push_str(out)
}
