---
title: "Syntax"
description: "Operators, literals, and language surface."
---

## Comments

```python
# Single-line comment
x = 1 # Trailing comment

"""
Triple-quoted strings used as
module-level documentation are
parsed but discarded.
"""
```

The same applies to `def` and `class` docstrings: parsed for compatibility, then discarded — there is no `__doc__` at runtime.

## Identifiers and assignment

Identifiers follow Python rules: letters, digits, underscores, plus any non-ASCII letter.

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

Targets can also be attributes, subscripts, parenthesized lists, or nested:

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

Starred targets (`first, *rest = ...`) must be plain names.

### Walrus operator

Assignment as expression. Useful in conditions and comprehensions.

```python
data = [1, 2, 3]
if (n := len(data)) > 0:
  print(n)
```

```text Output
3
```

## Numbers

Integer literals: hex (`0x`), octal (`0o`), binary (`0b`). `_` separates digits. Range and promotion: [Data types, Integer](/language/data-types#integer).

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

Underscores are validated: `1_`, `1__2`, `0x_1`, `1e_5` -> `SyntaxError`. Must sit between two digits.

```python
# Floats: IEEE-754 doubles
print(3.14)
print(1e-5)
print(.5)
print(1e16) # repr switches to scientific notation

# Mixed arithmetic coerces to float
print(2 + 3.0)
```

```text Output
3.14
1e-05
0.5
1e+16
5.0
```

Complex literals (`1j`, `2+3j`) are not part of the language.

## Strings

```python
print('single')
print("double")
print("""triple
quoted""")
print(r'raw\n') # backslash not escaped
print('hello' ' world') # implicit concatenation
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

Supported: `\n`, `\t`, `\r`, `\a`, `\b`, `\f`, `\v`, `\\`, `\'`, `\"`, `\0`, `\xHH`, `\uHHHH`, `\UHHHHHHHH`, `\NNN` (1–3 octal digits). Named-char escapes (`\N{GREEK SMALL LETTER ALPHA}`) are not supported. Use `\u`.

```python
print('\n line break')
print('\t tab')
print('\x41 hex')
print('\u00e9 unicode')
print('\101') # octal escape, 'A'
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
print(f"{n:04d}") # zero-padded width
print(f"{pi:.3f}") # float precision
print(f"{255:#x}") # hex with prefix
print(f"{name!r}") # !r conversion
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

Full format mini-language: `[[fill]align][sign][#][0][width][,|_][.precision][type]`, with `!r` / `!s` / `!a` conversions before the spec. Type chars: `b o c d e E f F g G n s x X %`. The `,` and `_` group digits (every three for decimals/floats; `_` groups `b`/`o`/`x`/`X` every four).

f-string literals process [escape sequences](#escape-sequences) like any string — `\r`, for instance, rewinds the cursor to the line start so a single output line can redraw in place.

## Booleans and None

The literals are `True`, `False`, and `None`; `not` negates. Truthiness rules and the full falsy table: see [Data types](/language/data-types#truthy-and-falsy).

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

`/` always yields a float; `//` and `%` follow floored division (the result of `%` takes the divisor's sign). With a string left operand, `%` is printf-style formatting instead of modulo (see [Formatting](/reference/methods#formatting)).

### Comparison and chaining

Ordering comparisons (`<`, `>`, `<=`, `>=`) work on numbers, strings, bytes, and tuples/lists (compared lexicographically). Mixing un-orderable types raises `TypeError`.

```python
print(1 < 2 < 3) # chained
print(0 < 5 < 10)
print(1 == 1 == 1)
print([1, 2] < [1, 3]) # lexicographic
```

```text Output
True
True
True
True
```

### Logical

Short-circuiting `and` / `or` return the operand, not a coerced bool:

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
print('a' in {'a': 1})
print(None is None)
print(1 is not 2)
```

```text Output
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

Literals: `[1, 2, 3]` (list), `(1, 2, 3)` / `(1,)` / `()` (tuple), `{"a": 1}` (dict), `{1, 2, 3}` (set, empty is `set()` since `{}` is a dict). See [Data types](/language/data-types) for semantics, mutation, methods.

### Slicing

```python
a = [1, 2, 3, 4, 5]
print(a[1:4]) # [start:stop]
print(a[:2])
print(a[3:])
print(a[::2]) # every 2nd
print(a[::-1]) # reversed
```

```text Output
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
print(sorted({x % 3 for x in range(10)})) # set order is implementation-defined; sort to compare
```

```text Output
[0, 1, 4, 9, 16]
[0, 2, 4, 6, 8]
[(0, 0), (0, 1), (1, 0), (1, 1)]
[[], [0], [0, 1]]
{0: 0, 1: 1, 2: 4, 3: 9}
[0, 1, 2]
```

Generator expressions: see [Functions](/language/functions#generator-expressions).

## Type annotations

Annotations parse on variables, parameters, and return positions but have no runtime effect. The parser drains them; they never reach the VM. No `__annotations__`, no runtime check. Treat them as docs for humans and static analysers.

```python
counter: int = 0
name: str = "edge"

def add(a: int, b: int) -> int:
  return a + b

print(add(3, 4))
print(add("a", "b")) # annotations don't enforce: int+str logic decides
```

```text Output
7
ab
```
