---
title: "Dunder methods"
description: "Protocol methods Edge Python invokes on user classes; operators, indexing, iteration, hashing, context managers, attribute fallback."
---

Dunders (`__add__`, `__eq__`, `__getitem__`, ...) plug a class into language protocols. Define them in the class body. The VM calls them when the matching operator, builtin, or syntax form runs.

```python
class V:
  def __init__(self, n):
    self.n = n
  def __add__(self, o):
    return V(self.n + o.n)
  def __eq__(self, o):
    return self.n == o.n

print((V(3) + V(4)).n)
print(V(3) == V(3))
```

```text Output
7
True
```

Dunders are looked up on the class chain. The instance dict is skipped. Subclasses inherit and may override. Operator overloading composes with [inheritance](/language/classes#inheritance-and-super). Dispatch is cached per call site; see [Design](/implementation/design) for the inline-cache mechanics.

## Arithmetic

| Operator | Forward | Reflected |
|----------|-----------------|------------------|
| `a + b` | `__add__` | `__radd__` |
| `a - b` | `__sub__` | `__rsub__` |
| `a * b` | `__mul__` | `__rmul__` |
| `a / b` | `__truediv__` | `__rtruediv__` |
| `a // b` | `__floordiv__` | `__rfloordiv__` |
| `a % b` | `__mod__` | `__rmod__` |
| `a ** b` | `__pow__` | `__rpow__` |
| `-a` | `__neg__` | - |
| `+a` | `__pos__` | - |

Return `NotImplemented` from the forward op to make the VM try the reflected op on the other operand. Both `NotImplemented` (or neither defined) -> `TypeError`.

Subclass-first: when `type(b)` is a strict subclass of `type(a)`, `b.__radd__` runs before `a.__add__`. This lets a subclass override an inherited reflected op without touching the base.

```python
class Money:
  def __init__(self, n): self.n = n
  def __add__(self, o):
    return Money(self.n + (o.n if isinstance(o, Money) else o))
  def __radd__(self, o):
    return Money(o + self.n)

print((Money(10) + Money(5)).n)
print((3 + Money(7)).n)
```

```text Output
15
10
```

## Bitwise and shifts

The bitwise and shift operators follow the same forward/reflected protocol.

| Operator | Forward | Reflected |
|----------|-----------------|------------------|
| `a \| b` | `__or__` | `__ror__` |
| `a & b` | `__and__` | `__rand__` |
| `a ^ b` | `__xor__` | `__rxor__` |
| `a << b` | `__lshift__` | `__rlshift__` |
| `a >> b` | `__rshift__` | `__rrshift__` |

## Comparison

| Operator | Forward | Reflected |
|------------|-------------|------------|
| `a == b` | `__eq__` | `__eq__` |
| `a != b` | `__ne__` | `__ne__` |
| `a < b` | `__lt__` | `__gt__` |
| `a <= b` | `__le__` | `__ge__` |
| `a > b` | `__gt__` | `__lt__` |
| `a >= b` | `__ge__` | `__le__` |

`!=` falls back to `not __eq__` (coerced to `bool`) when `__ne__` is absent. Every other comparison returns the dunder's raw result: `__lt__` returning `'A.lt'` yields the string, not `True`.

## Truth and length

`bool(x)` (and any boolean context) consults:

1. `__bool__` if defined (must return `bool`, else `TypeError`).
2. `__len__` if defined -> `False` when length is 0, else `True`.
3. Default `True`.

`len(x)` calls `__len__` directly. It must return a non-negative int.

```python
class Empty:
  def __bool__(self):
    return False

class Container:
  def __init__(self, n): self.n = n
  def __len__(self):
    return self.n

print(bool(Empty()))
print(bool(Container(0)), bool(Container(3)))
print(len(Container(5)))
```

```text Output
False
False True
5
```

## Indexing and containment

| Form | Dunder | Arguments |
|---------------------|------------------|------------------------|
| `obj[i]` | `__getitem__` | `(self, i)` |
| `obj[i] = v` | `__setitem__` | `(self, i, value)` |
| `del obj[i]` | `__delitem__` | `(self, i)` |
| `v in obj` | `__contains__` | `(self, value)` |

Slices pass as a `slice` object: `obj[1:3]` calls `__getitem__(self, slice(1, 3, None))`.

Absent `__contains__`: `v in obj` falls back to iterating `obj` with `__eq__`.

## Iteration

| Method | Role |
|---------------|----------------------------------------------------------------------|
| `__iter__` | Returns an iterator (often `self`). |
| `__next__` | Returns the next item, or raises `StopIteration` to end the loop. |

```python
class Up:
  def __init__(self, stop):
    self.i = 0
    self.stop = stop
  def __iter__(self):
    return self
  def __next__(self):
    if self.i >= self.stop:
      raise StopIteration
    self.i += 1
    return self.i

print(list(Up(3)))
```

```text Output
[1, 2, 3]
```

`for` loops, `list(x)`, and `tuple(x)` all honour the protocol.

## Callable

`__call__` makes instances invocable. Positional and keyword arguments are forwarded like any method call.

```python
class Double:
  def __call__(self, x, times=2):
    return x * times

d = Double()
print(d(7))
print(d(7, times=3))
print(callable(d))
```

```text Output
14
21
True
```

## Hashing

`hash(x)` calls `__hash__`. It must return `int` (masked to `INT_MAX`).

Eq/hash invariant: a class defining `__eq__` without `__hash__` is unhashable: `hash(x)` and `{x: 1}` raise `TypeError`. Prevents inconsistent dict keys.

```python
class K:
  def __init__(self, n): self.n = n
  def __hash__(self):
    return self.n
  def __eq__(self, o):
    return self.n == o.n

k = K(5)
print(hash(k))
print({k: 'found'}[k]) # same instance reference looks up reliably
```

```text Output
5
found
```

Built-in dict/set compare instance keys by identity (`Val` bits). User `__hash__` is returned by `hash()`, but doesn't change containment in built-in containers. Use the same instance reference to look up reliably.

## Representation

| Function / form | Dunder | Fallback |
|---------------------|---------------|-----------------------------|
| `repr(x)` | `__repr__` | `<ClassName instance>` |
| `str(x)`, `print(x)`| `__str__` | `__repr__`, then default |
| `f"{x}"` (no spec) | `__str__` | same as `str(x)` |
| `f"{x:spec}"` | `__format__` | built-in format spec engine |
| `f"{x!r}"` | `__repr__` | - |

`__format__(spec)` receives the spec string and must return `str`. `int(x)` on an instance calls `__int__`, also used by `%d` / `%x` / `%X` / `%o` formatting.

## Attribute access fallback

`__getattr__(self, name)` runs only when normal lookup (instance dict -> class chain) misses. It receives the name as a string. Return the value, or raise `AttributeError` to surface a real miss.

```python
class Proxy:
  def __getattr__(self, name):
    return f"computed:{name}"

p = Proxy()
print(p.anything)
print(p.foo)
```

```text Output
computed:anything
computed:foo
```

Existing attributes bypass `__getattr__`. Only misses trigger it.

## Context managers

`with cm() as x:` invokes `__enter__`. Its return binds to `as`. On exit, `__exit__(exc_type, exc_value, traceback)` runs: `(None, None, None)` for normal exit; on a raise, the exception type and value with `traceback` always `None`. A truthy return suppresses the exception; a falsy one propagates it.

```python
class Suppress:
  def __enter__(self):
    return self
  def __exit__(self, t, v, tb):
    return True # swallow whatever raised

with Suppress():
  raise ValueError("boom")
print("after")
```

```text Output
after
```

Multiple managers (`with a(), b() as x:`) nest LIFO: `b` enters last, exits first. Each has its own implicit handler, so inner suppression still lets outer managers run their normal `__exit__(None, None, None)`.

If `__exit__` itself raises, the new exception replaces the original.

## What's not dispatched

Parsed for compatibility but never invoked on user classes:

- `__init_subclass__`, `__set_name__`, descriptors (`__get__` / `__set__` / `__delete__`)
- `__new__`, VM constructs the instance; `__init__` runs user logic
- Augmented-assignment dunders (`__iadd__`, ...), `a += b` desugars to `a = a + b`, so `__add__` covers it. Exception: list `+=` extends in place (alias-visible); see [Data types](/language/data-types#list)
- Async dunders (`__aenter__` / `__aexit__` / `__aiter__` / `__anext__`), `async with` / `async for` use the sync paths

## See also

- [Classes](/language/classes): constructors, inheritance, properties.
