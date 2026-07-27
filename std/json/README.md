# Edge Python JSON

JSON serialization, deserialization, and round-tripping shipped as a `.wasm` plugin. Scripts see `json` as ordinary.

```python
from json import dumps, loads

data = loads('{"name":"ada","tags":["math","cs"],"score":91.5}')
print(data["name"], data["tags"][0])
print(dumps({"k": [1, 2, 3], "ok": True}))
```

## API

### Conventions

- **Types map straight through.** `null<->None`, `true/false<->bool`, JSON numbers split into `int` (i128) and `float` (f64), strings are UTF-8, arrays are `list`, objects are `dict`. Python `tuple`/`set` serialize as JSON arrays but parse back as `list`.
- **Errors raise typed exceptions.** Parser failures surface as `ValueError` with the byte offset; non-serializable values raise `TypeError` naming the offending type unless `default=` is supplied.
- **Compact output by default.** No whitespace, dict insertion order preserved. Integer-valued floats keep a trailing `.0` so a round-trip returns the same Python type.
- **CPython parity.** Both functions accept the full kwargs of `json.dumps` / `json.loads`; defaults match CPython.

### loads

```python
loads(s, *, cls=None, object_hook=None, parse_float=None,
      parse_int=None, parse_constant=None, object_pairs_hook=None)
```

```python
loads("9223372036854775808") # -> 2**63 (int via i128 wire tag)
loads("1.5e3") # -> 1500.0
loads('{"k":"v"}') # -> {"k": "v"}
loads("NaN") # -> float('nan')   (also Infinity / -Infinity)
```

Each kwarg fires at the matching production and replaces the default decoding for that node.

| Kwarg | Type | Behaviour |
| --- | --- | --- |
| `object_hook` | `callable(dict) -> any` | Called with every completed object; the return value replaces the dict. |
| `object_pairs_hook` | `callable(list[[key, val]]) -> any` | Wins over `object_hook` when both are set. Receives a list of `[key, value]` lists. |
| `parse_float` | `callable(str) -> any` | Receives the raw float source token; return value used as-is. |
| `parse_int` | `callable(str) -> any` | Receives the raw int source token; return value used as-is. |
| `parse_constant` | `callable(str) -> any` | Fires for `"NaN"`, `"Infinity"`, `"-Infinity"`. |
| `cls` | callable | Reserved for an alternate decoder class (currently behaves like default decoding). |

### dumps

```python
dumps(obj, *, skipkeys=False, ensure_ascii=True, check_circular=True, allow_nan=True, cls=None, indent=None, separators=None, default=None, sort_keys=False)
```

```python
dumps(1.0) # -> '1.0'
dumps((1, 2, 3)) # -> '[1,2,3]'
dumps({"a": 1, "b": None}) # -> '{"a":1,"b":null}'
dumps({"b": 2, "a": 1}, sort_keys=True) # -> '{"a":1,"b":2}'
dumps("héllo", ensure_ascii=False) # -> '"héllo"'
```

| Kwarg | Default | Behaviour |
| --- | --- | --- |
| `indent` | `None` | Integer pretty-print width. With indent, key separator becomes `": "` by default. |
| `sort_keys` | `False` | Sort dict keys ASCII-lexicographically before emit. |
| `ensure_ascii` | `True` | Escape characters >= U+0080 as `\uXXXX`. When `False`, emit the UTF-8 bytes directly. |
| `check_circular` | `True` | Bound recursive nesting at 200 levels; deeper raises `ValueError("Circular reference detected")`. |
| `allow_nan` | `True` | Emit `NaN` / `Infinity` / `-Infinity` for non-finite floats. With `False`, they raise `ValueError`. |
| `skipkeys` | `False` | Silently skip non-`str` dict keys instead of raising `TypeError`. |
| `separators` | `(",", ":")` compact, `(",", ": ")` with indent | Two-element tuple `(item_sep, key_sep)`. |
| `default` | `None` | Callable that receives any non-serializable value; its return is re-serialized. |
| `cls` | `None` | Encoder class with an `.encode(obj)` method; if supplied, the whole walk is delegated to it. |

### Round-tripping

`dumps(loads(s))` is idempotent for canonical shapes: output parsed then re-`dumps`'d returns the same text.

## How it works

Compiles to `wasm32-unknown-unknown` (`cdylib`) against the [wasm-pdk](https://github.com/dylan-sutton-chavez/edge-python/tree/main/wasm-pdk) `v0.1.0` ABI. `loads` builds Python values through the handle ABI (`new_dict`/`new_list`, `set_item`, primitives via `encode`); `dumps` walks the input handle with `type_of`/`iter`/`len`/`get_item`. Hooks are forwarded to the caller's Python callable via `Handle::call("__call__", args)`.

Single-pass tokenizer + recursive-descent parser, ~95 KB stripped. Plugin memory recycles per call through a static 4 MB pool, so long-running workers stay flat. The upstream wasm-pdk ABI leaks ~8 bytes per host call, so a single worker session caps at roughly 500 k plugin calls; recycle the worker periodically for unbounded streaming.

Pre-built `.wasm` is served from `https://cdn.edgepython.com/std/json.wasm`, deployed by the `Std` workflow.

## License

Apache-2.0
