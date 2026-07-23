---
title: "Methods"
description: "Built-in methods on strings, bytes, lists, dicts, sets, ints, and floats."
---

Built-in methods on `str`, `bytes`, `list`, `dict`, `set`, plus a small set on `int` and `float`. The set is curated for common operations. Rarely used variants are omitted, documented per section.

`int` exposes `bit_length`, `bit_count`, `to_bytes`, and the `from_bytes` classmethod; `float` exposes `is_integer` (see [int / float methods](#int-and-float-methods)). `tuple` has no methods: `(1, 2).count(1)` raises `AttributeError`; use `sum(1 for x in t if x == v)`.

```python
# Methods are accessed with dot notation
print("hello".upper())
print([3, 1, 2].count(1))
print({"a": 1}.get("a"))
```

```text Output
HELLO
1
1
```

## String methods

### Case transforms

`title` titlecases each word (a maximal run of letters); digits and punctuation are word boundaries. `casefold` is aggressive lowercasing for caseless comparison. `swapcase` flips each letter's case.

```python
print("hello".upper())
print("HELLO".lower())
print("hello world".capitalize())
print("hello WORLD".title())
print("Hello".casefold())
print("Hello World".swapcase())
```

```text Output
HELLO
hello
Hello world
Hello World
hello
hELLO wORLD
```

### Whitespace

```python
print("  hi  ".strip())
print("  hi  ".lstrip())
print("  hi  ".rstrip())

# With a custom strip set
print("xxhelloxx".strip("x"))
```

```text Output
hi
hi  
  hi
hello
```

### Predicates

`isdigit` (Unicode-aware), `isalpha`, `isalnum`, `isspace`, plus the cased predicates `isupper` / `islower` / `istitle`. The cased ones need at least one cased character. All return `False` on empty string.

```python
print("123".isdigit())
print("abc".isalpha())
print("abc123".isalnum())
print("   ".isspace())

print("ABC".isupper())
print("abc".islower())
print("Hello World".istitle())
```

```text Output
True
True
True
True
True
True
True
```

Not provided: `isascii`, `isidentifier`, `isnumeric`, `isdecimal`, `isprintable`. Write the predicate explicitly.

### Search and count

`find` / `rfind` return a code-point index (not a byte offset), `-1` if missing. `index` / `rindex` raise `ValueError` instead of returning `-1`. `find`, `rfind`, `index`, `rindex`, `count` take optional `start` / `end` code-point positions. `startswith` / `endswith` accept a single string or a tuple of strings.

```python
print("hello".startswith("he"))
print("hello".startswith(("hi", "he"))) # tuple of prefixes
print("hello".endswith(("x", "lo")))
print("abcabc".find("c"))
print("abcabc".rfind("c"))
print("abcabc".find("a", 1)) # search from index 1
print("hello".find("z"))
print("hello".count("l"))
```

```text Output
True
True
True
2
5
3
-1
2
```

### Split, join, replace

`split` with no arg (or `None`) splits on whitespace runs. An explicit separator splits on every occurrence; an empty separator raises `ValueError`. Both `split` and `rsplit` take an optional `maxsplit`. `replace` takes an optional `count` cap. `splitlines` drops separators (no `keepends`) and recognises `\n \r \r\n \v \f` and the Unicode line breaks.

```python
print("a,b,c".split(","))
print("a,b,c".split(",", 1)) # maxsplit
print("a b c".rsplit(" ", 1)) # from the right
print("hello world".split()) # any whitespace
print(",".join(["a", "b", "c"]))
print("aaaa".replace("a", "b", 2)) # count cap
print("foobar".removeprefix("foo"))
print("foobar".removesuffix("bar"))
print("a\nb\nc".splitlines())
print("foo:bar:baz".partition(":"))
print("foo:bar:baz".rpartition(":"))
```

```text Output
['a', 'b', 'c']
['a', 'b,c']
['a b', 'c']
['hello', 'world']
a,b,c
bbaa
bar
foo
['a', 'b', 'c']
('foo', ':', 'bar:baz')
('foo:bar', ':', 'baz')
```

### Padding

`center`, `ljust`, `rjust` take `(width[, fill])`; `zfill(width)` pads with leading zeros. All measure in code points, not bytes, so `'ñ'.center(5, '*')` produces `**ñ**` (5 visible chars). A multi-character `fill` raises `TypeError`. `expandtabs([tabsize])` replaces tabs with spaces to the next tab stop (default 8). Not provided: `translate`, `maketrans`, `format_map`.

```python
print("abc".center(7, "-"))
print("hi".ljust(5, "."))
print("hi".rjust(5, "."))
print("42".zfill(5))
print("-42".zfill(5))
print("a\tbc".expandtabs(4))
```

```text Output
--abc--
hi...
...hi
00042
-0042
a   bc
```

### Formatting

`str.format(*args)` fills positional fields (`{}` auto-numbered, `{0}` explicit) with the f-string [format mini-language](/language/syntax#f-strings) after `:`. Keyword fields (`"{name}".format(name=...)`) are not supported, use positional indices. The `%` operator does printf-style formatting (`%s %r %d %i %x %X %o %f %e %g %c %%`, with flags / width / `.precision`, where `*` reads the width or precision from the next argument); a tuple spreads, any other value is a single argument.

```python
print("{} and {}".format("a", "b"))
print("{0}-{1}-{0}".format("x", "y"))
print("{:>8}".format("hi"))
print("{:,}".format(1234567))
print("%d apples, %.1f kg" % (3, 1.5))
print("%05.2f|%-6s|" % (3.1, "hi"))
```

```text Output
a and b
x-y-x
      hi
1,234,567
3 apples, 1.5 kg
03.10|hi    |
```

### Encoding

`s.encode([encoding])` -> bytes. Supports `"utf-8"`, `"utf8"`, `"ascii"` (errors on non-ASCII). Else `ValueError`. Default `"utf-8"`.

```python
print("café".encode())
print("hi".encode("ascii"))
```

```text Output
b'caf\xc3\xa9'
b'hi'
```

## Bytes methods

`bytes.decode([encoding[, errors]])` takes `encoding` (`utf-8` or `ascii`) and an `errors` handler: `strict` (default) raises on invalid UTF-8, `ignore` drops the bad bytes, `replace` substitutes `U+FFFD`. `bytes.find` returns a byte offset (not a code-point index). `bytes.index` raises `ValueError` if absent. `split` needs an explicit separator (no whitespace-split mode). `lower` / `upper` case-fold ASCII bytes; `strip` / `lstrip` / `rstrip` trim ASCII whitespace (or any byte in the optional set). `join` concatenates an iterable of bytes. `bytes.fromhex(s)` parses a hex string (whitespace ignored). `bytearray` and `memoryview` are unimplemented.

```python
b = b"\x48\x65\x6c\x6c\x6f"

print(b.decode()) # default utf-8
print(b.hex()) # no separator argument
print(b.startswith(b"He"))
print(b.find(b"ll"))
print(b.count(b"l"))
print(b.replace(b"l", b"L"))
print(b"a,b,c".split(b","))
print(b"ABc".lower())
print(b"  hi  ".strip())
print(b"-".join([b"a", b"b", b"c"]))
print(bytes.fromhex("48 65 6c 6c 6f"))
```

```text Output
Hello
48656c6c6f
True
2
2
b'HeLLo'
[b'a', b'b', b'c']
b'abc'
b'hi'
b'a-b-c'
b'Hello'
```

## List methods

### Pure (return a new value or query)

`index` accepts optional `start` / `end` bounds (negatives count from the end) and raises `ValueError` if the value isn't found in that range; `copy` returns a shallow copy.

```python
xs = [1, 2, 3, 2]

print(xs.index(2))
print(xs.index(2, 2)) # search from index 2
print(xs.count(2))

ys = xs.copy()
ys.append(99)
print(xs) # original unchanged
print(ys)
```

```text Output
1
3
2
[1, 2, 3, 2]
[1, 2, 3, 2, 99]
```

### Mutating

Return `None`, mutate in place. `extend` accepts any iterable. `sort` accepts `key=fn` and `reverse=True/False`, and orders custom objects by their `__lt__`.

```python
xs = [1, 2, 3]

xs.append(4)
print(xs)

xs.extend(range(5, 7)) # any iterable
print(xs)

xs.insert(0, 99)
print(xs)
```

```text Output
[1, 2, 3, 4]
[1, 2, 3, 4, 5, 6]
[99, 1, 2, 3, 4, 5, 6]
```

```python
xs = [1, 2, 3, 2]

xs.remove(2) # first occurrence
print(xs)

popped = xs.pop() # last
print(popped, xs)

popped = xs.pop(0) # by index
print(popped, xs)
```

```text Output
[1, 3, 2]
2 [1, 3]
1 [3]
```

```python
xs = [3, 1, 4, 1, 5]
xs.sort()
print(xs)

xs.reverse()
print(xs)

xs.clear()
print(xs)
```

```text Output
[1, 1, 3, 4, 5]
[5, 4, 3, 1, 1]
[]
```

## Dict methods

### Views

`keys`, `values`, `items` return concrete `list` snapshots, not live views. Dict mutations don't affect captured snapshots. This is intentional: live views are shared mutable state, which conflicts with the functional paradigm.

```python
d = {"a": 1, "b": 2, "c": 3}

print(list(d.keys()))
print(list(d.values()))
print(list(d.items()))

# Snapshot is detached from the dict
k = d.keys()
d["d"] = 4
print(k) # ['a', 'b', 'c'] pre-mutation
```

```text Output
['a', 'b', 'c']
[1, 2, 3]
[('a', 1), ('b', 2), ('c', 3)]
['a', 'b', 'c']
```

### Lookup with default

```python
d = {"a": 1}

print(d.get("a"))
print(d.get("z"))
print(d.get("z", 0))
```

```text Output
1
None
0
```

### Mutation

`update` accepts a `dict`, an iterable of length-2 sequences, or keyword arguments (`d.update(a=1)`). `popitem` returns the last-inserted entry; empty -> `KeyError`. `pop(key)` on a missing key -> `KeyError` unless a default is given. The `dict.fromkeys(iterable[, value])` classmethod builds a new dict mapping each key to `value` (default `None`). `clear` empties the dict in place, so shared references see it.

```python
d = {"a": 1}

d.update({"b": 2, "a": 99})
print(d)

# Iterable-of-pairs and kwargs also work
d.update([("c", 3)], e=5)
print(d)

removed = d.pop("a")
print(removed, d)

print(d.pop("missing", "fallback"))
print(dict.fromkeys(["x", "y"], 0))
```

```text Output
{'a': 99, 'b': 2}
{'a': 99, 'b': 2, 'c': 3, 'e': 5}
99 {'b': 2, 'c': 3, 'e': 5}
fallback
{'x': 0, 'y': 0}
```

```python
d = {}
d.setdefault("a", 1)
d.setdefault("a", 999) # second call ignored
print(d)

alias = d
d.clear()
print(d, alias)
```

```text Output
{'a': 1}
{} {}
```

## Set methods

These are `set`-only. `frozenset` exposes no methods; it uses the algebra operators (`|` `&` `-` `^`) and comparisons instead. See [Frozenset](/language/data-types#frozenset).

### Mutation

`remove` raises `KeyError` if absent. `discard` is silent. `pop` removes an arbitrary element; empty -> `KeyError`. `update` accepts any iterable (variadic).

```python
s = {1, 2, 3}

s.add(4)
print(s)

s.remove(2) # raises KeyError if absent
s.discard(99) # silently ignores absent values
print(s)

print(s.pop() in {1, 3, 4}) # removes an arbitrary element

s.clear()
print(s)
```

```text Output
{2, 4, 1, 3}
{4, 1, 3}
True
set()
```

### Algebra

`union`, `intersection`, `difference` return fresh sets and accept any number of iterable arguments; `symmetric_difference` takes exactly one. The in-place `intersection_update`, `difference_update`, `symmetric_difference_update` mutate the receiver. Operator forms (`|`, `&`, `-`, `^`) and augmented assignment (`|=`, `&=`, `-=`, `^=`) require both sides to be a set or frozenset; the named methods accept any iterable.

```python
a = {1, 2, 3}
b = {3, 4, 5}

print(a | b)
print(a & b)
print(a - b)
print(a ^ b)

print({1, 2}.union([2, 3], range(4, 6))) # any iterables, variadic
s = {1, 2, 3, 4}
s.intersection_update({2, 4, 6})
print(s)
print({1, 2}.issubset({1, 2, 3}))
print({1, 2, 3}.issuperset({1}))
print({1, 2}.isdisjoint({3, 4}))
```

```text Output
{5, 2, 4, 1, 3}
{3}
{1, 2}
{5, 2, 4, 1}
{5, 2, 4, 1, 3}
{2, 4}
True
True
True
```

Comparison operators between sets follow subset / superset semantics, not total order:

```python
print({1, 2} <  {1, 2, 3}) # proper subset
print({1, 2} <= {1, 2}) # subset
print({1, 2} >= {1}) # superset
print({1, 2} <= {2, 3}) # disjoint sides -> False
```

```text Output
True
True
True
False
```

## int and float methods

Small set on the numeric primitives. `int` exposes `bit_length()` (bits to represent the absolute value, `0` for `0`), `bit_count()` (number of set bits), and `to_bytes(length=1, byteorder='big')` (unsigned big/little-endian, `OverflowError` if it doesn't fit). `int.from_bytes(bytes, byteorder='big')` is a classmethod. `float` exposes `is_integer()`.

```python
print((255).bit_length())
print((255).bit_count())
print((1000).to_bytes(2, "big"))
print((1000).to_bytes(2, "little"))
print(int.from_bytes(b"\x03\xe8", "big"))
print((3.0).is_integer())
print((3.5).is_integer())
```

```text Output
8
8
b'\x03\xe8'
b'\xe8\x03'
1000
True
False
```

The standalone [`int_to_bytes` / `int_from_bytes` / `bytes_fromhex`](/reference/builtins#bytes_fromhex-int_from_bytes-int_to_bytes) builtin functions remain available and behave identically.

