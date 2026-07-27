/*
Recursive-descent JSON parser. Composites via `Handle::new_dict`/`new_list`/`set_item`/`list.append`; primitives via `encode`. `LoadCtx` carries CPython `json.loads` hooks, each replaces the default at its production.
*/

use alloc::{format, vec::Vec};
use wasm_pdk::{encode, Error, Handle, Result, Value};

use super::tokenizer::{JsonError, Token, Tokenizer};

pub struct LoadCtx {
    pub object_hook: Option<Handle>,
    pub object_pairs_hook: Option<Handle>,
    pub parse_float: Option<Handle>,
    pub parse_int: Option<Handle>,
    pub parse_constant: Option<Handle>,
}

pub fn parse(src: &str, ctx: &LoadCtx) -> Result<Handle> {
    let mut tk = Tokenizer::new(src);
    let value = parse_value(&mut tk, ctx)?;
    match tk.next_token().map_err(to_pdk_err)? {
        Token::Eof => Ok(value),
        _ => Err(value_err(tk.pos(), "trailing data after JSON value")),
    }
}

fn parse_value(tk: &mut Tokenizer, ctx: &LoadCtx) -> Result<Handle> {
    let t = tk.next_token().map_err(to_pdk_err)?;
    parse_value_with(tk, t, ctx)
}

fn parse_value_with(tk: &mut Tokenizer, t: Token, ctx: &LoadCtx) -> Result<Handle> {
    match t {
        Token::Null => encode(Value::None),
        Token::True => encode(Value::Bool(true)),
        Token::False => encode(Value::Bool(false)),
        Token::Int(i, src) => {
            if let Some(hook) = &ctx.parse_int {
                let arg = encode(Value::Bytes(src.into_bytes()))?;
                hook.call("__call__", &[arg.raw()])
            } else {
                encode(Value::Int(i))
            }
        }
        Token::Float(f, src) => {
            if let Some(hook) = &ctx.parse_float {
                let arg = encode(Value::Bytes(src.into_bytes()))?;
                hook.call("__call__", &[arg.raw()])
            } else {
                encode(Value::Float(f))
            }
        }
        Token::Constant(name) => {
            if let Some(hook) = &ctx.parse_constant {
                let arg = encode(Value::Bytes(name.into_bytes()))?;
                hook.call("__call__", &[arg.raw()])
            } else {
                encode(Value::Float(match name.as_str() {
                    "NaN" => f64::NAN,
                    "Infinity" => f64::INFINITY,
                    "-Infinity" => f64::NEG_INFINITY,
                    _ => return Err(value_err(tk.pos(), "unknown constant")),
                }))
            }
        }
        Token::Str(s) => encode(Value::Bytes(s.into_bytes())),
        Token::LBracket => parse_array(tk, ctx),
        Token::LBrace => parse_object(tk, ctx),
        Token::RBracket | Token::RBrace | Token::Comma | Token::Colon => {
            Err(value_err(tk.pos(), "unexpected token"))
        }
        Token::Eof => Err(value_err(tk.pos(), "unexpected end of input")),
    }
}

fn parse_array(tk: &mut Tokenizer, ctx: &LoadCtx) -> Result<Handle> {
    let list = Handle::new_list()?;
    let first = tk.next_token().map_err(to_pdk_err)?;
    if matches!(first, Token::RBracket) { return Ok(list); }
    let mut current = first;
    loop {
        let item = parse_value_with(tk, current, ctx)?;
        let _ = list.call("append", &[item.raw()])?;
        match tk.next_token().map_err(to_pdk_err)? {
            Token::Comma => current = tk.next_token().map_err(to_pdk_err)?,
            Token::RBracket => return Ok(list),
            _ => return Err(value_err(tk.pos(), "expected ',' or ']' in array")),
        }
    }
}

fn parse_object(tk: &mut Tokenizer, ctx: &LoadCtx) -> Result<Handle> {
    // `object_pairs_hook` wins over `object_hook` per CPython spec: gather (key, value) pairs and call the hook on them, skipping dict build.
    if ctx.object_pairs_hook.is_some() {
        return parse_object_pairs(tk, ctx);
    }
    let dict = Handle::new_dict()?;
    let first = tk.next_token().map_err(to_pdk_err)?;
    if matches!(first, Token::RBrace) {
        return apply_object_hook(dict, ctx);
    }
    let mut current = first;
    loop {
        let key_str = match current {
            Token::Str(s) => s,
            _ => return Err(value_err(tk.pos(), "object key must be a string")),
        };
        let key = encode(Value::Bytes(key_str.into_bytes()))?;
        match tk.next_token().map_err(to_pdk_err)? {
            Token::Colon => {}
            _ => return Err(value_err(tk.pos(), "expected ':' after object key")),
        }
        let value = parse_value(tk, ctx)?;
        dict.set_item(&key, &value)?;
        match tk.next_token().map_err(to_pdk_err)? {
            Token::Comma => current = tk.next_token().map_err(to_pdk_err)?,
            Token::RBrace => return apply_object_hook(dict, ctx),
            _ => return Err(value_err(tk.pos(), "expected ',' or '}' in object")),
        }
    }
}

fn parse_object_pairs(tk: &mut Tokenizer, ctx: &LoadCtx) -> Result<Handle> {
    let mut pairs: Vec<(Handle, Handle)> = Vec::new();
    let first = tk.next_token().map_err(to_pdk_err)?;
    if !matches!(first, Token::RBrace) {
        let mut current = first;
        loop {
            let key_str = match current {
                Token::Str(s) => s,
                _ => return Err(value_err(tk.pos(), "object key must be a string")),
            };
            let key = encode(Value::Bytes(key_str.into_bytes()))?;
            match tk.next_token().map_err(to_pdk_err)? {
                Token::Colon => {}
                _ => return Err(value_err(tk.pos(), "expected ':' after object key")),
            }
            let value = parse_value(tk, ctx)?;
            pairs.push((key, value));
            match tk.next_token().map_err(to_pdk_err)? {
                Token::Comma => current = tk.next_token().map_err(to_pdk_err)?,
                Token::RBrace => break,
                _ => return Err(value_err(tk.pos(), "expected ',' or '}' in object")),
            }
        }
    }
    // Materialise as `list[list[key, val]]`; CPython hands `list[tuple]` but wasm-pdk has no `new_tuple` (the hook can `tuple(p) for p in pairs` if it needs them).
    let list = Handle::new_list()?;
    for (k, v) in pairs {
        let pair = Handle::new_list()?;
        pair.call("append", &[k.raw()])?;
        pair.call("append", &[v.raw()])?;
        list.call("append", &[pair.raw()])?;
    }
    let hook = ctx.object_pairs_hook.as_ref().unwrap();
    hook.call("__call__", &[list.raw()])
}

fn apply_object_hook(dict: Handle, ctx: &LoadCtx) -> Result<Handle> {
    if let Some(hook) = &ctx.object_hook {
        hook.call("__call__", &[dict.raw()])
    } else {
        Ok(dict)
    }
}

fn to_pdk_err(e: JsonError) -> Error {
    Error::Value(format!("{} at byte {}", e.msg, e.pos))
}

fn value_err(pos: usize, msg: &str) -> Error {
    Error::Value(format!("{} at byte {}", msg, pos))
}
