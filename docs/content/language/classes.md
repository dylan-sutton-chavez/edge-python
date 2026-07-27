---
title: "Classes"
description: "User-defined classes as state machines and library namespaces."
---

Classes are state containers and namespaces, not the primary abstraction. This is a design choice for the compiler's purpose. Two patterns:

- State machines: a few methods that mutate the receiver.
- Namespaces: a bundle of related functions and constants.

Supported:

- Single and multiple inheritance (C3 MRO) with `super()`.
- `@property` / `@x.setter`.
- `@staticmethod` and `@classmethod`.
- A curated dunder protocol: operators, indexing, iteration, hashing, context managers, attribute fallback (see [Dunder methods](/language/dunders)).

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

Single or multiple bases (`class Sub(Base):`, `class C(A, B):`). Methods not on the subclass resolve along the C3 linearization (the MRO), the same order CPython uses; an inconsistent hierarchy raises `TypeError` at class creation. `isinstance(x, Base)` walks the ancestor chain, so `Sub` instances are also instances of every ancestor.

`super()` (zero-arg) delegates to the next class up the chain, bound to current `self`. Most common in `__init__` to extend a base constructor.

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

## Attribute access on classes vs instances

| Access form | Resolves to |
|---------------------|-------------------------------------------|
| `MyClass.attr` | class member, returned as-is (no binding) |
| `MyClass.method()` | method called directly, no `self` |
| `instance.attr` | instance `__dict__` first, then class |
| `instance.method()` | bound method, `self` prepended |

`setattr` / `delattr` work on instances and on class objects (the latter mutating the class's members).

## Class decorators

A class decorator is called with the class object; its return binds to the name. It can add or replace class attributes (`cls.kind = ...`) or return a replacement.

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

`@property` turns a method into a read-only attribute. `@x.setter` (via `property.setter`) makes it writable. Properties live on the class. Subclasses inherit and can override either side.

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

Two-arg form `property(fget, fset)` also works without decorator syntax.

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

Functional form `staticmethod(func)` also works without decorator syntax.

## Class methods

`@classmethod` binds the class — not the instance — as the first argument. Accessed through a subclass, `cls` is the subclass, so alternate constructors return the right type down the hierarchy.

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

Functional form `classmethod(func)` also works without decorator syntax.

## Operator overloading and protocols

Operators, indexing, iteration, context managers, hashing, `repr` / `str` / `format` all dispatch through dunders: `__add__` for `+`, `__eq__` for `==`, `__getitem__` for `x[i]`, `__iter__` / `__next__` for `for`, `__enter__` / `__exit__` for `with`, etc.

```python
class Vec:
  def __init__(self, x, y):
    self.x, self.y = x, y
  def __add__(self, other):
    return Vec(self.x + other.x, self.y + other.y)

v = Vec(1, 2) + Vec(3, 4)
print(v.x, v.y)
```

```text Output
4 6
```

See [Dunder methods](/language/dunders) for the full matrix.

## What is not supported

* Metaclasses, descriptors (`__get__` / `__set__`), `__slots__`, ABCs, `__init_subclass__`.
* Async dunders; see [Dunders, What's not dispatched](/language/dunders#whats-not-dispatched).

Reuse behaviour through free functions and composition by default. Dispatch is fast and aligns with the multi-paradigm identity. Reach for inheritance and operator overloading when the abstraction genuinely calls for them.
