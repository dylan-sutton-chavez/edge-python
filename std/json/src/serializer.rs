/*
Walk a Handle, emit JSON text. Dispatch by `type_of`; sequences use `iter`+`len`+`get_item` (skip `iter_next`: wasm-pdk v0.1.0 StopIteration broken). Full CPython `json.dumps` kwargs supported.
*/

use alloc::{borrow::ToOwned, format, string::String, vec::Vec};
use wasm_pdk::{Error, FromValue, Handle, Result, Value, decode, encode};

pub struct Options {
    pub indent: Option<i64>,
    pub sort_keys: bool,
    pub ensure_ascii: bool,
    pub check_circular: bool,
    pub allow_nan: bool,
    pub skipkeys: bool,
    pub item_sep: String,
    pub key_sep: String,
    pub cls: Option<Handle>,
    pub default: Option<Handle>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent: None, sort_keys: false, ensure_ascii: true,
            check_circular: true, allow_nan: true, skipkeys: false,
            item_sep: ",".to_owned(), key_sep: ":".to_owned(),
            cls: None, default: None,
        }
    }
}

pub struct SerCtx<'a> {
    pub opts: &'a Options,
    pub depth: usize,
}

/* Recursion-depth cap for circular detection: wasm plugin can't see Val identity (CPython's `id(x) in seen`); trips before JS call stack overflow on self-reference. */
const MAX_DEPTH: usize = 200;

pub fn serialize(value: &Handle, opts: Options) -> Result<String> {
    // `cls` short-circuits the whole walk: instantiate and delegate to its `.encode(value)`.
    if let Some(cls) = &opts.cls {
        let encoder = cls.call("__call__", &[])?;
        let result = encoder.call("encode", &[value.raw()])?;
        return match decode(result.raw())? {
            Value::Bytes(b) => String::from_utf8(b).map_err(|e| Error::Value(format!("cls.encode produced non-UTF-8: {}", e))),
            _ => Err(Error::Type("cls.encode must return str".into())),
        };
    }
    let mut out = String::new();
    let mut ctx = SerCtx { opts: &opts, depth: 0 };
    serialize_into(value, &mut out, &mut ctx, 0)?;
    Ok(out)
}

fn serialize_into(value: &Handle, out: &mut String, ctx: &mut SerCtx, depth: usize) -> Result<()> {
    let ty_handle = value.type_of()?;
    let ty = String::from_handle(ty_handle.raw())?;
    match ty.as_str() {
        "NoneType" => { out.push_str("null"); Ok(()) }
        "bool" => {
            match decode(value.raw())? {
                Value::Bool(true) => out.push_str("true"),
                Value::Bool(false) => out.push_str("false"),
                _ => return Err(Error::Type("bool decoded as non-bool".into())),
            }
            Ok(())
        }
        "int" => {
            match decode(value.raw())? {
                Value::Int(i) => out.push_str(&format!("{}", i)),
                _ => return Err(Error::Type("int decoded as non-int".into())),
            }
            Ok(())
        }
        "float" => {
            match decode(value.raw())? {
                Value::Float(f) => {
                    if !f.is_finite() {
                        if !ctx.opts.allow_nan {
                            return Err(Error::Value("Out of range float values are not JSON compliant".into()));
                        }
                        out.push_str(if f.is_nan() { "NaN" } else if f > 0.0 { "Infinity" } else { "-Infinity" });
                    } else {
                        out.push_str(&format_float(f));
                    }
                }
                _ => return Err(Error::Type("float decoded as non-float".into())),
            }
            Ok(())
        }
        "str" => {
            match decode(value.raw())? {
                Value::Bytes(b) => {
                    let s = String::from_utf8(b).map_err(|e| Error::Value(format!("invalid utf-8 in str: {}", e)))?;
                    escape_string(&s, out, ctx.opts.ensure_ascii);
                }
                _ => return Err(Error::Type("str decoded as non-bytes".into())),
            }
            Ok(())
        }
        "list" | "tuple" | "set" | "frozenset" => serialize_sequence(value, out, ctx, depth),
        "dict" => serialize_object(value, out, ctx, depth),
        other => {
            if let Some(default) = &ctx.opts.default {
                let replacement = default.call("__call__", &[value.raw()])?;
                return serialize_into(&replacement, out, ctx, depth);
            }
            Err(Error::Type(format!("'{}' is not JSON-serializable", other)))
        }
    }
}

