---
name: edge-python
description: Write, run, test and package Edge Python programs with the edge CLI. Use when editing .py files in an Edge Python project or when the user asks for Edge Python code.
---

# Edge Python

This document is self-verifying and its examples follow the cells v1 grammar. A `python` or `yml` block followed immediately by a `text` block is a runnable cell, and the `skill` crate in this directory executes every cell through the edge CLI and compares it against the `text` block. The tag on the `text` block picks the engine, `Output` runs on both, `Native` on the native engine only, `Web` on the web runtime only, and `Error` expects a failing run whose stderr contains the given text. A `python` block tagged `skip` never runs on any engine and never pairs with a `text` block, and it always says why with one comment at the exact construct that is nondeterministic. A `yml` block tagged `swarm` runs a trusted worker pool through `edge swarm`, while one tagged `untrusted` runs eval groups. Any `python` block without a `text` pair is illustrative only. Verify the whole file from the repository root with `cargo run -p skill -- skill/SKILL.md --engine both`.

Edge Python is a sandboxed Python subset compiled in a single pass to bytecode and executed by a stack VM. It runs in the browser as WebAssembly and in the `edge` CLI as an in-process native engine. There is no bundled stdlib, every module is an external package resolved at compile time. Programs are deterministic, there is no file, network or environment access unless a system module grants it.

Use this skill to write correct Edge Python on the first try. The language looks like Python 3 but is a strict subset, and the differences matter more than the similarities. Read the delta section before writing non-trivial code.

## The working loop

A project is any folder with `.py` files and an optional `packages.json`. The loop is always the same.

1. Write or edit the `.py` files.
2. Run the entry point with `edge run main.py`.
3. Add tests in `*_test.py` files and run `edge test`.
4. Pack a release with `edge build` when the program must run elsewhere.

```bash
edge init myapp        # scaffold main.py, packages.json and index.html
cd myapp
edge run main.py       # execute in the native engine
edge test              # discover and run every *_test.py
edge build             # pack a standalone ./app.edge binary
```

Piping a script works too, which is how the cells of this document run.

```bash
echo 'print(6 * 7)' | edge run
```

When a file path is given, piped stdin instead feeds `input()`, one line per call.

## CLI reference

Bare `edge` prints help and exits 0. `edge -v` prints the version. `Ctrl+C` exits 130. Errors print to stderr and exit 1.

### Global flags

| Flag | Effect |
|---|---|
| `--packages <file>` | Use this manifest instead of `./packages.json` |
| `--web` | Run in headless Chromium instead of the native engine, applies to `run`, `repl` and `test` |

### edge run

`edge run [file]` executes a `.py` script, a packed `.edge` binary or a `.package` bundle, auto-detected by content. With no file it reads the script from stdin. A bare `edge run` in a terminal with no pipe errors. `edge run -c 'print(1)'` runs inline code instead of a file or stdin, and piped stdin then feeds `input()`.

Native-only flags, combining them with `--web` is an error.

| Flag | Effect |
|---|---|
| `--events <f>` | Each line of the file or FIFO feeds one `receive()` call, EOF parks the script |
| `--save-state <f>` | When the script parks on an unservable wait, write a snapshot blob, print `state saved` to stderr and exit 0 |
| `--restore-state <f>` | Boot from a snapshot blob instead of a script and keep running |
| `--preempt <n>` | Yield every `n` loop back-edges so even a tight loop stays snapshottable, 0 disables |

`raise SystemExit(code)` with no argument or an integer exits cleanly with that code. Any other uncaught error prints a traceback and exits 1.

### edge repl

A persistent interpreter across prompts. Imports, definitions and mutations survive between lines, and an input that raises keeps the effects made before the error. One line is one eval, so compound statements go on a single line. Expression results are not auto-printed, use `print()`. Dot commands are `.reset` to wipe state and `.exit` to quit. History lives for the session only.

### edge test

`edge test [path]` discovers `*_test.py` recursively, skipping hidden dirs, `node_modules`, `target` and `dist`. A file argument runs exactly that file. Each file executes in a fresh interpreter and state never leaks between files. Exit code is 0 when everything passes, 1 when a file fails or no tests are found, 2 when the engine cannot start. See the test package section for the API.

### edge init, edge add, edge remove

`edge init [name]` scaffolds `main.py`, `packages.json` and `index.html`, with `--bare` skipping the HTML. `edge add json network` writes manifest entries for known packages, and `edge add foo=<url>` registers a custom URL, a `.wasm` or `.py` URL is treated as a std package and anything else as a system module. `edge remove` deletes entries. Unknown names abort the whole command before any write.

