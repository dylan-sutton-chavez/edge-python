---
title: "Limits and errors"
description: "Sandbox limits, error types, and runtime guarantees."
---

## Sandbox limits

Two profiles via `VM::with_limits`. The same `compiler.wasm` runs unsandboxed in trusted contexts, clamped in untrusted ones.

| Limit | `none()` (default) | `sandbox()` | What hitting it raises |
|----------------|--------------------|---------------|------------------------|
| Max call depth | 1,000 | 256 | `RecursionError` |
| Max operations | unbounded | 100,000,000 | `RuntimeError` |
| Max live objects | 10,000,000 | 100,000 | `MemoryError` |

## Integer width

Two-tier:

* **Inline (fast)**: 47-bit signed in a NaN-boxed `Val`. Range `-2^47 .. 2^47-1` (`-140_737_488_355_328 .. 140_737_488_355_327`). One ALU op per arithmetic, no allocation.
* **Wide (slow)**: i128 in `HeapObj::LongInt`. Range `+/-2^127 - 1`. Auto-used when a literal exceeds 47-bit or inline arithmetic overflows.

Outside `+/-2^127` raises `OverflowError`. Promotion is automatic. User code doesn't see the boundary.

```python
print(140737488355327) # inline, fast path
print(2 ** 47) # 140737488355328: auto-promotes to LongInt
print(2 ** 100) # 1267650600228229401496703205376
try:
  print(2 ** 127) # past the i128 cap
except OverflowError:
  print("overflow")
```

```text Output
140737488355327
140737488355328
1267650600228229401496703205376
overflow
```

### Caveats

- **`pow(a, b, m)` modular**: modulus must be `<= 2^63` (larger overflows i128 in the multiply). Hard cap without arbitrary-precision arithmetic.
- **No CPython-style unbounded ints**: by design, edge workloads don't need wider than 128 bits. Crypto-scale math is out of scope.

### Triggering limits

```python
# Recursion depth
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

```python
# Heap quota, a tight loop retaining new objects
try:
  xs = []
  while True:
    xs.append([])
except MemoryError:
  print("hit heap limit")
