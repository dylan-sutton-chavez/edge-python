---
title: "Methods"
description: "Methods available on the built-in types."
---

`str`, `bytes`, `list`, `dict`, and `set` carry built-in methods, plus a small set on `int` and `float`. The set is curated for common operations. Missing variants are noted per section.

`tuple` and `frozenset` have no methods. `(1, 2).count(1)` raises `AttributeError`. Frozensets use the algebra operators from [Set](/language/data-types#set) instead.

```python
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

`upper`, `lower`, `capitalize`, `title`, `casefold`, `swapcase`. `title` titlecases each maximal run of letters. `casefold` is aggressive lowercasing for caseless comparison.

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

`strip`, `lstrip`, `rstrip` remove whitespace, or any character in the optional string argument.

```python
print("  hi  ".strip())
print("  hi  ".lstrip())
print("  hi  ".rstrip())
print("xxhelloxx".strip("x"))
```

```text Output
hi
hi  
  hi
hello
```

### Predicates

`isdigit`, `isalpha`, `isalnum`, `isspace`, `isupper`, `islower`, `istitle`. All return `False` on an empty string. The cased predicates also require at least one cased character. `isdigit` is Unicode-aware.

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

Not provided: `isascii`, `isidentifier`, `isnumeric`, `isdecimal`, `isprintable`.

### Search and count

`find` and `rfind` return a code-point index, or `-1` on a miss. `index` and `rindex` raise `ValueError` on a miss. `count` counts non-overlapping occurrences. `startswith` and `endswith` accept a single string or a tuple of strings. All of these take optional `start` and `end` code-point bounds.

```python
print("hello".startswith(("hi", "he")))
print("hello".endswith(("x", "lo")))
print("abcabc".find("c"))
print("abcabc".rfind("c"))
print("abcabc".find("a", 1))
print("hello".find("z"))
print("hello".count("l"))
```

```text Output
True
True
2
5
3
-1
2
```

### Split, join, replace

`split()` with no argument (or `None`) splits on whitespace runs. An explicit separator splits on every occurrence, and an empty separator raises `ValueError`. `split` and `rsplit` take an optional `maxsplit`. `replace(old, new)` takes an optional `count` cap. `splitlines()` drops the line separators and has no `keepends` mode. `partition` and `rpartition` split once into a `(head, sep, tail)` tuple. `removeprefix` and `removesuffix` strip an affix when present.

```python
print("a,b,c".split(","))
print("a,b,c".split(",", 1))
print("a b c".rsplit(" ", 1))
print("hello world".split())
print(",".join(["a", "b", "c"]))
print("aaaa".replace("a", "b", 2))
print("foobar".removeprefix("foo"))
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
['a', 'b', 'c']
('foo', ':', 'bar:baz')
('foo:bar', ':', 'baz')
```

### Padding

`center`, `ljust`, `rjust` take `(width[, fill])`. `zfill(width)` pads with leading zeros after any sign. Widths are measured in code points, not bytes. A multi-character `fill` raises `TypeError`. `expandtabs([tabsize])` replaces tabs with spaces up to the next tab stop, default 8.

```python
print("abc".center(7, "-"))
print("hi".ljust(5, "."))
print("hi".rjust(5, "."))
print("42".zfill(5))
print("-42".zfill(5))
print("a\tbc".expandtabs(4))
print("ñ".center(5, "*"))
```

```text Output
--abc--
hi...
...hi
00042
-0042
a   bc
**ñ**
```

Not provided: `translate`, `maketrans`, `format_map`.

### Formatting

`str.format(*args)` fills positional fields. `{}` auto-numbers and `{0}` picks an index. A spec after `:` uses the [format mini-language](/language/syntax#f-strings). Keyword fields like `{name}` are not supported.

The `%` operator does printf-style formatting. Supported verbs are `%s %r %d %i %u %x %X %o %f %F %e %E %g %G %c %%`, with flags, width, and `.precision`. `*` reads the width or precision from the next argument. A tuple on the right spreads into the fields, any other value is a single argument.

```python
print("{} and {}".format("a", "b"))
print("{0}-{1}-{0}".format("x", "y"))
print("{:>8}".format("hi"))
print("%d apples, %.1f kg" % (3, 1.5))
print("%05.2f|%-6s|" % (3.1, "hi"))
```

```text Output
a and b
x-y-x
      hi
3 apples, 1.5 kg
03.10|hi    |
```

### Encoding

`s.encode([encoding])` returns bytes. The encodings are `"utf-8"` (the default), `"utf8"`, and `"ascii"`. ASCII raises `ValueError` on non-ASCII input, and any other encoding name raises `ValueError`.

```python
print("café".encode())
print("hi".encode("ascii"))
```

```text Output
b'caf\xc3\xa9'
b'hi'
```

## Bytes methods

`decode([encoding[, errors]])` returns a string. The encodings match `str.encode`. The `errors` handler is `"strict"` (the default, raises `ValueError` on invalid UTF-8), `"ignore"` (drops bad bytes), or `"replace"` (substitutes U+FFFD).

`hex()` returns lowercase hex with no separator option. `startswith` and `endswith` take a single bytes value, no tuple form. `find` returns a byte offset or `-1`, and `index` raises `ValueError` on a miss. `count` counts non-overlapping occurrences. `replace(old, new)` has no count cap. `split(sep)` requires an explicit separator. `lower` and `upper` case-fold ASCII bytes only. `strip`, `lstrip`, `rstrip` trim ASCII whitespace or any byte in the optional argument. `join` concatenates an iterable of bytes. `bytes.fromhex(s)` parses a hex string, ignoring whitespace.

`bytearray` and `memoryview` do not exist.

```python
b = b"\x48\x65\x6c\x6c\x6f"

print(b.decode())
print(b.hex())
print(b.startswith(b"He"))
print(b.endswith(b"lo"))
print(b.find(b"ll"))
print(b.count(b"l"))
print(b.replace(b"l", b"L"))
print(b"a,b,c".split(b","))
print(b"ABc".lower())
print(b"  hi  ".strip())
print(b"-".join([b"a", b"b", b"c"]))
print(bytes.fromhex("48 65 6c 6c 6f"))
print(b"\xff".decode("utf-8", "replace"))
```

```text Output
Hello
48656c6c6f
True
True
2
2
b'HeLLo'
[b'a', b'b', b'c']
b'abc'
b'hi'
b'a-b-c'
b'Hello'
�
```

## List methods

### Query

`index(value[, start[, end]])` returns the first match and raises `ValueError` on a miss. Negative bounds count from the end. `count(value)` counts matches. `copy()` returns a shallow copy.

```python
xs = [1, 2, 3, 2]

print(xs.index(2))
print(xs.index(2, 2))
print(xs.count(2))

ys = xs.copy()
ys.append(99)
print(xs)
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

These return `None` and mutate in place. `append(x)` adds one item. `extend(iter)` adds every item of any iterable. `insert(i, x)` inserts at an index. `remove(x)` deletes the first match and raises `ValueError` on a miss. `pop()` removes and returns the last item, `pop(i)` by index. Both raise `IndexError` when the index is invalid. `sort()` accepts `key=fn` and `reverse=True` and orders objects by their `__lt__`. `reverse()` flips in place. `clear()` empties the list.

```python
xs = [1, 2, 3]

xs.append(4)
xs.extend(range(5, 7))
xs.insert(0, 99)
print(xs)

xs.remove(99)
print(xs.pop(), xs)
print(xs.pop(0), xs)
```

```text Output
[99, 1, 2, 3, 4, 5, 6]
6 [1, 2, 3, 4, 5]
1 [2, 3, 4, 5]
```

```python
xs = [3, 1, 4, 1, 5]
xs.sort()
print(xs)

xs.sort(reverse=True)
print(xs)

words = ["banana", "apple", "kiwi"]
words.sort(key=len)
print(words)
```

```text Output
[1, 1, 3, 4, 5]
[5, 4, 3, 1, 1]
['kiwi', 'apple', 'banana']
```

## Dict methods

### Views

`keys`, `values`, `items` return concrete list snapshots, not live views. Later mutations of the dict do not affect a captured snapshot.

```python
d = {"a": 1, "b": 2, "c": 3}

print(d.keys())
print(d.values())
print(d.items())

k = d.keys()
d["d"] = 4
print(k)
```

```text Output
['a', 'b', 'c']
[1, 2, 3]
[('a', 1), ('b', 2), ('c', 3)]
['a', 'b', 'c']
```

### Lookup

`get(key)` returns the value or `None`. `get(key, default)` returns `default` on a miss.

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

`update(src)` merges a dict, an iterable of length-2 pairs, or keyword arguments. `pop(key)` removes and returns the value, raising `KeyError` on a miss unless a default is given. `popitem()` removes and returns the last-inserted `(key, value)` pair and raises `KeyError` on an empty dict. `setdefault(key, default)` inserts only when the key is missing and returns the stored value. `clear()` empties the dict in place, so aliases see the change. `copy()` returns a shallow copy. `dict.fromkeys(iterable[, value])` builds a new dict mapping each key to `value`, default `None`.

```python
d = {"a": 1}

d.update({"b": 2, "a": 99})
d.update([("c", 3)], e=5)
print(d)

print(d.pop("a"), d)
print(d.pop("missing", "fallback"))
print(d.setdefault("b", 0))
print(dict.fromkeys(["x", "y"], 0))
print(d.popitem())
```

```text Output
{'a': 99, 'b': 2, 'c': 3, 'e': 5}
99 {'b': 2, 'c': 3, 'e': 5}
fallback
2
{'x': 0, 'y': 0}
('e', 5)
```

## Set methods

These exist on `set` only. Frozensets use the operators and comparisons from [Set](/language/data-types#set).

### Mutation

`add(x)` inserts. `remove(x)` deletes and raises `KeyError` on a miss. `discard(x)` deletes silently. `pop()` removes and returns an arbitrary element and raises `KeyError` on an empty set. `update(*iterables)` inserts from any number of iterables. `clear()` empties the set. `copy()` returns a shallow copy.

```python
s = {1, 2, 3}

s.add(4)
print(s)

s.remove(2)
s.discard(99)
print(s)

print(s.pop() in {1, 3, 4})

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

`union`, `intersection`, `difference` return fresh sets and accept any number of iterable arguments. `symmetric_difference` takes exactly one. `intersection_update`, `difference_update`, `symmetric_difference_update` mutate the receiver. `issubset`, `issuperset`, `isdisjoint` test relations. The named methods accept any iterable, while the operator forms (`|`, `&`, `-`, `^`) require a set or frozenset on both sides.

```python
a = {1, 2, 3}
b = {3, 4, 5}

print(a | b)
print(a & b)
print(a - b)
print(a ^ b)

print(sorted({1, 2}.union([2, 3], range(4, 6))))

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
[1, 2, 3, 4, 5]
{2, 4}
True
True
True
```

## int and float methods

`int` exposes `bit_length()` (bits needed for the absolute value, `0` for zero), `bit_count()` (number of set bits), and `to_bytes(length=1, byteorder='big')` (unsigned, `OverflowError` when the value does not fit or is negative). `int.from_bytes(bytes, byteorder='big')` is a classmethod, unsigned, raising `OverflowError` past the 128-bit range. `float` exposes `is_integer()`.

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

The standalone [`int_to_bytes`, `int_from_bytes`, `bytes_fromhex`](/reference/builtins#bytes-helpers) functions do similar jobs but are fixed-arity, capped at 8 bytes, and reject negative ints with `ValueError`.
