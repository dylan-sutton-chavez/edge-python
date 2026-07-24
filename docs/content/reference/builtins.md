---
title: "Built-in functions"
description: "Every built-in function in Edge Python with examples and outputs."
---

66 built-in functions, all first-class values: pass as arguments, store in containers, alias.

```python
# All built-ins are real values
fns = [abs, hex, str]
print([f(-3) for f in fns])

p = print
p("aliased")
```

```text Output
[3, '-0x3', '-3']
aliased
```

Edge Python is multi-paradigm. Introspection helpers (`eval`, `exec`, `compile`, `dir`, `ascii`, `help`, `__import__`, `breakpoint`, `open`) are absent by design. The static-import contract and the lack of a writable global module table make them impossible or inconsistent with the paradigm. `super`, `property`, `staticmethod`, and `classmethod` are supported. See [`/language/classes`](/language/classes), [`/language/dunders`](/language/dunders).

## Output

### print

`print(*args, sep=' ', end='\n')`: values joined by `sep` to stdout, then `end`. `*` unpacking spreads an iterable into the arguments. `file` / `flush` are accepted and ignored (the sandbox has one output stream).

```python
print(1, 2, 3)
print("a", "b", "c", sep="-")
print("no newline", end="")
print("!")
print(*[1, 2, 3], sep=", ")
```

```text Output
1 2 3
a-b-c
no newline!
1, 2, 3
```

### input

`input()`: one line from the host buffer. Native: stdin. WASM: drains the buffer the host wrote via `set_input`. Empty buffer -> empty string. No prompt argument.

## Numeric

### abs

