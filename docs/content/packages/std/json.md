---
title: "json (web, native)"
description: "JSON parsing and serialization."
---

`json` is `loads` and `dumps`. Import it by bare name, both runtimes resolve it with no manifest. To pin a different version, import it by URL or through a `packages.json` alias, see [Modules](/reference/modules#packagesjson).

`loads` accepts the `object_hook`, `object_pairs_hook`, `parse_float`, `parse_int`, and `parse_constant` callables. `dumps` accepts `indent`, `sort_keys`, `ensure_ascii`, `check_circular`, `allow_nan`, `skipkeys`, `default`, `separators`, and `cls`. Parse failures raise `ValueError`, and a non-serializable value raises `TypeError` unless `default` handles it. Integers cross as 128-bit values, so `loads` accepts numbers beyond 64 bits. Non-finite floats map to `NaN`, `Infinity`, and `-Infinity` on both sides, and integer-valued floats dump with a trailing `.0` so a round-trip preserves the type.

```python
from json import dumps, loads

data = loads('{"name":"ada","tags":["math","cs"]}')
print(data["name"])
print(dumps({"k": [1, 2, 3], "ok": True}))
```

```text Output
ada
{"k":[1,2,3],"ok":true}
```

`indent` and `sort_keys` shape the output:

```python
from json import dumps

print(dumps({"b": 1, "a": [True, None]}, indent=2, sort_keys=True))
```

```text Output
{
  "a": [
    true,
    null
  ],
  "b": 1
}
```

A round-trip preserves types, even past 64 bits:

```python
from json import dumps, loads

big = 2 ** 100
print(loads(dumps(big)) == big)
print(dumps(3.0))
print(loads(dumps(3.0)))
```

```text Output
True
3.0
3.0
```

Failures raise ordinary exceptions you can catch:

```python
from json import dumps, loads

try:
    loads("{nope")
except ValueError as e:
    print("parse failed:", e)

def helper():
    pass

try:
    dumps({"fn": helper})
except TypeError as e:
    print("dump failed:", e)
```

```text Output
parse failed: unknown literal at byte 1
dump failed: 'function' is not JSON-serializable
```
