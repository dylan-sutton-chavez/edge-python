#![no_std]
#![cfg_attr(target_arch = "wasm32", no_main)]

extern crate alloc;

use alloc::string::{String, ToString};
use wasm_pdk::*;

module_fixed_pool!();

pub mod parser;
pub mod serializer;
pub mod tokenizer;

#[plugin_fn]
fn loads(text: String, kw: Kwargs) -> Result<Handle> {
    let ctx = parser::LoadCtx {
        object_hook: kw.get_handle("object_hook")?,
        object_pairs_hook: kw.get_handle("object_pairs_hook")?,
        parse_float: kw.get_handle("parse_float")?,
        parse_int: kw.get_handle("parse_int")?,
        parse_constant: kw.get_handle("parse_constant")?,
    };
    parser::parse(&text, &ctx)
}

#[plugin_fn]
fn dumps(value: Handle, kw: Kwargs) -> Result<String> {
    let mut opts = serializer::Options {
        indent: kw.get::<i64>("indent")?,
        sort_keys: kw.get::<bool>("sort_keys")?.unwrap_or(false),
        ensure_ascii: kw.get::<bool>("ensure_ascii")?.unwrap_or(true),
        check_circular: kw.get::<bool>("check_circular")?.unwrap_or(true),
        allow_nan: kw.get::<bool>("allow_nan")?.unwrap_or(true),
        skipkeys: kw.get::<bool>("skipkeys")?.unwrap_or(false),
        cls: kw.get_handle("cls")?,
        default: kw.get_handle("default")?,
        item_sep: ",".to_string(),
        key_sep: ":".to_string(),
    };
    // `separators` arrives as a 2-tuple `(item, key)`, indexed manually since `Kwargs::get` can't decode tuples.
    if let Some(seps) = kw.get_handle("separators")? {
        let zero = encode(Value::Int(0))?;
        let one = encode(Value::Int(1))?;
        let item = seps.get_item(&zero)?;
        let key = seps.get_item(&one)?;
        opts.item_sep = decode_str(&item, "separators[0]")?;
        opts.key_sep = decode_str(&key, "separators[1]")?;
    } else if opts.indent.is_some() {
        // Per Python, when `indent` is given and `separators` is not, the default key separator becomes `": "`.
        opts.key_sep = ": ".to_string();
    }
    serializer::serialize(&value, opts)
}

fn decode_str(h: &Handle, what: &str) -> Result<String> {
    match decode(h.raw())? {
        Value::Bytes(b) => String::from_utf8(b).map_err(|e| Error::Value(alloc::format!("{} not UTF-8: {}", what, e))),
        _ => Err(Error::Type(alloc::format!("{} must be str", what))),
    }
}
