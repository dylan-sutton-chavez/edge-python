---
title: "Built-in functions"
description: "The global functions available in every Edge Python program."
---

Edge Python provides 68 built-in functions. They are first-class values, so you can pass them as arguments, store them in containers, and alias them.

```python
fns = [abs, hex, str]
print([f(-3) for f in fns])

p = print
p("aliased")
```

```text Output
[3, '-0x3', '-3']
aliased
```

There is no `eval`, `exec`, `compile`, `open`, or `__import__`. Static imports and the sandbox rule them out.

## Output

### print

`print(*args, sep=' ', end='\n')` writes the arguments joined by `sep`, then `end`. `*` unpacking spreads an iterable into the arguments. The `file` and `flush` keywords are accepted and ignored.

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

`input()` pops one line from the host-provided input buffer and returns it as a string. There is no prompt argument. The CLI fills the buffer from piped stdin, one line per call. An empty buffer raises `RuntimeError`. In WASM the host copies stdin bytes into the guest input buffer before running.

## Numeric

### abs

`abs(x)` returns the absolute value of an int or float. Other types raise `TypeError`.

```python
print(abs(-7))
print(abs(3.14))
```

```text Output
7
3.14
```

### round

`round(x)` rounds to the nearest integer and returns an int. Ties go to even. `round(x, n)` rounds to `n` decimal digits and returns a float. A negative `n` rounds to tens, hundreds, and so on.

```python
print(round(2.5))
print(round(0.5))
print(round(-1.5))
print(round(1.55, 1))
print(round(1234, -2))
```

```text Output
2
0
-2
1.6
1200
```

### min, max

`min(a, b, ...)` takes several values or a single iterable. `max` works the same way. An empty iterable raises `ValueError` unless a `default=` is given. A `key=` function selects the comparison value while the original element is returned.

```python
print(min(3, 1, 4))
print(max([3, 1, 4]))
print(min([], default=-1))
print(max(["a", "bb", "c"], key=len))
```

```text Output
1
4
-1
bb
```

### sum

`sum(iterable)` or `sum(iterable, start)`. An empty iterable sums to `start`, which defaults to `0`.

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

`pow(base, exp)` matches the `**` operator. `pow(base, exp, mod)` does modular exponentiation on integers. The three-argument form requires a non-negative exponent and a modulus with absolute value at most 2^63. A zero modulus raises `ZeroDivisionError`. The other violations raise `ValueError`.

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

`divmod(a, b)` returns `(a // b, a % b)` as a tuple. Ints and floats both work. Float operands give a float quotient and remainder.

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

`bin(x)`, `oct(x)`, and `hex(x)` format an integer in base 2, 8, or 16 with the matching prefix.

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

