---
title: "Limits and errors"
description: "Sandbox limits, integer width, error types, and runtime guarantees."
---

## Sandboxed execution

Every execution is metered, there is no unmetered mode. Both shipped engines, the browser runtime and the CLI's native engine, run every script under these limits, and `edge actor` groups can override them per field.

| Limit | Value | What hitting it raises |
|---|---|---|
| Max call depth | 256 | `RecursionError` |
| Max operations | 100,000,000 | `RuntimeError` |
| Max live objects | 100,000 | `MemoryError` |

The op-limit `RuntimeError` cannot be caught. The `except` handler is entered but its first operation re-raises because the budget is still exhausted.

```python
def loop(n):
  return loop(n + 1)

try:
  loop(0)
except RecursionError:
  print("hit max depth")
```

```text Output
hit max depth
```

## Integer width

Integers are two-tier:

- **Inline (fast).** 48-bit signed, packed into the NaN-boxed value. Range `-140_737_488_355_328` to `140_737_488_355_327` (`-2^47` to `2^47 - 1`). One ALU op per arithmetic, no allocation.
- **Wide (slow).** 128-bit, heap-allocated. Used automatically when a literal exceeds the inline range or inline arithmetic overflows.

Promotion is automatic and invisible. Past ±2^127, arithmetic raises `OverflowError`. Integers are not unbounded: wider than 128 bits is out of scope by design.

```python
print(140737488355327)   # inline, fast path
print(2 ** 47)           # auto-promotes to the wide path
print(2 ** 100)
try:
  print(2 ** 127)        # past the 128-bit cap
except OverflowError:
  print("overflow")
```

```text Output
140737488355327
140737488355328
1267650600228229401496703205376
overflow
```

`pow(a, b, m)` with a modulus is supported, but the modulus must be at most `2^63`. Larger values raise `ValueError`, because the intermediate multiply would overflow 128 bits.

## Source and token limits

Source must be under 10 MiB. Larger input is rejected at lex time. The remaining caps prevent asymmetric inputs, small sources that would produce huge parse trees or instruction streams:

| Limit | Value | Diagnostic |
|---|---|---|
| Source size | 10 MiB | `source file exceeds maximum size (10 MiB)` |
| Indent depth | 100 | `indentation depth exceeds maximum (100)` |
| F-string nesting depth | 200 | `f-string nesting depth exceeds maximum (200)` |
| Expression nesting depth | 200 | `expression too deeply nested` |
| Instructions, names, or constants per chunk | 65,535 | `program too large: exceeded the 65535 instruction, name, or constant limit` |
| Call arguments | 255 positional, 255 keyword | `too many arguments in call (max 255 positional and 255 keyword)` |
| Native imports per module | 256 | `too many native imports (max 256 per module)` |

Separately, `repr` output is truncated with a trailing `, ...` past 1,000,000 characters, so printing a huge structure cannot exhaust memory.

## Compile-time errors

Syntax and resolution errors are reported as diagnostics with byte offsets into the source, rendered with line, column, and a caret preview. They are caught before any code runs and cannot be caught by `try` / `except`.

| Diagnostic | Cause |
|---|---|
| `expected X, got 'Y'` | Unexpected token |
| `'(' was never closed` (or `'['` / `'{'`) | Bracket opened with no matching closer |
| `')' does not match '[', expected ']'` | Wrong closer kind for the innermost opener |
| `unexpected ')', no matching opener` | Closer with no opener on the stack |
| `unterminated string literal` | String missing its closing quote |
| `unterminated triple-quoted string literal` | Triple-quoted string hit EOF |
| `f-string was never closed` | F-string body hit EOF before its close |
| `inconsistent indentation: mixing tabs and spaces` | Indent mixes both whitespace kinds |
| `unindent does not match any outer indentation level` | Dedent lands between two outer levels |
| `integer literal too large to represent (max ±2^127)` | Literal past the 128-bit cap |
| `'break' outside loop` / `'continue' outside loop` | Misplaced control keyword |
| `default 'except:' must be last` | Bare `except` not at the end |
| `expression too deeply nested` | Past the expression depth cap |
| `program too large: exceeded the 65535 instruction, name, or constant limit` | Past the chunk cap |

