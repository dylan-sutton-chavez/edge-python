# Edge Python Struct

`struct` package for [Edge Python](https://edgepython.com), compiled to `wasm32-unknown-unknown` over the [WASM module ABI](https://edgepython.com/reference/wasm-abi). Packs values into fixed-width binary `bytes` driven by format strings so, the binary fast lane for bulk numeric data: pack once, cross the host boundary once, and the receiving side reads fixed-width values with zero parsing.

```python
from struct import pack, unpack, calcsize

buf = pack("3f", 92.5, -115.25, 0.75) # 3 floats to 12 bytes
print(unpack("3f", buf)) # [92.5, -115.25, 0.75]
print(calcsize("<qQd")) # 24
```

## Format strings

Optional byte-order prefix, then `[count]code` sequences:

| Prefix | Order |
|--------|-------|
| `<` (default), `=` | little-endian |
| `>`, `!` | big-endian |

| Code | Type | Size |
|------|------|------|
| `x` | pad byte (no value) | 1 |
| `b` / `B` | signed / unsigned | 1 |
| `?` | bool | 1 |
| `h` / `H` | signed / unsigned | 2 |
| `i` / `I` | signed / unsigned | 4 |
| `q` / `Q` | signed / unsigned | 8 |
| `f` | IEEE 754 single | 4 |
| `d` | IEEE 754 double | 8 |

Sizes are always fixed with no alignment padding, so `calcsize` is the plain sum of widths. Out-of-range ints raise `ValueError`; non-int values for int codes raise `TypeError`; `f`/`d` accept ints.

## Limitations

- `unpack` returns a `list`, not a tuple.
- No platform-native sizes or alignment mode (`@`); byte order is always explicit.
- Not implemented: `s`/`p` strings, `e` half floats, `n`/`N`, `pack_into` / `unpack_from` / `iter_unpack`.

## Build

```bash
cargo build --release --target wasm32-unknown-unknown
```

## License

Apache-2.0