### edge serve

A static dev server with live reload for the current directory. `--host` defaults to `127.0.0.1`, `--port` to 5173, `--open` opens a browser.

### edge build

Three mutually exclusive modes.

| Mode | Default output | Artifact |
|---|---|---|
| `edge build` | `app.edge` | Standalone binary, runs anywhere with nothing installed |
| `edge build --bundle` | `app.package` | Raw bundle for hosts and swarms that already have the runtime |
| `edge build --web` | `dist/` | Browser distribution with vendored runtime and packages |

`--out <path>` overrides the default. The bundle contains every `.py` under the project plus `packages.json`, and the entry is `main.py`, `app.py` or `index.py` when present. An `.edge` binary accepts only the snapshot flags `--save-state`, `--restore-state`, `--preempt` and `--events`.

### edge swarm

`edge swarm <file>` runs a worker pool from a `swarm.yml` manifest. See the workers section for the schema and the two execution models.

### edge uninstall

Interactive removal of the binary, PATH entries and caches.

### Environment variables

| Variable | Effect |
|---|---|
| `EDGE_NO_BROWSER=1` | Installer skips the chrome-headless-shell download |
| `EDGE_CHROME_PATH` | Explicit browser binary for `--web`, highest priority |
| `EDGE_CHROME_DIR` | Browser cache root, defaults to `~/.cache/edge` |
| `EDGE_STD_DIR` | Native engine serves std packages from a local checkout instead of the CDN |
| `EDGE_RUNTIME_DIR` | Serve the web runtime from local disk, used for pre-deploy validation |
| `EDGE_COMPILER_WASM` | Serve compiler.wasm from local disk, used for pre-deploy validation |

## The Python delta

Edge Python parses like Python 3 but deliberately drops parts of the language. This section is the one to internalize, because everything here is valid CPython that fails or behaves differently in Edge Python.

### Not supported at all

- No stdlib. Every module is an external package, so `import os`, `import sys` and `import asyncio` fail at compile time.
- No dynamic code. `exec`, `eval`, `compile` and `__import__` do not exist.
- No `open`. `input()` reads from a host fed buffer with no prompt argument.

```python
open("data.txt")
```

```text Native Error
NameError
```

- No complex numbers. `1j` lexes as `1` followed by the name `j`.
- No metaclasses, descriptors, `__slots__`, `__new__`, `__init_subclass__` or `__set_name__`. Some parse but are never dispatched.
- No augmented assignment dunders. `a += b` desugars to `a = a + b` for user classes, except list `+=` and set `|=`, `&=`, `^=`, `-=` which mutate in place.
- No `bytearray` and no `memoryview`.
- No exception chaining. `raise X from Y` evaluates `Y` but the cause is discarded.
- No `gen.send`, `gen.throw` or `gen.close`. Generators are one-way producers.

### Eager where Python is lazy

Generator expressions lower eagerly to lists. Write `def` plus `yield` when real laziness matters.

```python
g = (i * 2 for i in range(3))
print(g)
```

```text Output
[0, 2, 4]
```

`reversed`, `map`, `filter` and `enumerate` return eager lists, not iterators, and `iter(x)` materializes a snapshot.

```python
print(map(str, [1, 2]))
print(reversed([1, 2, 3]))
```

```text Output
['1', '2']
[3, 2, 1]
```

Dict views are concrete list snapshots taken at call time, not live views.

```python
d = {"a": 1}
keys = d.keys()
d["b"] = 2
print(keys)
```

```text Output
['a']
```

### Numbers are bounded

Integers are 48-bit inline with automatic promotion to 128-bit. Past ±2^127 the run raises `OverflowError`.

```python
print(2**126)
```

```text Output
85070591730234615865843651857942052864
```

```python
print(2**127)
```

```text Native Error
OverflowError
```

`pow(a, b, m)` requires a modulus below 2^63, and the `int_to_bytes` and `int_from_bytes` builtins cap at 8 bytes while the `int.to_bytes` and `int.from_bytes` methods do not.

### Reduced pattern matching

`match` supports literal patterns, captures, the `_` wildcard, OR patterns with `|`, guards with `if`, and flat sequence patterns like `[x, y]` or `[first, *rest]`. Sequence patterns match only list and tuple subjects. There are no nested sequence patterns, no mapping patterns, no class patterns and no `as` captures.

