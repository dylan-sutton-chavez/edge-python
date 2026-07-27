---
title: "Control flow"
description: "Conditionals, loops, exceptions, pattern matching."
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

The `else` runs if the loop completes without `break`.

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

Iterates anything that produces a sequence: list, tuple, dict, set, range, string, generator.

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

Runs when the loop exhausts its iterator (no `break`).

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

Subset supported: literal patterns, capture variables, `_` wildcard, OR (`|`), guards (`if`), sequence patterns with `*rest`.

Sequence-pattern items must be literals (`int` / `float` / `str` / `True` / `False` / `None`), capture names, or `_`. Nested sequences (`case [[a, b], c]:`), mapping patterns (`{"key": x}`), class patterns (`Point(x=0)`), and `as` captures are unsupported. Use chained `if` / `elif` instead. A `case [...]` pattern matches only a `list` or `tuple` subject; any other value (including `str` / `bytes`) just fails it and falls through, so scalar and sequence cases mix freely in one `match`.

Scalars: literals, OR-patterns, capture-with-guard, wildcard.

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

Sequences: fixed-length patterns, a guard, and a `*rest` capture.

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

```python
# Multiple handlers and finally
try:
  x = int("abc")
except ValueError:
  x = -1
finally:
  print("cleanup")
print(x)
```

```text Output
cleanup
-1
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

A bare `raise` inside an `except` re-raises the exception currently being handled.

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

`raise X from Y` raises `X`. The `from` clause parses and the cause evaluates, but `__cause__` / `__context__` aren't preserved. Only `X` reaches the handler.

```python
try:
  raise ValueError from KeyError
except ValueError:
  print("caught the ValueError")
```

```text Output
caught the ValueError
```

Handlers match on class and declared parents. `except Exception` catches `ValueError`, `RuntimeError`, `KeyError`, etc:

```python
try:
  raise RuntimeError("boom")
except Exception:
  print("subclass caught")
```

```text Output
subclass caught
```

### Exception names available

Pre-bound exception classes (with their parent links so `except <Parent>:` matches subclasses) are listed in [Limits and errors, Runtime](/reference/limits-and-errors#runtime).

## with

`with` drives the context-manager protocol:

1. Evaluate the expression.
2. Call `__enter__`.
3. Bind the result to `as`.

On exit, `__exit__` runs; a truthy return suppresses the exception, a falsy one propagates it. See [Dunders](/language/dunders#context-managers) for the `__exit__` signature and exception info.

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

### Cleanup on early exit

`finally` and `__exit__` run on *every* way out of the block — not only normal completion and exceptions, but also `return`, `break`, and `continue`. A `return` in a `try` runs the `finally` before the value leaves; a `break` out of a `with` still calls `__exit__`. A `return` or `break` in the `finally` itself replaces the original exit.

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

A failed assertion raises `AssertionError`. Catch it with `except AssertionError`, `except Exception`, or bare `except`. The optional message after the comma is evaluated only when the assertion fails and becomes the exception's argument (`e.args`).

## del

Removes a binding from the slot. Works on plain names, attributes (`del obj.attr`), indexed positions (`del seq[i]`), and parenthesized target groups (`del (a, b)`).

```python
x = 42
del x
try:
  print(x)
except NameError:
  print("gone")
```

```text Output
gone
```
