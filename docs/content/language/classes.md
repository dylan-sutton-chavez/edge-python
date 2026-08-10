---
title: "Classes"
description: "User-defined classes, inheritance, properties, and the dunder protocol."
---

Classes are state containers and namespaces, not the primary abstraction. This is a design choice. Two patterns:

- State machines: a few methods that mutate the receiver.
- Namespaces: a bundle of related functions and constants.

Supported:

- Single and multiple inheritance (C3 MRO) with `super()`.
- `@property` / `@x.setter`.
- `@staticmethod` and `@classmethod`.
- A curated dunder protocol for operators, indexing, iteration, hashing, context managers, and attribute fallback (see [Operator overloading and protocols](#operator-overloading-and-protocols)).

Out of scope: descriptors, metaclasses, `__slots__`.

## State-machine pattern

```python
class Counter:
  def __init__(self, n=0):
    self.n = n
  def tick(self):
    self.n += 1
  def value(self):
    return self.n

c = Counter()
c.tick()
c.tick()
c.tick()
print(c.value())
```

```text Output
3
```

## Namespace pattern

A class with no `__init__` and no per-instance state is a namespace. Methods called on the class are unbound, with no `self` prepended.

```python
class Status:
  IDLE = 0
  RUNNING = 1
  DONE = 2

class Math:
  PI = 3.14159
  def square(x):
    return x * x
  def cube(x):
    return x * x * x

print(Status.IDLE)
print(Math.PI)
print(Math.square(5))
print(Math.cube(3))
```

```text Output
0
3.14159
25
27
```

## Inheritance and super()

Single or multiple bases (`class Sub(Base):`, `class C(A, B):`). Methods not on the subclass resolve along the C3 linearization (the MRO). An inconsistent hierarchy raises `TypeError` at class creation. `isinstance(x, Base)` walks the ancestor chain, so `Sub` instances are also instances of every ancestor.

`super()` (zero-arg) delegates to the next class up the chain, bound to the current `self`. Most common in `__init__` to extend a base constructor.

```python
class Animal:
  def __init__(self, name):
    self.name = name
  def describe(self):
    return self.name

class Dog(Animal):
  def __init__(self, name, breed):
    super().__init__(name)
    self.breed = breed
  def describe(self):
    return super().describe() + " (" + self.breed + ")"

d = Dog("Rex", "lab")
print(d.describe())
print(isinstance(d, Animal))
```

```text Output
Rex (lab)
True
```

```python
class A:
  def who(self):
    return "A"
class B(A):
  def who(self):
    return "B"
class C(A):
  def who(self):
    return "C"
class D(B, C):
  pass

print(D().who())  # B comes first in the C3 order

try:
  class Bad(A, B):
    pass
except TypeError:
  print("inconsistent hierarchy")
```

```text Output
B
inconsistent hierarchy
```

## Attribute access on classes vs instances

| Access form | Resolves to |
|---------------------|-------------------------------------------|
| `MyClass.attr` | class member, returned as-is (no binding) |
| `MyClass.method()` | method called directly, no `self` |
| `instance.attr` | instance `__dict__` first, then class |
| `instance.method()` | bound method, `self` prepended |

`setattr` / `delattr` work on instances and on class objects. The latter mutates the class's members.

## Class decorators

A class decorator is called with the class object and its return value binds to the name. It can add or replace class attributes (`cls.kind = ...`) or return a replacement.

```python
def tag(cls):
  cls.kind = "tagged"
  return cls

@tag
class Box:
  def __init__(self, v):
    self.v = v

print(Box.kind)
print(Box(7).v)
```

```text Output
tagged
7
```

## Properties

`@property` turns a method into a read-only attribute. `@x.setter` makes it writable. Properties live on the class. Subclasses inherit and can override either side.

```python
class Temp:
  def __init__(self, c):
    self._c = c
  @property
  def celsius(self):
    return self._c
  @celsius.setter
  def celsius(self, value):
    self._c = value
  @property
  def fahrenheit(self):
    return self._c * 9 / 5 + 32

t = Temp(20)
print(t.celsius)
print(t.fahrenheit)
t.celsius = 100
print(t.fahrenheit)
```

```text Output
20
68.0
212.0
```

The two-argument form `property(fget, fset)` also works without decorator syntax.

## Static methods

`@staticmethod` makes a method that receives no implicit `self`. It is a plain function that lives in the class namespace, callable as `Class.method(...)` or `instance.method(...)` with identical arguments. Subclasses inherit it and can override it. Use it for helpers that belong to a class conceptually but need no receiver.

```python
class Geometry:
  @staticmethod
  def add(a, b):
    return a + b
  @staticmethod
  def triangle_area(base, height):
    return base * height / 2

print(Geometry.add(2, 3))
print(Geometry().triangle_area(10, 4))
```

```text Output
5
20.0
```

The functional form `staticmethod(func)` also works without decorator syntax.

## Class methods

`@classmethod` binds the class, not the instance, as the first argument. Accessed through a subclass, `cls` is the subclass, so alternate constructors return the right type down the hierarchy.

```python
class Color:
  def __init__(self, r, g):
    self.r, self.g = r, g
  @classmethod
  def rgb(cls, r, g=0):
    return cls(r, g)
  @classmethod
  def which(cls):
    return cls.__name__

class Bright(Color):
  pass

c = Color.rgb(1, g=2)
print(c.r, c.g)
print(Color.which(), Bright.which())
```

```text Output
1 2
Color Bright
```

The functional form `classmethod(func)` also works without decorator syntax.

## Operator overloading and protocols

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

Dunders are looked up on the class chain. The instance dict is skipped, so assigning `obj.__add__ = ...` has no effect. Subclasses inherit and may override.

### Arithmetic

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

Return `NotImplemented` from the forward op to make the VM try the reflected op on the other operand. If both return `NotImplemented` (or neither is defined), the operation raises `TypeError`.

Subclass-first ordering applies here. When `type(b)` is a strict subclass of `type(a)`, `b.__radd__` runs before `a.__add__`. This lets a subclass override an inherited reflected op without touching the base.

```python
class Base:
  def __add__(self, o):
    return "base.__add__"
  def __radd__(self, o):
    return "base.__radd__"
class Sub(Base):
  def __radd__(self, o):
    return "sub.__radd__"

print(Base() + Sub())
```

```text Output
sub.__radd__
```

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

### Bitwise and shifts

The bitwise and shift operators follow the same forward/reflected protocol.

| Operator | Forward | Reflected |
|----------|-----------------|------------------|
| `a \| b` | `__or__` | `__ror__` |
| `a & b` | `__and__` | `__rand__` |
| `a ^ b` | `__xor__` | `__rxor__` |
| `a << b` | `__lshift__` | `__rlshift__` |
| `a >> b` | `__rshift__` | `__rrshift__` |
| `~a` | `__invert__` | - |

### Comparison

| Operator | Forward | Reflected |
|------------|-------------|------------|
| `a == b` | `__eq__` | `__eq__` |
| `a != b` | `__ne__` | `__ne__` |
| `a < b` | `__lt__` | `__gt__` |
| `a <= b` | `__le__` | `__ge__` |
| `a > b` | `__gt__` | `__lt__` |
| `a >= b` | `__ge__` | `__le__` |

`!=` falls back to `not __eq__` (coerced to `bool`) when `__ne__` is absent. Every other comparison returns the dunder's raw result. A `__lt__` that returns `'A.lt'` yields the string, not `True`.

```python
class N:
  def __init__(self, v):
    self.v = v
  def __eq__(self, o):
    return self.v == o.v

print(N(1) != N(2))
print(N(1) != N(1))
```

```text Output
True
False
```

### Truth and length

`bool(x)` (and any boolean context) consults, in order:

1. `__bool__` if defined. It must return `bool`, else `TypeError`.
2. `__len__` if defined. `False` when the length is 0, else `True`.
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

### Indexing and containment

| Form | Dunder | Arguments |
|---------------------|------------------|------------------------|
| `obj[i]` | `__getitem__` | `(self, i)` |
| `obj[i] = v` | `__setitem__` | `(self, i, value)` |
| `del obj[i]` | `__delitem__` | `(self, i)` |
| `v in obj` | `__contains__` | `(self, value)` |

Slices pass as a `slice` object, so `obj[1:3]` calls `__getitem__(self, slice(1, 3, None))`. Indexes on built-in sequences coerce via `__index__`, including slice bounds. Dict keys never coerce.

Without `__contains__`, `v in obj` falls back to iterating `obj` and comparing with `__eq__`.

```python
class Store:
  def __init__(self):
    self.data = {}
  def __setitem__(self, key, value):
    self.data[key] = value
  def __getitem__(self, key):
    return self.data.get(key, "missing")
  def __contains__(self, key):
    return key in self.data

s = Store()
s["a"] = 1
print(s["a"], s["b"])
print("a" in s, "z" in s)
```

```text Output
1 missing
True False
```

### Iteration

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
print(2 in Up(3))
```

```text Output
[1, 2, 3]
True
```

`for` loops, `list(x)`, and `tuple(x)` all honour the protocol.

### Callable

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

### Hashing

`hash(x)` calls `__hash__`. It must return an `int`, which is masked to `INT_MAX`.

Eq/hash invariant. A class that defines `__eq__` without `__hash__` is unhashable. `hash(x)` and `{x: 1}` raise `TypeError`. This prevents inconsistent dict keys.

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

Built-in dict and set compare instance keys by identity. A user `__hash__` is returned by `hash()`, but does not change containment in built-in containers. Use the same instance reference to look up reliably.

### Representation

| Function / form | Dunder | Fallback |
|---------------------|---------------|-----------------------------|
| `repr(x)` | `__repr__` | `<ClassName instance>` |
| `str(x)`, `print(x)`| `__str__` | `__repr__`, then default |
| `f"{x}"` (no spec) | `__str__` | same as `str(x)` |
| `f"{x:spec}"` | `__format__` | built-in format spec engine |
| `f"{x!r}"` | `__repr__` | - |

`__format__(spec)` receives the spec string and must return `str`. `int(x)` on an instance calls `__int__`, which is also used by `%d` / `%x` / `%X` / `%o` formatting. `float(x)` calls `__float__` and falls back to `__index__`. `abs(x)` calls `__abs__`.

```python
class P:
  def __init__(self, n):
    self.n = n
  def __repr__(self):
    return f"P({self.n})"

p = P(3)
print(repr(p))
print(str(p))  # falls back to __repr__
print(f"{p!r}")
print([p])  # containers use __repr__
```

```text Output
P(3)
P(3)
P(3)
[P(3)]
```

### Attribute access fallback

`__getattr__(self, name)` runs only when normal lookup (instance dict, then class chain) misses. It receives the name as a string. Return the value, or raise `AttributeError` to surface a real miss.

```python
class Proxy:
  def __init__(self):
    self.real = 1
  def __getattr__(self, name):
    return f"computed:{name}"

p = Proxy()
print(p.real)  # existing attribute, no fallback
print(p.anything)
print(p.foo)
```

```text Output
1
computed:anything
computed:foo
```

Existing attributes bypass `__getattr__`. Only misses trigger it.

### Context managers

`with cm() as x:` invokes `__enter__`. Its return value binds to `as`. On exit, `__exit__(exc_type, exc_value, traceback)` runs. The arguments are `(None, None, None)` on normal exit. On a raise, they carry the exception type and value, with `traceback` always `None`. A truthy return suppresses the exception. A falsy one propagates it.

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

Multiple managers (`with a(), b() as x:`) nest LIFO. `b` enters last and exits first. Each has its own implicit handler, so inner suppression still lets outer managers run their normal `__exit__(None, None, None)`.

If `__exit__` itself raises, the new exception replaces the original.

### What's not dispatched

Parsed for compatibility but never invoked on user classes:

- `__init_subclass__`, `__set_name__`, descriptors (`__get__` / `__set__` / `__delete__`)
- `__new__`. The VM constructs the instance and `__init__` runs user logic.
- Augmented-assignment dunders (`__iadd__`, ...). `a += b` desugars to `a = a + b`, so `__add__` covers it. Exception: list `+=` extends in place (alias-visible). See [Data types](/language/data-types#list).
- Async dunders (`__aenter__` / `__aexit__` / `__aiter__` / `__anext__`). `async with` and `async for` use the sync paths. See [Async](/language/async).

## What classes do not support

- Metaclasses, descriptors (`__get__` / `__set__`), `__slots__`, ABCs, `__init_subclass__`.
- Async dunders, covered above.

Reuse behaviour through free functions and composition by default. Reach for inheritance and operator overloading when the abstraction genuinely calls for them.