```python
def describe(value):
    match value:
        case 0 | 1:
            return "small"
        case [first, *rest]:
            return f"list of {len(rest) + 1}"
        case n if n < 0:
            return "negative"
        case _:
            return "other"

print(describe([10, 20, 30]))
```

```text Output
list of 3
```

`match` is a soft keyword. A parenthesized subject like `match (a, b):` works as a statement, and `match(a, b)` in expression position still parses as a call.

### Missing pieces by type

- `tuple` and `frozenset` have no methods at all. `(1, 2).count(1)` raises `AttributeError`, use operators or convert to list or set first.
- `str` lacks `translate`, `maketrans`, `format_map`, `isascii`, `isidentifier`, `isnumeric`, `isdecimal` and `isprintable`. `str.format` accepts positional fields only, no `{name}` keyword fields.
- `bytes.split` requires an explicit separator, `bytes.replace` has no count, and codecs are limited to `utf-8` and `ascii` with `strict`, `ignore` and `replace` error handling.
- `zip` has no `strict` flag. `round` uses ties-to-even and always returns int for one argument, float for two.

### Async without asyncio

There is no `asyncio` and no event loop object. The async primitives are top-level builtins, `run`, `gather`, `sleep`, `with_timeout`, `cancel`, `frame` and `receive`. There are no async comprehensions, no async dunders and no background tasks, `create_task` does not exist. See the async section.

### Compile time versus run time

Import failures and syntax errors are compile-time diagnostics and can never be caught with `try`. Everything else raises normal catchable exceptions at run time.

## Imports

Every import resolves at compile time through a host resolver. The compiler flattens each module into the bytecode and the VM fetches nothing at run time.

```python
import math
from json import dumps, loads
from math import sqrt as root
from re import *

print(root(16.0), loads(dumps({"ok": True}))["ok"])
```

```text Output
4.0 True
```

Dotted specs import files. A leading dot anchors at the importing file, one extra dot per directory up. Without it the spec anchors at the nearest `packages.json` dir. The `.py` suffix is implicit.

```python
from .lib.helpers import slugify
from ..shared.util import chunks
from lib.helpers import slugify as sl
```

Not supported. `from . import x` and any form of dynamic import.

Bare names resolve through `packages.json`, walking up from the importing file with the nearest manifest winning. The manifest maps names to paths or URLs under `imports`, JS system modules under `system`, and may `extend` a parent manifest.

```json
{
  "imports": {
    "utils": "./lib/utils.py",
    "mypkg": "https://example.com/mypkg.wasm"
  },
  "system": {
    "charts": "https://example.com/charts.js"
  }
}
```

The names `json`, `re`, `math`, `struct`, `test`, `dom`, `network`, `storage` and `time` resolve with no manifest at all, as official defaults. Modules are singletons with shared mutable state, an import cycle raises `RuntimeError` at startup, and inside an imported module `__name__` is its canonical spec so `if __name__ == "__main__":` blocks are skipped on import. `import_module(name)` looks up a module already bound by a plain `import` in scope.

## Builtins

The global namespace holds exactly 68 builtin functions, the type objects, the exception classes, `NotImplemented`, `__name__` and the async primitives. Nothing else exists, and names like `dir`, `help` or `exit` are simply undefined.

### Output and input

`print(*args, sep=' ', end='\n')` accepts `file` and `flush` and ignores them. `input()` reads one host fed line with no prompt.

```python
print("a", "b", sep="-", end="!\n")
```

```text Output
a-b!
```

### Numeric

`abs`, `round`, `min`, `max`, `sum`, `pow`, `divmod`, `bin`, `oct`, `hex`. `round` breaks ties to even. `min` and `max` accept variadic args or one iterable plus `key` and `default`.

```python
print(round(2.5), round(3.5), round(1.55, 1))
print(divmod(7, 2), max("xy", "abcde", key=len))
print(pow(2, 10, 100))
```

```text Output
2 4 1.6
(3, 1) abcde
24
```

### Conversion

`int`, `float`, `str`, `bool`, `list`, `tuple`, `set`, `frozenset`, `dict`, `bytes`, `chr`, `ord`. `int` truncates toward zero and parses bases 2 to 36 or 0 for auto-detect. `int("nan")` style failures raise `ValueError` and `int(float("inf"))` raises `OverflowError`.

```python
print(int("ff", 16), int("0b101", 0), int(-3.7))
print(float("inf") > 1e308, ord("A"), chr(97))
```

