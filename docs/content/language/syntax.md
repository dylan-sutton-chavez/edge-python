---
title: "Syntax"
description: "Lexical and grammatical rules of the language subset."
---

## Comments

`#` starts a comment that runs to the end of the line.

```python
# A comment on its own line
x = 1  # A trailing comment
```

A string literal as the first statement of a module, function, or class body parses and is discarded. There is no `__doc__` at runtime.

## Line joining

A statement continues on the next line after a trailing backslash, or freely inside `()`, `[]`, and `{}`.

```python
total = 1 + \
    2
names = [
  "a",
  "b",
]
print(total, names)
```

```text Output
3 ['a', 'b']
```

## Identifiers and assignment

Identifiers start with a letter or underscore and continue with letters, digits, and underscores. Non-ASCII characters are allowed.

```python
counter = 0
café = "open"
π = 3.14159

# Multiple targets
a = b = c = 0
print(a, b, c)
```

```text Output
0 0 0
```

### Tuple unpacking

```python
a, b = 1, 2
print(a, b)

# Star pattern
first, *middle, last = [1, 2, 3, 4, 5]
print(first, middle, last)

# List-form targets unpack the same way
[x, y] = (10, 20)
print(x, y)
```

```text Output
1 2
1 [2, 3, 4] 5
10 20
```

Targets can also be attributes, subscripts, parenthesized lists, or nested patterns.

```python
class Point:
  def __init__(self, x, y):
    self.x, self.y = x, y

p = Point(1, 2)
lst = [0]
lst[0], p.x = p.x, lst[0]
print(lst, p.x)

(a, b) = (3, 4)
h, (i, j) = 5, (6, 7)
print(a, b, h, i, j)
```

```text Output
[1] 0
3 4 5 6 7
```

A starred target such as `*middle` must be a plain name.

### Walrus operator

`:=` assigns as an expression. Useful in conditions and comprehensions.

```python
data = [1, 2, 3]
if (n := len(data)) > 0:
  print(n)
```

```text Output
3
```

## Numbers

Integer literals are decimal by default, with `0x` hex, `0o` octal, and `0b` binary forms. `_` may separate digits. The value range and overflow behavior: [Data types](/language/data-types).

```python
print(0xDEAD_BEEF)
print(0o777)
print(0b1010_1010)
print(1_000_000)
```

```text Output
3735928559
511
170
1000000
```

An underscore must sit between two digits. `1_`, `1__2`, and `0x_1` are rejected.

Float literals are IEEE-754 doubles.

```python
print(3.14)
print(1e-5)
print(.5)
print(1e16)  # repr switches to scientific notation
```

```text Output
3.14
1e-05
0.5
1e+16
```

Complex literals such as `1j` do not exist.

## Strings

Strings take single, double, or triple quotes. An `r` prefix makes a raw string where backslashes stay literal. Adjacent literals concatenate into one. A `b` prefix builds [bytes](/language/data-types) instead of a string.

```python
print('single')
print("double")
print("""triple
quoted""")
print(r'raw\n')  # backslash not escaped
print('hello' ' world')  # implicit concatenation
```

```text Output
single
double
triple
quoted
raw\n
hello world
```

### Escape sequences

Supported escapes are `\n`, `\t`, `\r`, `\a`, `\b`, `\f`, `\v`, `\\`, `\'`, `\"`, `\0`, `\xHH`, `\uHHHH`, `\UHHHHHHHH`, and `\NNN` with 1 to 3 octal digits. An unknown escape keeps its backslash. Named escapes such as `\N{GREEK SMALL LETTER ALPHA}` are not supported and stay literal.

```python
print('\n line break')
print('\t tab')
print('\x41 hex')
print('\u00e9 unicode')
print('\101')  # octal escape, 'A'
```

```text Output

 line break
	 tab
A hex
é unicode
A
```

### f-strings

```python
name = "world"
n = 42
pi = 3.14159
print(f"hello {name}")
print(f"answer is {n + 1}")
print(f"{n:04d}")  # zero-padded width
print(f"{pi:.3f}")  # float precision
print(f"{255:#x}")  # hex with prefix
print(f"{name!r}")  # !r conversion
print(f"{{literal braces}}")
```

```text Output
hello world
answer is 43
0042
3.142
0xff
'world'
{literal braces}
```

The format spec is `[[fill]align][sign][#][0][width][,|_][.precision][type]`. The conversions `!r`, `!s`, and `!a` come before the spec. Type characters are `b c d e E f F g G n o s x X %`. The `,` and `_` options group digits, every three for decimal output and every four for `_` with `b`, `o`, `x`, or `X`.