`int(x)` accepts an int, bool, float, or numeric string. Floats truncate toward zero. Strings accept `_` as a digit separator. `int(s, base)` parses a string in radix 2 to 36, or radix 0 to auto-detect a `0x`, `0o`, or `0b` prefix. Bad strings raise `ValueError`. `int(inf)` raises `OverflowError` and `int(nan)` raises `ValueError`. Results are bounded by the [integer width](/reference/limits-and-errors#integer-width).

```python
print(int(3.9))
print(int("42"))
print(int(True))
print(int("ff", 16))
print(int("0x1f", 0))
print(int("1_000"))
```

```text Output
3
42
1
255
31
1000
```

### float

`float(x)` accepts an int, bool, float, or string. Strings recognize `inf`, `-inf`, and `nan`, case-insensitively.

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

`str(x)` returns the display form of `x`. No argument gives an empty string. `str(bytes, encoding)` decodes bytes like [`bytes.decode`](/reference/methods#bytes-methods).

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

`bool(x)` returns the truth value of `x`. The rules live in [Truthy and falsy](/language/data-types#truthy-and-falsy).

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

### list, tuple, set, frozenset

Each accepts any iterable and builds a new container. Iterating a dict yields its keys. With no argument, each builds an empty container. A live generator object (a `def` with `yield`) is only accepted by `list()`. The others raise `TypeError`.

```python
print(list("abc"))
print(tuple(range(3)))
print(set({"a": 1, "b": 2}))
print(frozenset(b"\x01\x02\x03"))
```

```text Output
['a', 'b', 'c']
(0, 1, 2)
{'b', 'a'}
frozenset({1, 2, 3})
```

### dict

`dict()` builds from a mapping, an iterable of key/value pairs, keyword arguments, or a mix. Each pair must have length 2.

```python
print(dict(a=1, b=2))
print(dict([("a", 1)]))
print(dict({"a": 1}, b=2))
```

```text Output
{'a': 1, 'b': 2}
{'a': 1}
{'a': 1, 'b': 2}
```

### chr, ord

`chr(i)` returns the one-character string for code point `i`, across full Unicode. Out-of-range values raise `ValueError`. `ord(c)` is the inverse and accepts a length-1 string or length-1 bytes.

```python
print(chr(65))
print(ord("A"))
print(ord(b"A"))
print(chr(0x1F600))
```

```text Output
A
65
65
😀
```

## Sequences and iteration

### len

`len(x)` returns the element count of a string (in code points), bytes, list, tuple, dict, set, frozenset, or range. Other types raise `TypeError`.

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

`range(stop)`, `range(start, stop)`, or `range(start, stop, step)`. Lazy. A zero step raises `ValueError` and non-integer arguments raise `TypeError`. Two ranges compare equal when they produce the same sequence of values.

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

`sorted(iterable)` returns a new sorted list. `key=fn` compares by `fn(item)`. `reverse=True` flips the order. Numbers, strings, bytes, and lists or tuples order lexicographically. Objects with `__lt__` sort by it. Mixing unordered types raises `TypeError`.

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

`reversed(x)` returns a new list in reverse order. It is eager, not a lazy iterator. A string becomes a list of one-character strings.

```python
print(reversed([1, 2, 3]))
print(reversed("abc"))
```

```text Output
[3, 2, 1]
['c', 'b', 'a']
```

### enumerate

`enumerate(iterable)` returns a list of `(index, value)` tuples. A second argument, positional or `start=`, sets the first index.

```python
for i, v in enumerate(["a", "b", "c"]):
    print(i, v)

print(enumerate(["a", "b"], start=7))
```

```text Output
0 a
1 b
2 c
[(7, 'a'), (8, 'b')]
```

### zip

`zip(a, b, ...)` returns a list of tuples pairing the inputs, truncated to the shortest. There is no `strict=` mode.

```python
for a, b in zip([1, 2, 3], ["x", "y"]):
    print(a, b)

print(list(zip([1, 2], [3, 4], [5, 6])))
```

```text Output
1 x
2 y
[(1, 3, 5), (2, 4, 6)]
```

### iter, next

`iter(x)` returns a fresh iterator over any iterable. It materialises a snapshot, so the original is never mutated. `next(it)` returns the next item and raises `StopIteration` when exhausted. `next(it, default)` returns `default` instead of raising. The two-argument `iter(callable, sentinel)` calls `callable()` until it returns `sentinel`.

```python
it = iter([10, 20, 30])
print(next(it))
print(next(it))
print(next(it))
print(next(it, "done"))
```

```text Output
10
20
30
done
```

### map, filter

`map(fn, *iterables)` returns a list of `fn(items...)`. Several iterables are walked in parallel and stop at the shortest. `filter(pred, iterable)` returns a list of items where `pred(item)` is truthy. A `None` predicate keeps truthy items. Both are eager.

```python
print(list(map(lambda x: x * 2, [1, 2, 3])))
print(list(map(lambda a, b: a + b, [1, 2], [10, 20])))
print(list(filter(lambda x: x > 2, [1, 2, 3, 4])))
print(list(filter(None, [0, 1, "", "hi", [], [1]])))
```

```text Output
[2, 4, 6]
[11, 22]
[3, 4]
[1, 'hi', [1]]
```

### all, any

`all(x)` and `any(x)` test truthiness across an iterable and short-circuit at the deciding element. `all([])` is `True` and `any([])` is `False`.

```python
print(all([1, 2, 3]))
print(all([1, 0, 3]))
print(all([]))
print(any([0, 0, 1]))
print(any([]))
```

```text Output
True
False
True
True
False
```

### slice

`slice(stop)`, `slice(start, stop)`, or `slice(start, stop, step)` builds a reusable slice object usable as a sequence index.

```python
xs = [10, 20, 30, 40, 50]
print(xs[slice(1, 4)])
print(xs[slice(0, 5, 2)])
```

```text Output
[20, 30, 40]
[10, 30, 50]
```

## Bytes helpers

`bytes_fromhex(s)` parses a hex string into bytes. ASCII whitespace is ignored and non-hex input raises `ValueError`.

`int_from_bytes(b, order)` reads bytes as an unsigned integer. `order` is `"big"` or `"little"`. At most 8 bytes, anything longer raises `OverflowError`.

`int_to_bytes(n, length, order)` converts a non-negative int to `length` bytes. `length` is at most 8. A negative `n` raises `ValueError` and a value that does not fit raises `OverflowError`.

The methods [`bytes.fromhex`, `int.from_bytes`, and `int.to_bytes`](/reference/methods#int-and-float-methods) do the same jobs with default arguments and no 8-byte cap.

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

## Type and identity

### type

`type(x)` returns the type object of `x`. The built-in type names are these same objects, so `type(x) is int` holds, and calling one constructs a value. For a user instance the result is its class object.

```python
print(type(42))
print(type(42) is int)
print(type([1, 2, 3])([4, 5]))

class C:
  pass

print(type(C()) is C)
```

```text Output
<class 'int'>
True
[4, 5]
True
```

Functions, type objects, and classes expose `__name__`, the bare declared name. On an exception instance, `type(e).__name__` gives the exception's class name.

```python
def greet():
  pass

print(greet.__name__)
print(int.__name__)

try:
  1 / 0
except Exception as e:
  print(type(e).__name__)
```

```text Output
greet
int
ZeroDivisionError
```

### object

`object()` returns a unique featureless instance. Use it as a sentinel. Every value is an instance of `object`.

```python
SENTINEL = object()

print(SENTINEL is object())
print(type(SENTINEL) is object)
print(isinstance(42, object))
```

```text Output
False
True
True
```

### isinstance

`isinstance(obj, t)` tests membership. `t` is a built-in type, exception class, user class, or a tuple of those. `bool` counts as `int`. Exception classes follow the [standard hierarchy](/reference/limits-and-errors#exception-hierarchy). User classes walk their inheritance chain. `object` matches every value.

```python
print(isinstance(42, int))
print(isinstance(True, int))
print(isinstance("x", (int, str)))
```

```text Output
True
True
True
```

### issubclass

`issubclass(C, B)` tests inheritance. `B` may be a tuple of classes. `C` must itself be a class or the call raises `TypeError`. `bool` is a subclass of `int`, and exception classes follow the standard hierarchy.

```python
print(issubclass(ZeroDivisionError, Exception))
print(issubclass(bool, int))

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
False
```

### callable

`callable(x)` is `True` for functions, lambdas, bound methods, type objects, built-in functions, and instances whose class defines `__call__`. `False` for everything else.

```python
print(callable(print))
print(callable(lambda x: x))
print(callable(42))
```

```text Output
True
True
False
```

### id, hash

`id(x)` returns a stable numeric identifier for the value. `hash(x)` returns the hash of a hashable value. Lists, dicts, and sets are unhashable and raise `TypeError`. Ints hash to themselves. Integral floats hash as the equal int, so `hash(1) == hash(1.0)`.

```python
print(hash("hello") == hash("hello"))
print(hash((1, 2, 3)) == hash((1, 2, 3)))
print(hash(1) == hash(1.0))

try:
    hash([1, 2, 3])
except TypeError:
    print("unhashable")
```

```text Output
True
True
True
unhashable
```

## Representation

### repr

`repr(x)` returns the developer-readable form. Strings are quoted and containers show the `repr` of their elements.

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

`format(value)` returns the display form. `format(value, spec)` applies the format spec mini-language from [f-strings](/language/syntax#f-strings).

```python
print(format(42))
print(format(42, "05d"))
print(format(3.14159, ".2f"))
print(format(255, "#x"))
```

```text Output
42
00042
3.14
0xff
```

## Attributes

`getattr(obj, name)` reads an attribute, looking in the instance `__dict__`, then the class chain, then the built-in method table. A missing name raises `AttributeError` unless a third argument gives a default.

`hasattr(obj, name)` runs the same lookup and returns a boolean.

`setattr(obj, name, value)` writes an attribute on a user instance, class, or function. Built-in types have no writable attributes.

`delattr(obj, name)` removes an attribute. A missing name raises `AttributeError` on an instance and is silently ignored on a class.

```python
class Box:
  pass

b = Box()
setattr(b, "x", 42)
print(b.x)
print(getattr(b, "missing", "default"))

delattr(b, "x")
print(hasattr(b, "x"))
```

```text Output
42
default
False
```

### vars

`vars(x)` returns a snapshot of the attribute dict of an instance or module. There is no no-argument form. Use `locals()` instead.

```python
class P:
  def __init__(self):
    self.x = 1
    self.y = 2

print(vars(P()))
```

```text Output
{'x': 1, 'y': 2}
```

### globals, locals

`globals()` returns a fresh dict of the module-level bindings. User names only, since built-ins live in a separate namespace. `locals()` returns a fresh dict of the current frame's locals inside a function, and matches `globals()` at module level. Both are copies. Mutating them does not change bindings.

```python
x = 100

def add(a, b):
    return a + b

print(globals()["x"])
print(globals()["add"](3, 4))

def f():
  a = 1
  b = 2
  return locals()

print(f())
```

```text Output
100
7
{'b': 2, 'a': 1}
```

## Modules

### import_module

`import_module(name)` returns a module that was imported statically somewhere in the program. It is a lookup, not a load. Every reachable module is still resolved and verified at compile time. An unknown name raises `NameError`. A name bound to a non-module global, such as a function, raises `TypeError`.

```python
import math

m = import_module("math")
print(m is math)
print(m.floor(3.7))
```

```text Output
True
3
```

Dynamic loading through `importlib` or `__import__` does not exist. Static imports plus `import_module` replace it.

## Classes

### super

`super()` takes no arguments and must be called inside a method. It returns a proxy that resolves attributes against the bases of the current class, starting one step up. See [Inheritance and super()](/language/classes#inheritance-and-super).

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

`property(fget, fset=None)` builds a descriptor for a class member. Usually applied through `@property` with an optional `@<name>.setter`. See [Properties](/language/classes#properties).

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

### staticmethod, classmethod

`staticmethod(func)` wraps a class member so it receives no implicit `self`. `classmethod(func)` wraps one so it receives the class as its first argument. Usually applied as decorators. See [Static methods](/language/classes#static-methods) and [Class methods](/language/classes#class-methods).

```python
class Math:
  @staticmethod
  def add(a, b):
    return a + b

  @classmethod
  def name(cls):
    return cls.__name__

print(Math.add(2, 3))
print(Math().add(4, 5))
print(Math.name())
```

```text Output
5
9
Math
```

## Async

These functions drive coroutines. [Async](/language/async) owns the full model.

- `run(*coros)` runs every argument to completion and returns the first argument's result. Errors from the other coroutines are discarded.
- `gather(*coros)` runs every argument and returns a list of results in argument order. The first error propagates.
- `sleep(seconds)` suspends for the duration. A negative value clamps to zero.
- `with_timeout(seconds, coro)` returns the coroutine's result or raises `TimeoutError` at the deadline.
- `cancel(coro)` flags a coroutine for cancellation at its next step.
- `frame()` suspends until the host's next render frame.
- `receive()` pops the oldest queued host message.

```python
async def task(n):
  return n * 2

print(gather(task(1), task(2), task(3)))

async def main():
  sleep(0)
  return 42

print(run(main()))
```

```text Output
[2, 4, 6]
42
```