```text Output
255 5 -3
True 65 a
```

### Iteration

`len`, `range`, `sorted`, `reversed`, `enumerate`, `zip`, `iter`, `next`, `map`, `filter`, `all`, `any`, `slice`. `range` is genuinely lazy and everything else eager, see the delta section.

```python
print(list(enumerate("ab", start=1)))
print(zip([1, 2, 3], "ab"))
print(sorted([3, 1, 2], reverse=True), any([0, "", 3]))
```

```text Output
[(1, 'a'), (2, 'b')]
[(1, 'a'), (2, 'b')]
[3, 2, 1] True
```

### Types and attributes

`type`, `object`, `isinstance`, `issubclass`, `callable`, `id`, `hash`, `repr`, `format`, `getattr`, `hasattr`, `setattr`, `delattr`, `vars`, `globals`, `locals`, `import_module`, `super`, `property`, `staticmethod`, `classmethod`. `isinstance` accepts a tuple of types and `bool` is a subclass of `int`. `vars(x)` returns a snapshot of instance attributes, and `globals()` and `locals()` return copies whose mutation binds nothing.

```python
print(isinstance(True, int), callable(len))
print(format(255, "08x"), repr("it's"))
```

```text Output
True True
000000ff "it's"
```

### Bytes helpers

`bytes_fromhex`, `int_from_bytes(b, order)` and `int_to_bytes(n, length, order)` with a limit of 8 bytes and unsigned values.

```python
print(bytes_fromhex("ff00"), int_from_bytes(b"\x01\x00", "little"))
```

```text Output
b'\xff\x00' 1
```

### Exceptions

The catchable tree under `Exception` is `ArithmeticError` with `OverflowError` and `ZeroDivisionError`, `LookupError` with `IndexError` and `KeyError`, `RuntimeError` with `RecursionError` and `NotImplementedError`, plus `ValueError`, `TypeError`, `AttributeError`, `NameError`, `OSError`, `StopIteration`, `StopAsyncIteration`, `AssertionError`, `MemoryError` and `TimeoutError`. Under `BaseException` sit `SystemExit` and `CancelledError`, which `except Exception` does not catch.

Handlers name one class, a tuple or nothing, and a bare `except` must come last. `except X as e` binds the exception and `e.args` is its argument tuple. `finally` runs on every exit path including `return`, `break` and `continue`.

```python
try:
    {}["missing"]
except (KeyError, IndexError) as e:
    print(type(e).__name__, e.args)
finally:
    print("always")
```

```text Output
KeyError ('missing',)
always
```

User exception classes support inheritance among themselves for `except` matching but do not join the builtin tree.

## Type methods

Methods live on the builtin types. `tuple`, `frozenset`, `bool` and `NoneType` have none.

### str

`encode`, `upper`, `lower`, `strip`, `lstrip`, `rstrip`, `capitalize`, `title`, `casefold`, `swapcase`, `isdigit`, `isalpha`, `isalnum`, `isspace`, `isupper`, `islower`, `istitle`, `startswith`, `endswith`, `find`, `rfind`, `index`, `rindex`, `count`, `split`, `rsplit`, `join`, `replace`, `removeprefix`, `removesuffix`, `splitlines`, `partition`, `rpartition`, `center`, `ljust`, `rjust`, `zfill`, `expandtabs`, `format`. Indices count code points. `startswith` and `endswith` accept a tuple of prefixes. `format` takes positional fields with specs, never keyword fields.

```python
print(" hello ".strip(), "a,b,c".split(",", 1))
print("-".join(["x", "y"]), "Hello".casefold(), "{0}{1}{0}".format("a", "b"))
print("file.py".removesuffix(".py"), "5".zfill(3))
```

```text Output
hello ['a', 'b,c']
x-y hello aba
file 005
```

### list

`append`, `extend`, `insert`, `remove`, `pop`, `clear`, `copy`, `reverse`, `index`, `count`, `sort` with `key` and `reverse`. Slice assignment resizes, and `+=` extends in place.

```python
xs = ["bb", "a", "ccc"]
xs.sort(key=len)
xs[1:1] = ["z"]
print(xs, xs.pop())
```

```text Output
['a', 'z', 'bb'] ccc
```

### dict

Insertion ordered. `keys`, `values`, `items` return list snapshots, plus `get`, `update`, `pop`, `popitem`, `setdefault`, `fromkeys`, `copy`, `clear`. `popitem` removes the most recently inserted pair. Numerically equal keys collapse, so `1`, `1.0` and `True` are one key.

