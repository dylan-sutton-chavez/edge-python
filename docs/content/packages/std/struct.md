---
title: "struct (web, native)"
description: "Packs primitive values into bytes and back."
---

`struct` packs primitive values into `bytes` and back. Import it by bare name, both runtimes resolve it with no manifest. To pin a different version, use a `packages.json` alias, see [Modules](/reference/modules#packagesjson).

Functions are `pack(fmt, *values)`, `unpack(fmt, data)`, and `calcsize(fmt)`. Codes are `x b B ? h H i I q Q f d` with repeat counts, and byte-order prefixes are `<` (the default), `=`, `>`, `!`. There is no native-alignment mode. `unpack` returns a list. A packed buffer crosses the host boundary once, which makes it the fast lane for bulk numeric data. Out-of-range integers raise `ValueError`, non-integer values for integer codes raise `TypeError`, and `f` / `d` accept integers. Not implemented: `s` / `p` strings, `e` half floats, `n` / `N`, and `pack_into` / `unpack_from` / `iter_unpack`.

```python
from struct import pack, unpack, calcsize

buf = pack("3f", 92.5, -115.25, 0.75)
print(unpack("3f", buf))
print(calcsize("!hh"))
```

```text Output
[92.5, -115.25, 0.75]
4
```

The byte-order prefix picks the layout:

```python
from struct import pack, unpack

print(pack("<H", 258))
print(pack(">H", 258))
print(unpack("!I", pack("!I", 70000)))
```

```text Output
b'\x02\x01'
b'\x01\x02'
[70000]
```

A value that does not fit its code raises `ValueError`:

```python
from struct import pack

try:
    pack("B", 300)
except ValueError as e:
    print("out of range:", e)
```

```text Output
out of range: argument out of range for 'B'
```
