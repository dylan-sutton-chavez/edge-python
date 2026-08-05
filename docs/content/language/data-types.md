---
title: "Data types"
description: "The builtin types, their values, and their behavior."
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

Use [`isinstance`](/reference/builtins) to test whether a value belongs to a type.

## Integer

One `int` type with a signed 128-bit range, from `-(2**127)` to `2**127 - 1`. Going out of range raises `OverflowError`. Details: [Limits and errors](/reference/limits-and-errors). There is no complex type.

```python
big = (2**126 - 1) * 2 + 1
print(big)
try:
  big + 1
except OverflowError:
  print("overflow")
```

```text Output
170141183460469231731687303715884105727
overflow
```

## Float

IEEE-754 double precision. Mixing int and float in arithmetic yields a float.

```python
print(0.1 + 0.2 == 0.3)
print(-0.0 == 0.0)
print(2 + 3.0)
print(round(2.5))  # ties round to even
print(round(0.5))
print(round(1.55, 1))
```

```text Output
False
True
5.0
2
0
1.6
```

## Boolean

`bool` has two values, `True` and `False`. It is a subclass of `int`.

```python
print(isinstance(True, int))
print(True + True)
```

```text Output
True
2
```

Which values count as true: [Truthy and falsy](#truthy-and-falsy) below.

## String

Immutable sequence of Unicode code points. Indexing returns a one-character string. `len` counts code points, not bytes. Iteration yields characters.

```python
s = "héllo"
print(s[0], s[-1])
print(s[1:4])
print(len(s))
print(s + " world")
print(s * 2)
print("ll" in s)
```

```text Output
h o
éll
5
héllo world
héllohéllo
True
```

Literal forms, quotes, and escapes: [Syntax](/language/syntax). Methods: [Methods](/reference/methods).

## Bytes

Immutable sequence of octets, each 0 to 255. `bytes` stores raw data, not text. Indexing returns an int, and iteration yields ints.

```python
data = b"hello"
print(data)
print(len(data))
print(data[0])  # int, the byte value
print(data[1:4])  # bytes slice

for byte in b"abc":
  print(byte)
```

```text Output
b'hello'
5
104
b'ell'
97
98
99
```

`bytes` never equals `str`, even for valid UTF-8. Non-ASCII characters in a bytes literal are stored as their UTF-8 encoding.

```python
print(b"abc" == "abc")
print(b"é")  # stored as UTF-8
```

```text Output
False
b'\xc3\xa9'
```

Encoding round-trips use `str.encode` and `bytes.decode`.

```python
encoded = "Edge Python".encode("utf-8")
print(encoded, encoded.decode("utf-8"))
```

```text Output
b'Edge Python' Edge Python
```

Constructor forms (`bytes()`, `bytes(n)`, from an int iterable, from an encoded string): [Builtins](/reference/builtins). Methods such as `hex`, `split`, and `fromhex`: [Methods](/reference/methods).

## List

Mutable sequence. Assignment copies the reference, so aliases share mutations.

```python
xs = [1, 2, 3]
xs[0] = 99
xs.append(4)
print(xs)
print(len(xs))

# Aliasing, both names see the mutation
ys = xs
ys.append(5)
print(xs)
```

```text Output
[99, 2, 3, 4]
4
[99, 2, 3, 4, 5]
```

Equality is structural.

```python
print([1, 2, 3] == [1, 2, 3])
print([1, [2, 3]] == [1, [2, 3]])
```

```text Output
True
True
```

Slice assignment with step 1 resizes the list in place. Slice deletion removes a range. Assigning into an empty slice inserts.

```python
xs = [1, 2, 3, 4, 5]
xs[1:3] = [20, 30, 40]
print(xs)

del xs[2:4]
print(xs)

xs[1:1] = [99]
print(xs)
```

```text Output
[1, 20, 30, 40, 4, 5]
[1, 20, 4, 5]
[1, 99, 20, 4, 5]
```

`+=` on a list extends it in place, so aliases see it.

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

Immutable sequence. A tuple is hashable when its elements are, which makes it the usual compound dict key.

```python
t = (1, 2, 3)
print(t[0])
print(t + (4, 5))
print({(1, 2): "key"}[(1, 2)])
```

```text Output
1
(1, 2, 3, 4, 5)
key
```

Literal forms such as the trailing comma in `(1,)`: [Syntax](/language/syntax).

## Dict

Insertion-ordered mapping. Keys must be hashable: numbers, strings, bytes, bools, `None`, frozensets, and tuples of hashables. An unhashable key raises `TypeError`. Numerically equal keys collapse, so `1`, `1.0`, and `True` are one key and the latest assignment wins. Iteration yields keys.

```python
d = {"a": 1, "b": 2}
print(d["a"])
d["c"] = 3
print(d)

e = {1: "int"}
e[1.0] = "float"
e[True] = "bool"
print(e)  # 1, 1.0, and True are one key

for k in {"x": 1, "y": 2}:
  print(k)
```

```text Output
1
{'a': 1, 'b': 2, 'c': 3}
{1: 'bool'}
x
y
```

```python
try:
  d = {[1, 2]: "x"}
except TypeError:
  print("unhashable key")
```

```text Output
unhashable key
```

Methods such as `keys`, `values`, `items`, and `get`: [Methods](/reference/methods).

## Set

Unordered collection of unique, hashable values. The operators `|`, `&`, `-`, and `^` build new sets. The augmented forms `|=`, `&=`, and `^=` update the set in place, so aliases observe the change. `<=` and `<` test subsets.

```python
s = {1, 2, 2, 3}
s.add(4)
print(len(s))
print(sorted(s))  # set order is arbitrary, sort to compare

# Algebra
print(sorted({1, 2, 3} | {3, 4}))
print(sorted({1, 2, 3} & {2, 3, 4}))
print({1, 2} <= {1, 2, 3})

# Augmented forms mutate in place
t = s
s |= {5}
print(sorted(t))
```

```text Output
4
[1, 2, 3, 4]
[1, 2, 3, 4]
[2, 3]
True
[1, 2, 3, 4, 5]
```

The empty set is `set()`, since `{}` makes a dict (see [Syntax](/language/syntax)). Named operations such as `union` and `issubset`: [Methods](/reference/methods).

## Frozenset

Immutable, hashable set built with `frozenset(iterable)`. It supports `len`, iteration, membership, unpacking, the algebra operators, and the subset comparisons. In mixed `set` and `frozenset` algebra the result takes the type of the left operand. A frozenset has no named methods. `frozenset({1}).union({2})` raises `AttributeError`. Convert with `set(fs)` for the method API.

```python
fs = frozenset({1, 2, 3})
print(len(fs))
print(2 in fs)

a, b, c = fs  # unpacking
print(sorted([a, b, c]))

print(type(fs | {4}))   # left operand wins
print(type({4} | fs))

print(sorted(fs | frozenset({4})))
print(fs <= frozenset({1, 2, 3, 4}))  # subset

print({fs: "ok"}[frozenset({3, 2, 1})])  # hashable, usable as a key
```

```text Output
3
True
[1, 2, 3]
<class 'frozenset'>
<class 'set'>
[1, 2, 3, 4]
True
ok
```

## Unpacking in literals

`*` spreads an iterable into a list or set literal. `**` spreads a mapping into a dict literal. Both mix freely with regular elements. For dicts, later keys win.

```python
xs = [1, 2]
print([*xs, 3, *xs])  # list spread
print(sorted({*xs, 2, 3}))  # set spread, deduped

a = {"x": 1}
print({**a, "y": 2, **{"x": 9}})  # dict spread, later key wins
```

```text Output
[1, 2, 3, 1, 2]
[1, 2, 3]
{'x': 9, 'y': 2}
```

## Range

Lazy integer sequence. `range(stop)`, `range(start, stop)`, or `range(start, stop, step)`.

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

`...` is a singleton of type `ellipsis`. It compares equal only to itself and is distinct from the string `'...'`.

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

Every constructor (`int`, `float`, `str`, `bool`, `list`, `tuple`, `set`) doubles as a converter. The full matrix: [Builtins](/reference/builtins). The two gotchas worth remembering:

```python
print(int(3.7))  # truncates toward zero, no rounding
print(int(-3.7))
print(bool([]))  # empty collections are falsy
print(bool([0]))  # non-empty is truthy, even [0]
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
| `""`, `b""` |
| `[]`, `()` |
| `{}`, `set()`, `frozenset()` |
| `range(0)` |

```python
for v in [None, 0, "", [], [0], "x"]:
  print(bool(v))
```

```text Output
False
False
False
False
True
True
```
