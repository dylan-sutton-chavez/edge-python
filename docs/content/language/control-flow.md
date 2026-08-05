---
title: "Control flow"
description: "Conditionals, loops, pattern matching, and exceptions."
---

## if / elif / else

```python
def classify(n):
  if n < 0:
    return "negative"
  elif n == 0:
    return "zero"
  else:
    return "positive"

for x in [-3, 0, 7]:
  print(classify(x))
```

```text Output
negative
zero
positive
```

## pass

`pass` does nothing. Use it where the grammar requires a block.

```python
for i in range(3):
  pass
print("done")
```

```text Output
done
```

## while

```python
n, total = 5, 0
while n > 0:
  total += n
  n -= 1
print(total)
```

```text Output
15
```

### while ... else

The `else` runs when the loop ends without `break`.

```python
x = 0
while x < 3:
  x += 1
else:
  print("loop finished cleanly")
```

```text Output
loop finished cleanly
```

## for

`for` iterates any iterable: list, tuple, dict, set, range, string, or generator.

```python
for ch in "abc":
  print(ch)
```

```text Output
a
b
c
```

```python
# Tuple unpacking in the loop variable
pairs = [("a", 1), ("b", 2), ("c", 3)]
for key, value in pairs:
  print(key, value)
```

```text Output
a 1
b 2
c 3
```

```python
# Star pattern works too
for first, *rest in [[1, 2, 3], [4, 5, 6, 7]]:
  print(first, rest)
```

```text Output
1 [2, 3]
4 [5, 6, 7]
```

### break and continue

`break` exits the loop. `continue` skips to the next item. Both work in `for` and `while`.

```python
for i in range(10):
  if i == 5:
    break
  if i % 2 == 0:
    continue
  print(i)
```

```text Output
1
3
```

### for ... else

The `else` runs when the loop exhausts its iterator without `break`.

```python
for i in range(3):
  pass
else:
  print("done")
```

```text Output
done
```

## match / case

Supported patterns: literals (`int`, `float`, `str`, `True`, `False`, `None`, and negative numbers), capture names, the `_` wildcard, OR patterns with `|`, guards with `if`, and sequence patterns with an optional `*rest`.

Items in a sequence pattern must be literals, capture names, or `_`. Nested sequence patterns, mapping patterns, class patterns, and `as` captures do not parse. Use chained `if` and `elif` for those. A sequence pattern matches only list and tuple subjects. Any other value, including `str` and `bytes`, fails the pattern and falls through to the next case.

```python
def sign(n):
  match n:
    case 0:
      return 'zero'
    case 1 | 2 | 3:
      return 'small'
    case x if x < 0:
      return 'negative'
    case _:
      return 'other'

print(sign(0), sign(2), sign(-7), sign(99))
```

```text Output
zero small negative other
```

```python
def shape(seq):
  match seq:
    case []:
      return 'empty'
    case [x]:
      return f'single {x}'
    case [x, y] if x == y:
      return 'pair-equal'
    case [first, *rest]:
      return f'{first} then {len(rest)} more'

print(shape([]))
print(shape([5]))
print(shape([3, 3]))
print(shape([1, 2, 3, 4]))
```

```text Output
empty
single 5
pair-equal
1 then 3 more
```

## try / except / else / finally

```python
def safe_div(a, b):
  try:
    return a / b
  except ZeroDivisionError:
    return None

print(safe_div(10, 2))
print(safe_div(10, 0))
```

```text Output
5.0
None
```

A handler names one exception, a tuple of exceptions, or nothing. A bare `except` catches everything. `else` runs when the `try` body raised nothing. `finally` always runs.

```python
try:
  x = int("42")
except (ValueError, TypeError):
  x = -1
else:
  print("parsed", x)
finally:
  print("cleanup")
```

```text Output
parsed 42
cleanup
```

```python
# Bare except catches everything
try:
  raise "boom"
except:
  print("caught")
```

```text Output
caught
```

Handlers match subclasses. `except Exception` catches `ValueError`, `RuntimeError`, `KeyError`, and the rest of the hierarchy listed in [Limits and errors](/reference/limits-and-errors).

```python
try:
  raise RuntimeError("boom")
except Exception:
  print("subclass caught")
```

```text Output
subclass caught
```

### raise

```python
def positive(n):
  if n < 0:
    raise ValueError
  return n

try:
  positive(-1)
except ValueError:
  print("rejected")
```

```text Output
rejected
```

A bare `raise` inside an `except` block re-raises the exception being handled.

```python
def attempt():
  try:
    raise ValueError("bad")
  except ValueError:
    print("logging")
    raise

try:
  attempt()
except ValueError as e:
  print("outer", e.args[0])
```

```text Output
logging
outer bad
```

`raise X from Y` raises `X`. The `from` clause parses and `Y` evaluates, but the cause is not preserved. There is no `__cause__` or `__context__`. Only `X` reaches the handler.

```python
try:
  raise ValueError from KeyError
except ValueError:
  print("caught the ValueError")
```

```text Output
caught the ValueError
```

## with

`with` runs the context manager protocol:

1. Evaluate the expression.
2. Call `__enter__` and bind its result to the `as` target.
3. On exit, call `__exit__(exc_type, exc_value, traceback)`.

On a clean exit the three arguments are `None`. On an exception they carry the exception info. A truthy return from `__exit__` suppresses the exception.

```python
class Resource:
  def __enter__(self):
    print("acquire")
    return "handle"
  def __exit__(self, *exc):
    print("release")
    return False

with Resource() as r:
  print(r)
print("after")
```

```text Output
acquire
handle
release
after
```

Multiple targets:

```python
class Tag:
  def __init__(self, name):
    self.name = name
  def __enter__(self):
    return self.name
  def __exit__(self, *exc):
    return False

with Tag("first") as x, Tag("second") as y:
  print(x, y)
```

```text Output
first second
```

```python
class Suppress:
  def __enter__(self):
    return self
  def __exit__(self, *exc):
    return True

with Suppress():
  raise ValueError("gone")
print("suppressed")
```

```text Output
suppressed
```

### Cleanup on early exit

`finally` and `__exit__` run on every way out of a block. That includes normal completion, exceptions, `return`, `break`, and `continue`. A `return` or `break` inside `finally` replaces the original exit.

```python
class Lock:
  def __enter__(self):
    return self
  def __exit__(self, *exc):
    print("released")

def take(n):
  with Lock():
    if n < 0:
      return "negative"
    return n * 2

print(take(5))
print(take(-1))

for i in range(3):
  try:
    if i == 1:
      break
  finally:
    print("tick", i)
```

```text Output
released
10
released
negative
tick 0
tick 1
```

## assert

```python
def reciprocal(n):
  assert n != 0, "n must be non-zero"
  return 1 / n

print(reciprocal(4))
```

```text Output
0.25
```

A failed assertion raises `AssertionError`. The message after the comma evaluates only on failure and becomes the exception argument (`e.args`).

```python
try:
  assert 1 == 2, "math broke"
except AssertionError as e:
  print(e.args[0])
```

```text Output
math broke
```

## del

`del` removes a binding. It works on plain names, attributes (`del obj.attr`), indexed positions (`del seq[i]`), and parenthesized groups (`del (a, b)`).

```python
x = 42
del x
try:
  print(x)
except NameError:
  print("gone")

xs = [1, 2, 3]
del xs[1]
print(xs)
```

```text Output
gone
[1, 3]
```