```python
d = dict.fromkeys(["a", "b"], 0)
d.update({"c": 1})
print(d.pop("a"), d.setdefault("d", 4), list(d))
print({1: "x", True: "y"})
```

```text Output
0 4 ['b', 'c', 'd']
{1: 'y'}
```

### set

`add`, `remove`, `discard`, `pop`, `clear`, `update`, `copy`, `union`, `intersection`, `difference`, `symmetric_difference`, `intersection_update`, `difference_update`, `symmetric_difference_update`, `issubset`, `issuperset`, `isdisjoint`. Named methods accept any iterable while the operators require sets on both sides. Iteration order is hash based, never rely on it and print through `sorted`.

```python
print(sorted({1, 2, 3} & {2, 3, 4}))
print({1, 2}.issubset({1, 2, 3}), {1}.isdisjoint({2}))
```

```text Output
[2, 3]
True True
```

### int and float

`int` has `bit_length`, `bit_count`, `to_bytes` and the classmethod `from_bytes`. `float` has `is_integer`.

```python
print((255).bit_length(), (5).bit_count(), (258).to_bytes(2).hex())
print((4.0).is_integer(), int.from_bytes(b"\x01\x02", "big"))
```

```text Output
8 2 0102
True 258
```

### bytes

`decode`, `encode` via str, `hex`, `fromhex`, `startswith`, `endswith`, `find`, `index`, `count`, `replace`, `split`, `lower`, `upper`, `strip`, `lstrip`, `rstrip`, `join`. Case methods are ASCII only.

```python
print(b"\x00\x01".hex(), b"a,b".split(b","), b"ABC".lower())
```

```text Output
0001 [b'a', b'b'] b'abc'
```

## Functions and classes

Functions support defaults, keyword arguments, `*args`, `**kwargs` and bare-`*` keyword-only parameters, plus call-site unpacking. The positional-only marker `/` parses but is not enforced, so never rely on it. Lambdas are single expressions. Decorators work on functions and classes, stacked bottom-up. Closures capture variables by reference, see the gotchas section.

```python
def greet(name, *, punct="!"):
    return f"hi {name}{punct}"

print(greet("edge", punct="?"))
```

```text Output
hi edge?
```

Classes support single and multiple inheritance with C3 linearization, zero-argument `super()`, `property` with setters, `staticmethod` and `classmethod`, and class decorators. There is no two-argument `super()` form. Dunders are looked up on the class, assigning one on an instance has no effect.

The supported dunders are `__init__`, `__call__`, `__repr__`, `__str__`, `__format__`, `__bool__`, `__len__`, `__hash__`, `__iter__`, `__next__`, `__getitem__`, `__setitem__`, `__delitem__`, `__contains__`, `__getattr__`, `__enter__`, `__exit__`, `__index__`, `__int__`, `__float__`, `__abs__`, the arithmetic and bitwise operators with their reflected forms, and the six comparisons. Returning `NotImplemented` from an arithmetic dunder triggers the reflected fallback.

```python
class Vector:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def __add__(self, other):
        return Vector(self.x + other.x, self.y + other.y)

    def __repr__(self):
        return f"Vector({self.x}, {self.y})"

class Scaled(Vector):
    pass

print(Scaled(1, 2) + Vector(10, 20))
```

```text Output
Vector(11, 22)
```

Context managers implement `__enter__` and `__exit__(exc_type, exc_value, traceback)` where the traceback argument is always `None`. A truthy `__exit__` suppresses the exception.

Pure functions are memoized automatically after two identical calls. The VM detects purity by the absence of I/O, mutation, raising and free-name reads, so naive recursive code is fast and side-effecting calls skip the cache safely.

## Async

The module body runs as an implicit coroutine, so top-level code can call suspending functions directly. A plain `def` called from a coroutine can also call them.

```python
async def dbl(n):
    await sleep(0)
    return n * 2

print(gather(dbl(1), dbl(2), dbl(3)))
```

```text Output
[2, 4, 6]
```

The primitives are builtins, no import needed.