## Booleans and None

The literals are `True`, `False`, and `None`. `not` negates. Truthiness rules: [Data types](/language/data-types).

```python
print(True, False, None)
print(not True)
```

```text Output
True False None
False
```

## Operators

### Arithmetic

```python
print(7 + 3, 7 - 3, 7 * 3, 7 / 3)
print(7 // 3, 7 % 3, 2 ** 10)
print(-5, +5)
```

```text Output
10 4 21 2.3333333333333335
2 1 1024
-5 5
```

`/` always yields a float. `//` and `%` use floored division, so the result of `%` takes the sign of the divisor. With a string on the left, `%` is printf-style formatting (see [Methods](/reference/methods)).

```python
print("%d of %s" % (3, "pies"))
```

```text Output
3 of pies
```

### Comparison and chaining

Comparisons chain. Ordering works on numbers, strings, bytes, and on lists or tuples compared lexicographically. Mixing types that cannot be ordered raises `TypeError`.

```python
print(1 < 2 < 3)  # chained
print(0 < 5 < 10)
print(1 == 1 == 1)
print([1, 2] < [1, 3])  # lexicographic
```

```text Output
True
True
True
True
```

```python
try:
  1 < "a"
except TypeError:
  print("cannot order")
```

```text Output
cannot order
```

### Logical

`and` and `or` short-circuit and return the deciding operand, not a coerced bool.

```python
print(True and "second")
print(0 or "fallback")
print(None or 0 or [] or "default")
```

```text Output
second
fallback
default
```

### Bitwise

```python
print(5 & 3, 5 | 3, 5 ^ 3, ~5)
print(1 << 4, 32 >> 2)
```

```text Output
1 7 6 -6
16 8
```

### Membership and identity

```python
print(2 in [1, 2, 3])
print(4 not in [1, 2, 3])
print('a' in {'a': 1})
print(None is None)
print(1 is not 2)
```

```text Output
True
True
True
True
True
```

### Augmented assignment

`+= -= *= /= //= %= **= &= |= ^= <<= >>=`

```python
x = 10
x += 5
x *= 2
print(x)
```

```text Output
30
```

### Conditional expression

```python
x = 5
print("big" if x > 3 else "small")
```

```text Output
big
```

## Containers

Literal forms: `[1, 2, 3]` is a list, `(1, 2, 3)` a tuple, `(1,)` a one-element tuple, `()` an empty tuple, `{"a": 1}` a dict, `{1, 2, 3}` a set. `{}` is an empty dict, so the empty set is `set()`. Semantics and methods: [Data types](/language/data-types) and [Methods](/reference/methods).

```python
print((1,), ())
print(type({}), type(set()))
```

```text Output
(1,) ()
<class 'dict'> <class 'set'>
```

### Indexing and slicing

Indices start at 0. Negative indices count back from the end. Slices take `[start:stop:step]`, and any part may be omitted. A negative step walks backwards.

```python
a = [1, 2, 3, 4, 5]
print(a[0], a[-1])  # first and last
print(a[1:4])  # [start:stop]
print(a[:2])
print(a[3:])
print(a[::2])  # every 2nd
print(a[::-1])  # reversed
```

```text Output
1 5
[2, 3, 4]
[1, 2]
[4, 5]
[1, 3, 5]
[5, 4, 3, 2, 1]
```

## Comprehensions

```python
print([x * x for x in range(5)])
print([x for x in range(10) if x % 2 == 0])
print([(i, j) for i in range(2) for j in range(2)])
print([[x for x in range(y)] for y in range(3)])
print({x: x * x for x in range(4)})
print(sorted({x % 3 for x in range(10)}))  # set order is arbitrary, sort to compare
```

```text Output
[0, 1, 4, 9, 16]
[0, 2, 4, 6, 8]
[(0, 0), (0, 1), (1, 0), (1, 1)]
[[], [0], [0, 1]]
{0: 0, 1: 1, 2: 4, 3: 9}
[0, 1, 2]
```

Generator expressions: [Functions](/language/functions).

## Type annotations

Annotations parse on variables, parameters, and return positions. They have no runtime effect. There is no `__annotations__` and no runtime check. Treat them as documentation for humans and static analyzers.

```python
counter: int = 0
name: str = "edge"

def add(a: int, b: int) -> int:
  return a + b

print(add(3, 4))
print(add("a", "b"))  # annotations do not enforce
```

```text Output
7
ab
```