```

## Source size

Source must be under 10 MiB. Larger input is rejected at lex time.

## Token limits

| Limit | Value |
|----------------------|-------|
| Max indent depth | 100 |
| Max f-string depth | 200 |
| Max expression depth | 200 |
| Max instructions per chunk | 65,535 |

Prevent asymmetric DoS: small input producing an exponentially large parse tree.

## Error types

### Compile-time

Reported as `Diagnostic { start, end, msg }`, byte offsets into source. Line and column are computed lazily by `render()`. Caught before any code runs.

| Diagnostic | Cause |
|-------------------------------------------|----------------------------------------|
| `expected X, got 'Y'` | Unexpected token |
| `'(' was never closed` (or `'['` / `'{'`) | Bracket opened with no matching closer |
| `')' does not match '[', expected ']'` | Wrong closer kind for innermost opener |
| `unexpected ')', no matching opener` | Closer with no opener on the stack |
| `unexpected ':' (missing 'if', 'while', 'for', ...)` | `expr:` at statement level |
| `unterminated string literal` | String missing closing quote |
| `unterminated triple-quoted string literal` | Triple-quoted string hit EOF |
| `f-string was never closed` | F-string body hit EOF before close |
| `inconsistent indentation: mixing tabs and spaces` | Indent mixes both whitespace kinds |
| `'break' outside loop` | Misplaced control keyword |
| `'continue' outside loop` | Misplaced control keyword |
| `default 'except:' must be last` | Bare `except` not at end |
| `expression too deeply nested` | Past `MAX_EXPR_DEPTH` |
| `program too large: exceeded maximum instruction limit` | Past `MAX_INSTRUCTIONS` |

### Runtime

Raised as `VmErr`. Most are catchable with `try` / `except`.

| Variant | Class name | When |
|-----------------|----------------------|------------------------------------|
| `Type` | `TypeError` | Wrong operand type |
| `TypeMsg` | `TypeError` | Wrong operand type (with context) |
| `Value` | `ValueError` | Right type, invalid value |
| `Attribute` | `AttributeError` | Attribute not found on object |
| `Name` | `NameError` | Undefined name |
| `ZeroDiv` | `ZeroDivisionError` | Division or modulo by zero |
| `Overflow` | `OverflowError` | Integer arithmetic past ±2^127 |
| `Raised("KeyError")` | `KeyError` | Dict / set lookup miss |
| `Raised("IndexError")` | `IndexError` | Sequence index out of range |
| `Raised("StopIteration")` | `StopIteration` | Iterator exhausted |
| `Raised("AssertionError")` | `AssertionError` | Failed `assert` |
| `Raised("TimeoutError")` | `TimeoutError` | `with_timeout` deadline expired |
| `Raised("CancelledError")` | `CancelledError` | User-thrown cancellation |
| `Raised("SystemExit")` | `SystemExit` | `raise SystemExit(code)`; uncaught = clean host exit with that code |
| `CallDepth` | `RecursionError` | Past `max_calls` (deep recursion or call depth) |
| `Heap` | `MemoryError` | Past heap limit |
| `Budget` | `RuntimeError` | Past op limit |
| `Runtime` | `RuntimeError` | Internal invariant or unsupported |
| `Raised` | (custom) | User `raise X` (X is a class or instance; raising a non-exception value such as a `str` or `int` gives `TypeError`) |

#### Exception hierarchy

Curated tree rooted at `BaseException -> Exception`. `except` walks parent links. `except Exception` catches `RuntimeError`, `ValueError`, `KeyError`, `AssertionError`, etc. `except RuntimeError` catches `RecursionError`, `NotImplementedError`. Intermediate groups match too: `except LookupError` catches `IndexError` / `KeyError`, and `except ArithmeticError` catches `OverflowError` / `ZeroDivisionError`; `OSError` also lives under `Exception`. `SystemExit` sits directly under `BaseException`, so `except Exception` does not catch it (use `except SystemExit` or a bare `except`).

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

User-defined classes don't auto-extend the built-in `BaseException` tree. They support inheritance among themselves: `except UserBase` catches a raised `UserSub` when `UserSub` inherits from `UserBase`. For `raise X from Y` and chaining, see [Control flow](/language/control-flow#raise).

### Exception arguments

Caught exceptions expose constructor args as `e.args` (tuple):

- `raise X("msg")` and `raise X(a, b)` carry through.
- Runtime-raised errors carry their message as a single arg.
- Bare `raise X` produces an empty tuple.

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

Failures surfaced before the source reaches the compiler. No line/column preview. No parsed code to anchor to. Emitted as plain text, uncatchable from Python.

| Error | When | Resolution |
|---------------------------------------------|-----------------------------------------------|---------------------------------------|
| `input rejected: invalid utf-8 at byte N` | Host input bytes not valid UTF-8 | Re-encode as UTF-8 |
| `source file exceeds maximum size (10 MiB)` | Source over the 10 MiB lex-time cap | Split or trim the input |

Handle at the embedder layer (path validation, encoding, size check) before invoking the compiler.

## Unavailable modules

Unavailable modules (`os`, `sys`, `asyncio`, …) parse for syntactic compatibility but have no resolver entry. So they are rejected at **compile time** before any code runs. This is a parse-time diagnostic, not a catchable runtime exception. See [Imports, Errors](/reference/imports#errors). For code reuse, use higher-order functions.

## Behavioral notes

A few supported operations have implementation-defined or by-design behavior worth knowing:

- **Set iteration / `repr` order**: sets store elements in a Rust hash table (`FxHash`), so iteration and `repr` follow hash order, not insertion order. It is implementation-defined — don't rely on it. `{3, 1, 2}` may `repr` as `{3, 2, 1}`.
- **`is` on numbers**: small/inline integers are compared by value (NaN-box), so `a = 1000; b = 1000; a is b` is `True`. Use `==` for value equality and reserve `is` for `None` and identity checks.
- **`str.casefold`**: simple lowercasing, without full Unicode case-fold expansion — `'ß'.casefold()` stays `'ß'` rather than expanding to `'ss'`.

## Determinism

Same source + input -> same output across runs and architectures (`x86_64`, `aarch64`, `wasm32`). No time, randomness, threading, or OS interaction. Heap-pool slot reuse is the only nondeterminism: observable through `id(x)` only, never `==`, `repr`, or any other operation.