| Builtin | Behavior |
|---|---|
| `run(*coros)` | Drives coroutines to completion, returns the first argument's result |
| `gather(*coros)` | Runs coroutines concurrently, returns the list of results in order, the first error re-raises |
| `sleep(s)` | Suspends for `s` seconds, `sleep(0)` yields once, negatives clamp to 0 |
| `with_timeout(s, coro)` | Runs the coroutine and raises `TimeoutError` when it overruns |
| `cancel(coro)` | Delivers `CancelledError` at the next tick, uncatchable, runs `finally` |
| `frame()` | Suspends until the next browser render frame |
| `receive()` | Parks until a host event or swarm message arrives |

```python
async def slow():
    await sleep(10)
    return "done"

try:
    with_timeout(0.01, slow())
except TimeoutError:
    print("timed out")
```

```text Output
timed out
```

Scheduling is cooperative. A tight loop without a suspending call cannot be cancelled or preempted unless the engine runs with `--preempt`. There is no `create_task` and no preemption between coroutines.

### Snapshots

The native engine can serialize the full interpreter state, heap, globals, suspended coroutines and scheduler, and restore it later. This is how long-running or event-driven programs survive process restarts.

```bash
edge run app.py --save-state state.bin     # writes the blob when the script parks
edge run --restore-state state.bin         # resumes from the blob
edge run app.py --preempt 500              # makes even while-True snapshottable
```

A snapshot is taken when the script parks on a wait the engine cannot serve, for example `receive()` with no events left. Without `--save-state` such a park is an error. The blob embeds a bytecode fingerprint and only restores into the same program. Feed a resumed run with `--events file`, one `receive()` line per call.

## Std packages

Five official packages import by bare name with no manifest, on both engines. `edge add <name>` writes the manifest entry explicitly when a project should pin it.

### json

`loads(s)` with optional `object_hook`, `object_pairs_hook`, `parse_float`, `parse_int` and `parse_constant`. `dumps(obj)` with `indent`, `sort_keys`, `ensure_ascii`, `check_circular`, `allow_nan`, `skipkeys`, `default`, `separators` and `cls`. Parse failures raise `ValueError`, non-serializable values raise `TypeError` unless `default` handles them. Integers round-trip at 128-bit and non-finite floats map to `NaN` and `Infinity`.

```python
import json

data = json.loads('{"n": 21, "xs": [1, 2]}')
print(json.dumps(data, sort_keys=True))
print(json.dumps({"bad": object()}, default=str))
```

```text Output
{"n":21,"xs":[1,2]}
{"bad":"<object instance>"}
```

### math

Constants `pi`, `e`, `tau`, `inf`, `nan`. Functions `sqrt`, `cbrt`, `exp`, `exp2`, `expm1`, `pow`, `log`, `log2`, `log10`, `log1p`, the trig and hyperbolic families, `atan2`, `hypot`, `dist`, `degrees`, `radians`, `erf`, `erfc`, `gamma`, `lgamma`, `fabs`, `fmod`, `remainder`, `copysign`, `ldexp`, `modf`, `frexp`, `floor`, `ceil`, `trunc`, `isnan`, `isinf`, `isfinite`, `fsum`, `prod`, and the integer functions `factorial`, `gcd`, `lcm`, `isqrt`, `comb`, `perm` at 128-bit. Domain errors raise `ValueError`. A batch family (`sqrt_all`, `add_all`, `dot_all`, `matvec` and friends) operates on `bytes` buffers of little-endian f64, pair it with `struct.pack`.

```python
import math

print(math.gcd(12, 18), math.factorial(10), math.isqrt(17))
print(math.floor(math.pi), math.isfinite(math.inf))
```

```text Output
6 3628800 4
3 False
```

### re

Backtracking engine with a step budget that raises `RuntimeError` against catastrophic backtracking. Module functions take `(pattern, string)` and `compile(pattern)` returns a pattern object with the same operations as methods. There are no Match objects, matchers return the matched string or `None`.

| Function | Returns |
|---|---|
| `match`, `search`, `fullmatch` | Matched string or `None` |
| `findall` | List of strings, grouped matches become lists |
| `groups` | List of captures or `None` |
| `span` | `[start, end]` codepoint offsets or `None` |
| `sub` | Substituted string, `\\1` and `\\g<name>` expand groups |

Supported syntax covers classes, anchors, quantifiers with lazy forms, capturing, non-capturing and named groups, backreferences, alternation, lookahead and fixed-width lookbehind, plus inline flags `(?i)`, `(?s)` and `(?m)`. Not supported, `\p{...}`, atomic groups, possessive quantifiers, conditionals and scoped flags.

