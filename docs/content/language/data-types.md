---
title: "Data types"
description: "Numbers, strings, sequences, mappings, sets, and None."
---

## Type checks

```python
print(type(42))
print(type(3.14))
print(type("hi"))
print(type([1, 2]))
print(type((1, 2)))
print(type({1, 2}))
print(type({"a": 1}))
print(type(None))
print(type(True))
```

```text Output
<class 'int'>
<class 'float'>
<class 'str'>
<class 'list'>
<class 'tuple'>
<class 'set'>
<class 'dict'>
<class 'NoneType'>
<class 'bool'>
```

For runtime membership in a type, use [`isinstance`](/reference/builtins#isinstance).

## Integer

One `int` type, transparently two-tier. A fast inline path auto-promotes to wide integers, capped at ±2¹²⁷ (full mechanics in [Integer width](/reference/limits-and-errors#integer-width)). No CPython unbounded ints. No complex (`1j`, `2+3j`).

```python
# Modular exponentiation
print(pow(7, 13, 19))
print(divmod(17, 5))
```

```text Output
7
(3, 2)
```

## Float

IEEE-754 double precision. Mixed arithmetic with int coerces to float.

```python
print(0.1 + 0.2 == 0.3)
print(-0.0 == 0.0)
print(1 / 3)
print(round(2.5)) # banker's rounding
print(round(0.5))
print(round(1.55, 1))
```

```text Output
False
True
0.3333333333333333
2
0
1.6
```

## String

Strings are immutable. Indexing returns a single-character string.

```python
s = "hello"
print(s[0], s[-1])
print(s[1:4])
print(len(s))
print(s + " world")
print(s * 2)
print("ll" in s)
```

```text Output
h o
ell
5
hello world
hellohello
True
```

Iteration yields characters:

```python
for ch in "abc":
  print(ch)
```

```text Output
a
b
c
```

`len(s)` measures code points, not bytes; padding methods do the same (see [Methods, padding](/reference/methods#padding)).

## Bytes

Immutable sequence of bytes (each 0-255). Distinct from `str`: it stores raw octets, not Unicode. Indexing returns an `int`, not a single-byte slice.

```python
data = b"hello"
print(data)
print(type(data))
print(len(data))
print(data[0]) # int, the byte value
print(data[1:4]) # bytes, slice
```

```text Output
b'hello'
<class 'bytes'>
5
104
b'ell'
```

```python
# Hex escapes for arbitrary bytes
raw = b"\x00\x01\xff"
print(raw)
print(raw.hex())
```

```text Output
b'\x00\x01\xff'
0001ff
```

```python
# Iteration yields ints, not bytes
for byte in b"abc":
  print(byte)
```

```text Output
97
98
99
```

The four constructor forms (`bytes()`, `bytes(n)`, from int iterable, from encoded string): see [bytes](/reference/builtins#bytes).

```python
# Round-tripping with str
s = "Edge Python"
encoded = s.encode("utf-8")
decoded = encoded.decode("utf-8")
print(encoded, decoded)
```

```text Output
b'Edge Python' Edge Python
```

`bytes` is hashable and comparable to other `bytes`. `bytes == str` is always `False`, even for valid UTF-8. Methods include `decode`, `hex`, `find`, `count`, `replace`, `split`, `startswith`, `endswith`, `lower`, `upper`, `strip`, `join`, and the `bytes.fromhex` classmethod (see [Methods](/reference/methods#bytes-methods)). Encodings: `"utf-8"` (default), `"ascii"`.

## List

Mutable sequence.

```python
xs = [1, 2, 3]
xs[0] = 99
xs.append(4)
print(xs)
print(len(xs))

# Aliasing, both names see mutation
ys = xs
ys.append(5)
print(xs)
```

```text Output
[99, 2, 3, 4]
4
[99, 2, 3, 4, 5]
```

```python
# Equality is structural
print([1, 2, 3] == [1, 2, 3])
print([1, [2, 3]] == [1, [2, 3]])
```

```text Output
True
True
```

```python
# Slice assignment (step=1) resizes the list in place
xs = [1, 2, 3, 4, 5]
xs[1:3] = [20, 30, 40]
print(xs)

# Slice deletion
del xs[2:4]
print(xs)

# Insertion via empty slice
xs[1:1] = [99]
print(xs)
```

```text Output
[1, 20, 30, 40, 4, 5]
[1, 20, 4, 5]
[1, 99, 20, 4, 5]
```

`+=` on a list extends in place, so aliases see it:

```python
xs = [1, 2, 3]
ys = xs
xs += [4]
print(ys)
```

```text Output
[1, 2, 3, 4]
```

## Tuple

Immutable sequence. It is the fastest container for fixed-size data. It is also the usual hashable container for compound dict keys (frozensets also work).

```python
t = (1, 2, 3)
print(t[0])
print(t + (4, 5))
print((1,)) # one-element needs trailing comma
print(()) # empty
```

```text Output
1
(1, 2, 3, 4, 5)
(1,)
()
```

## Dict

Insertion-ordered mapping. Keys must be hashable: numbers, strings, bytes, bools, `None`, frozensets, tuples of hashables. Mutable containers as keys -> `TypeError: unhashable type`. Numerically equal keys (`1`/`1.0`, `True`/`1`) collapse. The second insertion overwrites.

```python
d = {"a": 1, "b": 2}
print(d["a"])
d["c"] = 3
print(d)
print(list(d.keys()))
print(list(d.values()))
print(list(d.items()))
```

```text Output
1
{'a': 1, 'b': 2, 'c': 3}
['a', 'b', 'c']
[1, 2, 3]
[('a', 1), ('b', 2), ('c', 3)]
```

```python
# Iteration yields keys
for k in {"x": 1, "y": 2}:
  print(k)
```

```text Output
x
y
```

## Set

Unordered, no duplicates, hashable values. Mutators (`add`, `remove`, `discard`, `pop`, `clear`, `update`) and algebraic operators (`|`, `&`, `-`, `^` and named methods). See [Methods](/reference/methods). Augmented `|=` `&=` `^=` mutate the set in place (aliases observe it); plain `|` `&` `^` build a new set.

```python
s = {1, 2, 2, 3}
s.add(4)
print(sorted(s)) # set order is implementation-defined; sort to compare
print(len(s))

# Empty set literal is set(), not {}
print(set())
print(type({})) # this is a dict

# Algebra
print(sorted({1, 2, 3} | {3, 4}))
print(sorted({1, 2, 3} & {2, 3, 4}))
print({1, 2} <= {1, 2, 3}) # subset
```

```text Output
[1, 2, 3, 4]
4
set()
<class 'dict'>
[1, 2, 3, 4]
[2, 3]
True
```

## Frozenset

Immutable, hashable set. Build with `frozenset(iterable)`. Supports `len`, iteration, membership (`in`), tuple-unpacking (`a, b = fs`), the algebra operators `|` `&` `-` `^`, the subset/superset comparisons `<` `<=` `>` `>=` `==` `!=`, and use as a dict key or set element. In mixed `set` / `frozenset` algebra the result takes the **left** operand's type (`frozenset | set` is a `frozenset`, `set | frozenset` is a `set`).

It has **no methods**: the named set operations (`union`, `intersection`, `difference`, `symmetric_difference`, `issubset`, `issuperset`, `isdisjoint`) and `copy` raise `AttributeError`. Use the operators, or convert with `set(fs)` for the named-method / mutating API.

```python
fs = frozenset({1, 2, 3})
print(len(fs))
print(2 in fs)

a, b, c = fs # tuple-unpacking
print(sorted([a, b, c]))

# Operators yield a set; the result type follows the left operand (see above).
print(sorted(fs | frozenset({4})))
print(sorted(fs - frozenset({1})))
print(fs <= frozenset({1, 2, 3, 4})) # subset

print({fs: "ok"}[frozenset({3, 2, 1})]) # hashable: usable as a key
```

```text Output
3
True
[1, 2, 3]
[1, 2, 3, 4]
[2, 3]
True
ok
```

## Unpacking in literals

`*` spreads an iterable into a list/set literal. `**` spreads a mapping into a dict literal. Mix freely with regular elements. For dicts, later keys win.

```python
xs = [1, 2]
print([*xs, 3, *xs]) # list spread
print({*xs, 2, 3})   # set spread (deduped)

a = {"x": 1}
print({**a, "y": 2, **{"x": 9}}) # dict spread, later key wins
```

```text Output
[1, 2, 3, 1, 2]
{1, 2, 3}
{'x': 9, 'y': 2}
```

## Range

Lazy integer sequence. `range(stop)`, `range(start, stop)`, `range(start, stop, step)`.

```python
print(list(range(5)))
print(list(range(2, 8)))
print(list(range(0, 10, 2)))
print(list(range(10, 0, -1)))
```

```text Output
[0, 1, 2, 3, 4]
[2, 3, 4, 5, 6, 7]
[0, 2, 4, 6, 8]
[10, 9, 8, 7, 6, 5, 4, 3, 2, 1]
```

## NoneType

Single value, single type.

```python
print(None)
print(None is None)
print(type(None))
```

```text Output
None
True
<class 'NoneType'>
```

## Ellipsis

`...` is a singleton of type `ellipsis`. It compares equal only to itself. It is distinct from `'...'`.

```python
print(...)
print(... is ...)
print(type(...))
print(... == '...')
```

```text Output
Ellipsis
True
<class 'ellipsis'>
False
```

## Conversions

Every constructor (`int`, `float`, `str`, `bool`, `list`, `tuple`, `set`) doubles as a converter; the full matrix lives in [Built-in functions](/reference/builtins#type-conversion). The two gotchas worth remembering:

```python
print(int(3.7)) # truncates toward zero, no rounding
print(int(-3.7))
print(bool([])) # empty collections are falsy
print(bool([0])) # non-empty is truthy, even [0]
```

```text Output
3
-3
False
True
```

## Truthy and falsy

Falsy values (everything else is truthy):

| Falsy values |
|---------------------|
| `None` |
| `False` |
| `0`, `0.0` |
| `""` (empty string) |
| `[]`, `()` |
| `{}`, `set()` |
| `range(0)` |