`abs(x)`: absolute value of int or float. Non-numeric -> `TypeError`. Works across the full integer range (see [Integer width](/reference/limits-and-errors#integer-width)).

```python
print(abs(-7))
print(abs(3.14))
```

```text Output
7
3.14
```

### round

`round(x)` or `round(x, n)`: banker's rounding (ties go to even).

```python
print(round(2.5))
print(round(0.5))
print(round(-1.5))
print(round(1.55, 1))
```

```text Output
2
0
-2
1.6
```

### min, max

Variadic or single iterable. Accept a `default=` returned when a single iterable is empty; without it an empty input raises `ValueError`. A `key=` function selects the comparison value (the original element is returned). Ordering follows `<`: numbers, strings, bytes, and tuples/lists (lexicographic).

```python
print(min(3, 1, 4))
print(max([3, 1, 4]))
print(min("hello"))
print(min([], default=-1))
print(max([], default=0))
```

```text Output
1
4
e
-1
0
```

### sum

`sum(iterable)` or `sum(iterable, start)`. `sum([])` returns `0`.

```python
print(sum([1, 2, 3]))
print(sum([1, 2, 3], 100))
print(sum(x * x for x in range(5)))
```

```text Output
6
106
30
```

### pow

`pow(base, exp)` or `pow(base, exp, mod)` for modular exp. 3-arg requires int operands and non-negative exp (`pow(a, b, 0)` -> `ZeroDivisionError`; `pow(a, -1, m)` -> `ValueError`). Modulus must be `<= 2^63` (larger overflows i128 in `(result * base) % m`), raises `ValueError("pow() modulus too large; must be < 2^63")`.

```python
print(pow(2, 10))
print(pow(2, 10, 1000))
print(pow(7, 13, 19))
```

```text Output
1024
24
7
```

### divmod

`divmod(a, b)`: `(a // b, a % b)` as a tuple. Ints or floats (float operands give a float quotient and remainder).

```python
print(divmod(7, 3))
print(divmod(-7, 3))
print(divmod(7.5, 2))
```

```text Output
(2, 1)
(-3, 2)
(3.0, 1.5)
```

### bin, oct, hex

Format an integer as a base-2, base-8, or base-16 string with prefix.

```python
print(bin(10))
print(oct(8))
print(hex(255))
print(hex(-256))
```

```text Output
0b1010
0o10
0xff
-0x100
```

## Type conversion

### int

`int(x)`: accepts `int`, `bool`, `float` (truncates toward zero), or a numeric string (with optional `_` separators). `int(s, base)` parses a string in radix `2`-`36`, or `0` to auto-detect a `0x` / `0o` / `0b` prefix. Bad strings -> `ValueError`; `int(inf)` / `int(nan)` -> `OverflowError` / `ValueError`. Supports +/-2^127 (inline 47-bit + `LongInt` i128); wider -> `OverflowError`.

```python
print(int(3.9))
print(int("42"))
print(int(True))
print(int("ff", 16))
print(int("0b101", 2))
print(int("1_000"))
```

```text Output
3
42
1
255
5
1000
```

### float

`float(x)`: `int`, `bool`, `float`, or string. Strings recognise `inf`, `-inf`, `nan` (case-insensitive).

```python
print(float(2))
print(float("3.14"))
print(float("inf"))
```

```text Output
2.0
3.14
inf
```

### str

`str(x)`: display form. No arg -> empty string. `str(bytes, encoding)` decodes the bytes.

```python
print(str(42))
print(str([1, 2, 3]))
print(str(None))
print(str(b"hi", "utf-8"))
```

```text Output
42
[1, 2, 3]
None
hi
```

### bool

```python
print(bool(0), bool(1))
print(bool([]), bool([0]))
print(bool(""), bool("x"))
```

```text Output
False True
False True
False True
```

### list, tuple, set, frozenset, dict

`list`, `tuple`, `set`, `frozenset` accept any iterable: list, tuple, set, frozenset, dict (keys), range, bytes, str, generator, coroutine. Share an `extract_iter` helper, so all constructors are interchangeable. With no argument, each builds an empty one.

```python
print(list("abc"))
print(tuple(range(3)))
print(set({"a": 1, "b": 2})) # iterates dict keys
print(frozenset(b"\x01\x02\x03"))
print(dict(a=1, b=2))
```

```text Output
['a', 'b', 'c']
(0, 1, 2)
{'b', 'a'}
frozenset({1, 2, 3})
{'a': 1, 'b': 2}
```

`dict` also accepts a mapping, kwargs, or an iterable of key/value pairs (`dict([('a', 1)])`); each pair element must have length 2.

### chr, ord

Convert between code points and single-char strings. `chr` accepts full Unicode (`chr(0x1F600)` -> `"😀"`); negative -> `ValueError`. `ord` requires a length-1 string; `ord(b'A')` not accepted.

```python
print(chr(65))
print(ord("A"))
print(chr(0x1F600))
```

```text Output
A
65
😀
```

## Sequence

### len

Element count for `str` (code points), `bytes`, `list`, `tuple`, `dict`, `set`, `frozenset`, `range`. Else `TypeError`.

```python
print(len("hello"))
print(len([1, 2, 3, 4]))
print(len({"a": 1, "b": 2}))
print(len(range(100)))
```

```text Output
5
4
2
100
```

### range

`range(stop)`, `range(start, stop)`, `range(start, stop, step)`. Lazy. `step=0` -> `ValueError`; non-int args -> `TypeError`. Two ranges compare equal when they produce the same value sequence: `range(0) == range(1, 1)` is `True`, `range(5) == range(0, 5, 1)` is `True`.

```python
print(list(range(5)))
print(list(range(2, 8)))
print(list(range(10, 0, -2)))
print(range(0, 6, 2) == range(0, 5, 2))
```

```text Output
[0, 1, 2, 3, 4]
[2, 3, 4, 5, 6, 7]
[10, 8, 6, 4, 2]
True
```

### sorted

New sorted list. Accepts `key=fn` and `reverse=True/False`. Numbers, strings, bytes, and tuples/lists order lexicographically; objects defining `__lt__` sort by it; mixed un-orderable types raise `TypeError`.

```python
print(sorted([3, 1, 4, 1, 5]))
print(sorted("hello"))
print(sorted([3, 1, 4, 1, 5], reverse=True))
print(sorted(["banana", "apple", "kiwi"], key=len))
```

```text Output
[1, 1, 3, 4, 5]
['e', 'h', 'l', 'l', 'o']
[5, 4, 3, 1, 1]
['kiwi', 'apple', 'banana']
```

### reversed

Returns a list (eager, not a lazy iterator). For strings: list of length-1 strings. Operationally equivalent to a lazy iterator for finite inputs.

```python
print(reversed([1, 2, 3]))
print(reversed("abc"))
```

```text Output
[3, 2, 1]
['c', 'b', 'a']
```

### enumerate

Pairs each element with its index -> list of `(i, value)` tuples. A second argument (positional or `start=`) sets the first index.

```python
for i, v in enumerate(["a", "b", "c"]):
    print(i, v)
```

```text Output
0 a
1 b
2 c
```

### zip

Pairs N iterables, truncating to shortest. No `strict=`, pre-validate lengths if needed.

```python
for a, b in zip([1, 2, 3], ["x", "y", "z"]):
    print(a, b)

print(list(zip([1, 2], [3, 4], [5, 6])))
```

```text Output
1 x
2 y
3 z
[(1, 3, 5), (2, 4, 6)]
```

### next

`next(iterator)` -> next item. Exhausted -> `StopIteration`. Two-arg `next(it, default)` returns `default` instead of raising on exhaustion.

```python
it = iter([10, 20, 30])
print(next(it))
print(next(it))
print(next(it))
```

```text Output
10
20
30
```

### iter

`iter(x)` returns a fresh iterator over any iterable (list, tuple, set, dict, range, str, bytes, frozenset). Materialises a snapshot, original never mutated. Two-arg `iter(callable, sentinel)` calls `callable()` until it returns `sentinel`, eagerly.

```python
it = iter([1, 2, 3])
print(next(it))
print(next(it))

# Strings yield characters
chars = iter("abc")
print(next(chars))
```

```text Output
1
2
a
```

### map

`map(fn, *iterables)` -> list of `fn(items...)`. With several iterables they are walked in parallel, stopping at the shortest. Eager, full list materialises immediately; pipelines into `sum`, `list`, `max`.

```python
print(list(map(lambda x: x * 2, [1, 2, 3])))
print(sum(map(lambda x: x * x, range(5))))
print(list(map(lambda a, b: a + b, [1, 2], [10, 20])))

def normalize(s):
    return s.strip().lower()

print(list(map(normalize, ["  Hi ", "WORLD"])))
```

```text Output
[2, 4, 6]
30
[11, 22]
['hi', 'world']
```

### filter

`filter(pred, iterable)` -> list of items where `pred(item)` is truthy. `None` predicate filters by truthiness (equivalent to `lambda x: x`).

```python
print(list(filter(lambda x: x > 2, [1, 2, 3, 4])))

# `None` keeps any truthy value
print(list(filter(None, [0, 1, "", "hi", [], [1]])))
```

```text Output
[3, 4]
[1, 'hi', [1]]
```

### import_module

`import_module(name)` returns a module previously imported statically. Runtime dispatch among pre-imported modules without a manual dict.

```python
import prod_handler
import dev_handler

def handle(env, request):
  return import_module(env + "_handler").handle(request)

handle("prod", req)
handle("dev",  req)
```

Candidates must be imported statically somewhere. `import_module` is a runtime *lookup*, not a *fetch*. It preserves lockfile and integrity: every reachable module is verified at compile time. Unknown name -> `NameError`. Non-module global -> `TypeError`.

Dynamic loading (`importlib.import_module`, `__import__`) doesn't exist by design. Static-import plus runtime-dispatch replaces it.

### bytes

Four forms:

- `bytes()` -> empty
- `bytes(n)` -> `n` zero bytes
- `bytes(iterable)` of ints in `0..=255` -> those values
- `bytes(s, encoding)` -> encoded (`"utf-8"`, `"utf8"`, `"ascii"` only; else `ValueError`)

```python
print(bytes())
print(bytes(4))
print(bytes([72, 101, 108, 108, 111]))
print(bytes("café", "utf-8"))
```

```text Output
b''
b'\x00\x00\x00\x00'
b'Hello'
b'caf\xc3\xa9'
```

See [Bytes](/language/data-types#bytes) for literal syntax (`b"..."`), indexing, slicing, methods.

### bytes_fromhex, int_from_bytes, int_to_bytes

Free-function forms. The equivalent [methods](/reference/methods#int-and-float-methods) (`bytes.fromhex`, `int.from_bytes`, `int.to_bytes`) also exist and behave identically; use whichever reads better.

- `bytes_fromhex(s)`: hex string -> bytes. Inner whitespace ignored; non-hex -> `ValueError`.
- `int_from_bytes(b, order)`: `order` is `"big"` or `"little"`. Unsigned (high bit never sign).
- `int_to_bytes(n, length, order)`: `n >= 0`, `length <= 8`. Accepts inline ints or `LongInt`; doesn't fit -> `OverflowError`.

```python
print(bytes_fromhex("48656c6c6f"))
print(int_from_bytes(b"\x01\x00", "big"))
print(int_to_bytes(255, 2, "big"))
```

```text Output
b'Hello'
256
b'\x00\xff'
```

## Logical reductions

### all, any

Both short-circuit: a generator argument stops at the deciding element.

```python
print(all([1, 2, 3]))
print(all([1, 0, 3]))
print(all([])) # vacuous truth

print(any([0, 0, 1]))
print(any([0, 0, 0]))
print(any([]))
```

```text Output
True
False
True
True
False
False
```

## Type and identity

### type

`type(x)` returns the type object for `x`, shown as `<class 'name'>`. Built-in type names (`int`, `set`, `list`, ...) are these same objects. So `type(x) is int` and `type(x) == int` hold, and calling one constructs it (`type([1])([2, 3])` gives `[2, 3]`). For a user instance, `type(x)` is its own class, so `type(x) is C` holds and its repr is qualified with the module (`<class '__main__.C'>`). No metaclass or `dir`.

```python
print(type(42))
print(type("hi"))
print(type([1, 2]))
print(type(print))
print(type(42) is int)
print(type([1, 2, 3])([4, 5]))

class C:
  pass
print(type(C()) is C)
```

```text Output
<class 'int'>
<class 'str'>
<class 'list'>
<class 'builtin_function_or_method'>
True
[4, 5]
True
```

Functions, type objects, and user classes expose `__name__` (the bare declared name). `type(e)` on an exception instance reports its concrete class, so `type(e).__name__` yields the exception's name.

```python
def greet():
  pass

class Box:
  pass

print(greet.__name__)
print(int.__name__)
print(Box.__name__)
try:
  1 / 0
except Exception as e:
  print(type(e).__name__)
```

```text Output
greet
int
Box
ZeroDivisionError
```

### object

`object()` builds a unique featureless instance — the classic sentinel idiom. `type(x) is object` holds for it, and every value is an instance of `object`. A lone `object` base in a class statement is a no-op (`class C(object)` equals `class C`).

```python
SENTINEL = object()

print(SENTINEL is object())
print(type(SENTINEL) is object)
print(isinstance(SENTINEL, object))
print(isinstance(42, object))

class C(object):
  pass
print(isinstance(C(), object))
```

```text Output
False
True
True
True
True
```

### isinstance

`isinstance(obj, X)`: `X` is a built-in type, exception class, user-defined `Class`, or tuple of any of those. String `X` (`isinstance(x, "str")`) -> `TypeError`. `bool` is a subtype of `int`. Exception classes walk the standard hierarchy (`isinstance(e, Exception)` matches any built-in exception). User classes walk the inheritance chain. `object` matches every value.

```python
print(isinstance(42, int))
print(isinstance(True, int)) # bool is a subtype of int
print(isinstance("x", (int, str))) # tuple of types
```

```text Output
True
True
True
```

### issubclass

`issubclass(C, B)`: both `C` and `B` are classes (`B` may be a tuple of classes). Arg 1 must itself be a class, or it raises `TypeError`. Built-in and exception classes walk the standard hierarchy (`issubclass(ZeroDivisionError, Exception)`), `bool` is a subclass of `int`, and user classes walk the inheritance chain.

```python
print(issubclass(ZeroDivisionError, Exception))
print(issubclass(bool, int))
print(issubclass(KeyError, (ValueError, Exception)))

class A:
  pass

class B(A):
  pass

print(issubclass(B, A))
print(issubclass(A, B))
```

```text Output
True
True
True
True
False
```

### callable

True for user functions, lambdas, bound methods, type objects, native builtins, and instances whose class defines `__call__` (see [Callable](/language/dunders#callable)). False for everything else.

```python
print(callable(print))
print(callable(lambda x: x))
print(callable(42))
print(callable("hello"))
```

```text Output
True
True
False
False
```

### id, hash

`id(x)`: stable identifier (NaN-box bit pattern masked to int range). `hash(x)`: hash for hashable values. `hash(1) == hash(1.0)`, so int/float keys collapse to one dict slot.

```python
x = 42
print(id(x) == id(x))
print(hash("hello") == hash("hello"))
print(hash((1, 2, 3)) == hash((1, 2, 3)))
```

```text Output
True
True
True
```

```python
# Lists, dicts, sets are unhashable
try:
    hash([1, 2, 3])
except TypeError:
    print("unhashable")
```

```text Output
unhashable
```

Mutable containers used as dict keys / set members -> `TypeError("unhashable type")` at insertion (caught in `store_item`, `BuildDict`, `build_set`).

## Representation

### repr

Developer-readable form. Quotes strings; renders containers with element `repr`s.

```python
print(repr("hello"))
print(repr(42))
print(repr([1, "two", 3]))
```

```text Output
'hello'
42
[1, 'two', 3]
```

### format

`format(value)` -> display form. `format(value, spec)` applies the f-string spec mini-language (`[[fill]align][sign][#][0][width][,][.precision][type]`). Width and precision are capped (precision at 65,000). A larger value raises `ValueError` rather than allocating an oversized string.

```python
print(format(42))
print(format(42, "05d"))
print(format(3.14159, ".2f"))
print(format(255, "#x"))
print(format("hi", ">10"))
```

```text Output
42
00042
3.14
0xff
        hi
```

## Attribute access

`getattr` / `hasattr` / `delattr` consult the instance `__dict__` first, then the user class chain (including inherited methods), then the built-in method table for primitives (str/bytes/list/dict/set, plus the small int/float method set), class attributes on a class object, and attributes stored on functions. Bound methods expose `__self__`, and user methods also `__func__`. So `hasattr(MyClass(), 'my_method')` is `True` for a defined method, and `delattr` on a missing attribute raises `AttributeError`.

### getattr

```python
m = getattr("hello", "upper")
print(m())
print(getattr("hello", "missing", "default"))
```

```text Output
HELLO
default
```

### hasattr

```python
print(hasattr("hello", "upper"))
print(hasattr([1, 2], "append"))
print(hasattr("hello", "missing"))
```

```text Output
True
True
False
```

### globals, locals

`globals()`: fresh dict snapshot of module-level bindings (builtins, types, top-level assignments). `locals()`: fresh dict of the current frame: function locals inside a function, same as `globals()` minus builtins at module level.

```python
x = 100
y = 200

def add(a, b):
    return a + b

g = globals()
print(g['x'] + g['y'])

# Dynamic dispatch by name
fn = globals()['add']
print(fn(3, 4))

def f():
  a = 1
  b = 2
  return locals()
print(f())
```

```text Output
300
7
{'b': 2, 'a': 1}
```

Dicts are copies. Mutation doesn't change VM bindings.

### setattr, delattr

`setattr(obj, name, value)` / `delattr(obj, name)` store/remove on user instances, on user classes (`cls.attr = ...` works too), and on functions. Builtin types have no writable attribute table.

```python
class Box:
  def __init__(self):
    pass

b = Box()
setattr(b, "x", 42)
print(b.x)
delattr(b, "x")
print(hasattr(b, "x"))
```

```text Output
42
False
```

### slice

`slice(stop)`, `slice(start, stop)`, `slice(start, stop, step)`: the `slice` type. Builds a reusable slice usable as a sequence index; `type(s) is slice` and `isinstance(s, slice)` hold.

```python
xs = [10, 20, 30, 40, 50]
s = slice(1, 4)
print(xs[s])
print(xs[slice(0, 5, 2)])
```

```text Output
[20, 30, 40]
[10, 30, 50]
```

### vars

`vars(instance)` -> attr-dict snapshot. `vars(module)` -> exported-names dict. Only instances and modules, no no-arg form (use `locals()`).

```python
class P:
  def __init__(self):
    self.x = 1
    self.y = 2

p = P()
print(vars(p))
```

```text Output
{'x': 1, 'y': 2}
```

## Classes

### super

`super()`: zero-arg only. Proxy resolving attribute access against the bases of the current method's class, starting one step up the MRO. Outside a method -> `TypeError`.

```python
class A:
  def m(self):
    return "a"

class B(A):
  def m(self):
    return super().m() + "b"

print(B().m())
```

```text Output
ab
```

### property

`property(fget, fset=None)`: descriptor for class members. Usually applied via `@property` with optional `@<name>.setter`.

```python
class C:
  def __init__(self, x):
    self._x = x
  @property
  def x(self):
    return self._x
  @x.setter
  def x(self, v):
    self._x = v

c = C(1)
c.x = 9
print(c.x)
```

```text Output
9
```

### staticmethod

`staticmethod(func)`: wraps a class member so it receives no implicit `self`. Usually applied via `@staticmethod`. Callable through the class or an instance with identical arguments.

```python
class Math:
  @staticmethod
  def add(a, b):
    return a + b

print(Math.add(2, 3))
print(Math().add(4, 5))
```

```text Output
5
9
```

### classmethod

`classmethod(func)`: wraps a class member so it receives the class as its first argument. Usually applied via `@classmethod`. Full model in [Class methods](/language/classes#class-methods).

## Async

Concurrency primitives. Full model in [Async](/language/async).

### run

`run(*coros)`: schedules every arg, drains until each reaches a terminal state, returns the first arg's result. Errors from peers other than the first are discarded. For fan-out that collects every result, use `gather`.

### sleep

`sleep(seconds)`: yield and resume after the duration. Negative clamps to zero. Without a host time hook, a virtual clock advances. With one, the scheduler signals `PendingTimer(deadline_ns)` and the embedder resumes via `run_resume`.

### frame

`frame()`: yield until the host's next render frame. Coro -> `WaitingFrame`, scheduler signals `PendingFrame`. Browser embedders hook `requestAnimationFrame`. Use for animation loops at display refresh rate.

```python
async def animate(node):
  for i in range(60):
    set_attribute(node, "style", f"transform: translateX({i}px)")
    frame()
```

### receive

`receive()`: pop the oldest message from the scheduler queue. Empty -> parks in `WaitingEvent`, scheduler signals `PendingEvent`. Embedder resumes via `run_push_event(bytes)`. Messages are arbitrary strings (e.g. DOM event names from `bind_event`).

### gather

`gather(*coros)`: concurrent fan-out. Schedules every arg, drains until each terminal, returns a list of results in argument order. First-error propagates after all peers terminal.

```python
async def task(n):
  return n * 2

print(gather(task(1), task(2), task(3)))
```

```text Output
[2, 4, 6]
```

### with_timeout

`with_timeout(seconds, coro)`: runs `coro` to completion or raises `TimeoutError` on deadline. Coro cancelled on timeout.

```python
async def slow():
  sleep(10)
  return "never"

try:
  with_timeout(0.1, slow())
except TimeoutError:
  print("timed out")
```

```text Output
timed out
```

### cancel

`cancel(coro)`: flag a registered coroutine for cancellation. The next tick stops it. Cooperative and silent: the body doesn't observe `CancelledError`. For deadline-driven exception-style cancellation, use `with_timeout`.