```python
import re

print(re.findall(r"\d+", "a1b22"))
print(re.sub(r"(\w+)@(\w+)", r"\2@\1", "user@host"))
print(re.groups(r"(\d+)-(\d+)", "12-34"))
```

```text Output
['1', '22']
host@user
['12', '34']
```

### struct

`pack(fmt, *values)` returns `bytes`, `unpack(fmt, data)` returns a list, `calcsize(fmt)` returns an int. Codes are `x b B ? h H i I q Q f d` with repeat counts, prefixes `<` (default), `=`, `>` and `!`. String codes `s` and `p`, half-float `e`, native-size `n` and `N`, and the `pack_into` family are not implemented.

```python
import struct

buf = struct.pack("<hh", 258, -1)
print(buf.hex(), struct.unpack("<hh", buf), struct.calcsize("<hh"))
```

```text Output
0201ffff [258, -1] 4
```

### test

The test framework, imported by bare name and driven by `edge test` discovery. Test files do not need to call `run()` themselves, the runner evaluates the file and then invokes the driver, so a file that registers no tests fails.

- `@fixture` registers a factory under its function name, built fresh per test.
- `@test("description", *uses)` registers a test and injects named fixtures by keyword.
- `with raises(ExcType):` asserts the block raises, accepts a class or a tuple.
- Assertions are plain `assert`.
- `run()` executes everything registered, prints verdicts and raises `SystemExit(0)` or `SystemExit(1)`.

```python
from test import fixture, test, raises, run

@fixture
def numbers():
    return [1, 2, 3]

@test("sum adds up", "numbers")
def total(numbers):
    assert sum(numbers) == 6

@test("division by zero raises")
def div():
    with raises(ZeroDivisionError):
        1 / 0

run()
```

```text Output
PASS - sum adds up
PASS - division by zero raises
2 passed, 0 failed
```

## System modules

Four system libraries plus the swarm module. Availability differs by engine, and importing a web-only module natively is a compile-time error telling you to rerun with `--web`.

| Module | Native CLI | Web runtime |
|---|---|---|
| `time` | Built into the binary, always UTC | System JS module, IANA timezone |
| `network` | Built into the binary, no CORS | System JS module, CORS applies |
| `storage` | Not available | System JS module |
| `dom` | Not available | System JS module |
| `swarm` | Built into the binary, see workers | Not available |

### time

`time()`, `time_ns()`, `monotonic()`, `monotonic_ns()`, `perf_counter()`, `perf_counter_ns()`, and a suspending `sleep(secs)`. `gmtime` and `localtime` return a JSON string of the nine struct_time fields, decode it with `json.loads`. `mktime`, `strftime`, `strptime`, `asctime` and `ctime` convert between forms. `timezone()`, `altzone()`, `daylight()` and `tzname()` are calls. The native engine has no timezone database, `tzname()` is always `"UTC"` and `localtime` equals `gmtime`.

```python
from time import tzname, time

print(tzname())
print(type(time()).__name__)
```

```text Native
UTC
float
```

### network

`fetch(url[, options_json])` returns a JSON string with `id`, `ok`, `status`, `headers` and `body`. `fetch_text` and `fetch_json` return the body directly and raise on non-2xx responses. All three suspend until the response arrives. `abort_request(id)` cancels an in-flight request and exists only on the web runtime. WebSockets use `ws_open`, `ws_send`, `ws_close` and `ws_state`, SSE uses `sse_open`, `sse_close` and `sse_state`, and both stream events through `receive()` as JSON payloads with a `type` field.

```python
from network import fetch_json

data = fetch_json("https://api.example.com/items")
items = json.loads(data)
```

### storage

Web only. Synchronous key-value access through `local_get`, `local_set`, `local_remove`, `local_clear`, `local_keys` and the `session_*` twins, values are strings so encode structured data with `json.dumps`. IndexedDB through suspending calls, `idb_open`, `idb_put`, `idb_get`, `idb_delete`, `idb_keys` and `idb_close`.

```python
import storage

storage.local_set("k", "v")
print(storage.local_get("k"))
```

```text Web
v
```

### dom

Web only. Handles are opaque ints, multi-result queries return CSV strings of handles, structured results return JSON strings, and async results arrive through `receive()`. The surface covers selection and traversal (`query`, `query_all`, `closest`, `parent`, `children`, siblings), creation and mutation (`create_element`, `append_child`, `insert_before`, `remove`, `replace_children`, `clone_node`), content and attributes (`get_text`, `set_text`, `get_html`, `set_html`, `get_attribute`, `set_attribute`, class and data helpers), style and layout (`set_style`, `rect`, `scroll_top`, `focus`), events (`bind_event`, `unbind_event`, `dispatch_event`, `click`), forms and files, observers, animations, media and platform dialogs. A `batch()` context manager buffers the mutating calls and applies them with one host call on exit.