Import failures are compile-time diagnostics too, including modules Edge Python does not ship (`os`, `sys`, `asyncio`). See [Modules](/reference/modules#resolution-errors) for those message formats.

## Runtime errors

Runtime errors raise as typed exceptions, catchable with `try` / `except`.

| Class | When |
|---|---|
| `TypeError` | Wrong operand or argument type |
| `ValueError` | Right type, invalid value |
| `AttributeError` | Attribute not found on the object |
| `NameError` | Undefined name |
| `ZeroDivisionError` | Division or modulo by zero |
| `OverflowError` | Integer arithmetic past ±2^127 |
| `KeyError` | Dict or set lookup miss |
| `IndexError` | Sequence index out of range |
| `StopIteration` | Iterator exhausted |
| `AssertionError` | Failed `assert` |
| `TimeoutError` | `with_timeout` deadline expired |
| `CancelledError` | Coroutine cancelled by `cancel()` |
| `SystemExit` | `raise SystemExit(code)`. Uncaught, the host exits with that code |
| `RecursionError` | Past the call-depth limit |
| `MemoryError` | Past the live-object limit |
| `RuntimeError` | Past the op limit, an import cycle, `input()` without host data, or an internal invariant |

Every entry in the table fires from ordinary code:

```python
def show(f):
  try:
    f()
  except Exception as e:
    print(type(e).__name__ + ":", e)

show(lambda: 1 + "x")
show(lambda: int("abc"))
show(lambda: {}["missing"])
show(lambda: [1][5])
show(lambda: nope)
show(lambda: 1 % 0)

def fail():
  assert 1 == 2, "math broke"

show(fail)
```

```text Output
TypeError: unsupported operand type(s) for +: 'int' and 'str'
ValueError: int(): invalid literal
KeyError: 'missing'
IndexError: list index out of range
NameError: name 'nope' is not defined
ZeroDivisionError: division by zero
AssertionError: math broke
```

`SystemExit` needs its own `except` clause, and `TimeoutError` fires when a `with_timeout` deadline expires:

```python
try:
  raise SystemExit(3)
except SystemExit:
  print("caught exit")

async def slow():
  sleep(1)

try:
  run(with_timeout(0.01, slow()))
except TimeoutError:
  print("timed out")
```

```text Output
caught exit
timed out
```

A user `raise X` re-raises whatever class or instance `X` is. Raising a value that does not derive from `BaseException` (a `str`, an `int`) gives `TypeError`.

### Exception hierarchy

`except` walks parent links in a curated tree rooted at `BaseException`:

- `Exception` sits under `BaseException`. Everything catchable in normal code derives from `Exception`.
- `LookupError` groups `IndexError` and `KeyError`. `ArithmeticError` groups `OverflowError` and `ZeroDivisionError`. `RuntimeError` parents `RecursionError` and `NotImplementedError`.
- `OSError`, `NameError`, `StopIteration`, `StopAsyncIteration`, `AssertionError`, `MemoryError`, and `TimeoutError` sit directly under `Exception`.
- `SystemExit` and `CancelledError` sit directly under `BaseException`, so `except Exception` does not catch them. Use their own name or a bare `except`.

```python
try:
  raise RuntimeError("oops")
except Exception as e:
  print("caught via parent:", e)

try:
  [][0]
except Exception:
  print("caught IndexError as Exception")
```

```text Output
caught via parent: oops
caught IndexError as Exception
```

User-defined classes do not join the built-in tree, but they support inheritance among themselves: `except UserBase` catches a raised `UserSub` when `UserSub` inherits from `UserBase`. For `raise X from Y` and chaining, see [Control flow](/language/control-flow).

### Exception arguments

Caught exceptions expose their constructor arguments as `e.args`, a tuple. `raise X("msg")` and `raise X(a, b)` carry through, runtime-raised errors carry their message as a single arg, and a bare `raise X` produces an empty tuple.

```python
try:
  raise TypeError("bad input")
except TypeError as e:
  print(e.args)

try:
  1 / 0
except ZeroDivisionError as e:
  print(e.args)

try:
  raise ValueError
except ValueError as e:
  print(e.args)
```

```text Output
('bad input',)
('division by zero',)
()
```

### Catching errors

```python
def safe(f, x):
  try:
    return f(x)
  except TypeError:
    return "type"
  except ValueError:
    return "value"
  except ZeroDivisionError:
    return "zero"
  except:
    return "other"

print(safe(lambda x: 1 / x, 0))
print(safe(lambda x: int(x), "abc"))
print(safe(lambda x: len(x), 42))
```

```text Output
zero
value
type
```

### Environmental errors

Failures that happen before the source reaches the compiler surface as plain text, uncatchable from script code, with no line or column to anchor to:

| Error | When |
|---|---|
| `input rejected: invalid utf-8 at byte N` | Host input bytes are not valid UTF-8 |
| `source file exceeds maximum size (10 MiB)` | Source over the lex-time cap |

Handle these at the embedder layer (path validation, encoding, size check) before invoking the compiler.

## Behavioral notes

A few supported operations have implementation-defined or by-design behavior worth knowing:

- **Set iteration and `repr` order.** Sets store elements in a hash table, so iteration and `repr` follow hash order, not insertion order. Do not rely on it. `{3, 1, 2}` may `repr` as `{3, 2, 1}`.
- **`is` on numbers.** Inline integers are compared by value, so `a = 1000; b = 1000; a is b` is `True`. Use `==` for value equality and reserve `is` for `None` and identity checks.
- **`str.casefold`.** Simple lowercasing without full Unicode case-fold expansion: `'ß'.casefold()` stays `'ß'` rather than expanding to `'ss'`.

The `is` note, demonstrated:

```python
a = 1000
b = 1000
print(a is b)
print(a == b)
```

```text Output
True
True
```

## Determinism

Same source plus same input gives the same output across runs and architectures (`x86_64`, `aarch64`, `wasm32`). There is no time, randomness, threading, or OS interaction in the core language. Heap-slot reuse is the only nondeterminism, and it is observable through `id(x)` only, never through `==`, `repr`, or any other operation.