fn serialize_sequence(value: &Handle, out: &mut String, ctx: &mut SerCtx, depth: usize) -> Result<()> {
    if ctx.opts.check_circular && depth >= MAX_DEPTH {
        return Err(Error::Value("Circular reference detected".into()));
    }
    out.push('[');
    let it = value.iter()?;
    let n = it.len()?;
    if n == 0 {
        out.push(']');
        return Ok(());
    }
    let indent_str = ctx.opts.indent.map(|n| " ".repeat(n.max(0) as usize));
    for i in 0..n {
        let idx = encode(Value::Int(i as i128))?;
        let item = it.get_item(&idx)?;
        if i > 0 { out.push_str(&ctx.opts.item_sep); }
        write_indent(out, indent_str.as_deref(), depth + 1);
        serialize_into(&item, out, ctx, depth + 1)?;
    }
    write_indent(out, indent_str.as_deref(), depth);
    out.push(']');
    Ok(())
}

fn serialize_object(value: &Handle, out: &mut String, ctx: &mut SerCtx, depth: usize) -> Result<()> {
    if ctx.opts.check_circular && depth >= MAX_DEPTH {
        return Err(Error::Value("Circular reference detected".into()));
    }
    out.push('{');
    let keys = value.iter()?;
    let n = keys.len()?;
    if n == 0 {
        out.push('}');
        return Ok(());
    }
    let mut pairs: Vec<(String, Handle)> = Vec::with_capacity(n as usize);
    for i in 0..n {
        let idx = encode(Value::Int(i as i128))?;
        let key = keys.get_item(&idx)?;
        let key_ty_handle = key.type_of()?;
        let key_ty = String::from_handle(key_ty_handle.raw())?;
        if key_ty != "str" {
            if ctx.opts.skipkeys { continue; }
            return Err(Error::Type(format!("keys must be str, not {}", key_ty)));
        }
        let key_str = match decode(key.raw())? {
            Value::Bytes(b) => String::from_utf8(b).map_err(|e| Error::Value(format!("invalid utf-8 in key: {}", e)))?,
            _ => return Err(Error::Type("str key decoded as non-bytes".into())),
        };
        pairs.push((key_str, key));
    }
    if ctx.opts.sort_keys { pairs.sort_by(|a, b| a.0.cmp(&b.0)); }
    let indent_str = ctx.opts.indent.map(|n| " ".repeat(n.max(0) as usize));
    for (i, (key_str, key)) in pairs.iter().enumerate() {
        let item = value.get_item(key)?;
        if i > 0 { out.push_str(&ctx.opts.item_sep); }
        write_indent(out, indent_str.as_deref(), depth + 1);
        escape_string(key_str, out, ctx.opts.ensure_ascii);
        out.push_str(&ctx.opts.key_sep);
        serialize_into(&item, out, ctx, depth + 1)?;
    }
    write_indent(out, indent_str.as_deref(), depth);
    out.push('}');
    Ok(())
}

fn write_indent(out: &mut String, unit: Option<&str>, depth: usize) {
    if let Some(u) = unit {
        out.push('\n');
        for _ in 0..depth { out.push_str(u); }
    }
}

fn escape_string(s: &str, out: &mut String, ensure_ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if ensure_ascii && (c as u32) >= 0x80 => {
                let cp = c as u32;
                if cp <= 0xFFFF {
                    out.push_str(&format!("\\u{:04x}", cp));
                } else {
                    // UTF-16 surrogate pair for code points beyond the BMP.
                    let v = cp - 0x10000;
                    let hi = 0xD800 + (v >> 10);
                    let lo = 0xDC00 + (v & 0x3FF);
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn format_float(f: f64) -> String {
    // Integer-valued floats keep a trailing ".0" to disambiguate from int (Python's `json.dumps(1.0)` -> `"1.0"`).
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') || s.contains('E') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        let mut t = s;
        t.push_str(".0");
        t
    }
}