```python
import dom

print(dom.tag_name(dom.body()))
```

```text Web
body
```

## Workers

`edge swarm` runs many isolated programs as cooperative workers over a few threads, share-nothing with message passing. There are two execution models and the manifest chooses per group.

```yaml
runtime:
  listen: tcp://127.0.0.1:7777   # optional, its presence makes the swarm a live server
  durable: tmp/swarm/log         # WAL path, replays unprocessed messages on restart
  schedulers: auto               # or a fixed number
  max_nodes: 1000000

groups:
  worker:
    run: app                     # script path or project directory
    replicas: 100                # ceiling, workers spawn on demand
    retry: 2                     # re-deliveries before a message is dropped dead
    seed: ["first message"]      # delivered before the pool starts
    out: stdout                  # stdout, null or file://path
    limits:                      # per-worker sandbox overrides
      heap: 65536
      preempt: 500
```

Each group picks exactly one of `run`, `code` or `eval: true`. Without `listen:` the pool runs until every message is processed and exits.

### The trusted model

Workers keep state between messages, pick work up with the `receive()` builtin and forward with `send` from the `swarm` module, strings only, never blocking.

```yml swarm
groups:
  worker:
    code: |
      msg = receive()
      print(f"got {msg}")
    replicas: 4
    seed: ["hello"]
```

```text Native
got hello
```

### The untrusted model

`eval: true` groups compile each message as its own program in a fresh interpreter. No state survives between messages and the `swarm` module is unavailable, so untrusted code cannot reach the pool's plumbing.

```yml untrusted
groups:
  guest:
    eval: true
    seed: ["print(6 * 7)"]
```

```text Native
42
```

With `listen:` the swarm accepts one `<group> <body>` line per TCP message and exposes an HTTP control endpoint, `GET /stats`, `POST /pub/<group>` and `POST /eval/<group>` for eval groups. Untrusted bundles arrive as base64 `.package` payloads behind an `EDGEPKG:` marker and run materialized in an isolated temp dir.

## Semantics that surprise Python programmers

Closures capture loop variables by reference. Bind the value with a default argument when building functions in a loop.

```python
fns = [(lambda i=i: i) for i in range(3)]
print([f() for f in fns])
```

```text Output
[0, 1, 2]
```

Inline integers compare by value under `is`, so `is` on numbers does not mean identity. Reserve `is` for `None` and sentinels.

```python
a = 1000
b = 1000
print(a is b)
```

```text Output
True
```

List `+=` and set `|=`, `&=`, `^=`, `-=` mutate in place and aliases see the change. Every other augmented assignment rebinds.

```python
a = [1]
b = a
a += [2]
print(b)
```

```text Output
[1, 2]
```

Set iteration and repr order is hash based. Always present sets through `sorted`.

```python skip
s = {"a", "b", "c"}  # skip: set iteration order is hash based
print(s)
```

`id()` reuses heap slots and varies between runs. Never print it in examples or tests.

```python skip
print(id(object()))  # skip: heap slots are reused, the value changes between runs
```

Truthiness follows Python, the falsy set is `None`, `False`, `0`, `0.0`, `""`, `b""`, `[]`, `()`, `{}`, `set()`, `frozenset()` and `range(0)`. `bool` subclasses `int` so `True + True == 2`. User objects in builtin dicts and sets compare by identity even when they define `__hash__` and `__eq__`, so look them up by the same reference. `len` on strings counts code points. Same source and input give the same output on every run, `id()` aside.

## Sandbox limits

Programs run under a fixed budget. Exceeding one raises the matching exception, catchable except the op-limit `RuntimeError` whose handler re-raises on its first operation because the budget is still exhausted.

| Limit | Value | Raised |
|---|---|---|
| Call depth | 256 frames | `RecursionError` |
| Operations | 100 million | `RuntimeError` |
| Live objects | 100 thousand | `MemoryError` |
| Source size | 10 MiB | Compile error |
| Expression nesting | 200 | Compile error |
| Indentation depth | 100 | Compile error |
| Instructions per chunk | 65535 | Compile error |
| `repr` output | 1M chars | Truncated |
