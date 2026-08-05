---
title: "Functions"
description: "First-class functions, lambdas, closures, generators, decorators."
---

Functions are values. Pass them, return them, store them, compose them.

## def

```python
def add(a, b):
  return a + b

print(add(3, 4))
```

```text Output
7
```

### Default arguments

```python
def greet(name, greeting="Hello"):
  return f"{greeting}, {name}!"

print(greet("world"))
print(greet("world", "Hi"))
```

```text Output
Hello, world!
Hi, world!
```

### Keyword arguments

```python
def f(x, y, z):
  return x * 100 + y * 10 + z

print(f(1, 2, 3))
print(f(x=1, z=3, y=2))
print(f(1, z=3, y=2))
```

```text Output
123
123
123
```

### Variadic parameters

`*args` collects extra positional arguments into a tuple. `**kwargs` collects extra keyword arguments into a dict.

```python
def total(*nums):
  return sum(nums)

print(total(1, 2, 3))
print(total(*[10, 20, 30]))
```

```text Output
6
60
```

```python
def opts(**kwargs):
  return sorted(kwargs.items())

print(opts(host="api", port=443))
```

```text Output
[('host', 'api'), ('port', 443)]
```

### Keyword-only parameters

A bare `*` marks the following parameters as keyword-only. They must be passed by name. A positional argument that would reach them raises `TypeError`, as does any positional argument beyond the declared parameters when there is no `*args`.

```python
def connect(host, *, port=80, secure=False):
  return f"{host}:{port} secure={secure}"

print(connect("api"))
print(connect("api", port=443, secure=True))

try:
  connect("api", 443) # positional can't fill a keyword-only param
except TypeError:
  print("rejected")
```

```text Output
api:80 secure=False
api:443 secure=True
rejected
```

### Argument unpacking at the call site

```python
def f(a, b, c):
  return a + b + c

print(f(*[1, 2, 3]))
print(f(*[1, 2], 3))
print(f(**{"a": 1, "b": 2, "c": 3}))
print(f(1, **{"b": 2, "c": 3}))
```

```text Output
6
6
6
6
```

## lambda

An anonymous function. The body is a single expression.

```python
double = lambda x: x * 2
print(double(21))

add = lambda a, b: a + b
print(add(3, 4))

# With defaults
greet = lambda name, msg="Hi": f"{msg}, {name}"
print(greet("world"))
```

```text Output
42
7
Hi, world
```

## First-class functions

Store functions in data structures and pass them as arguments.

```python
ops = [abs, hex, str]
print([f(-3) for f in ops])
```

```text Output
[3, '-0x3', '-3']
```

```python
# Functions as dict values; replaces switch/case
handlers = {
  "add": lambda a, b: a + b,
  "mul": lambda a, b: a * b,
  "max": max,
}

print(handlers["add"](3, 4))
print(handlers["mul"](3, 4))
print(handlers["max"](3, 4))
```

```text Output
7
12
4
```

## Function attributes

Functions carry writable attributes, like any object. `getattr` / `hasattr` / `setattr` / `delattr` work on them, and an assigned `__name__` wins over the declared one. Decorators use these for metadata.

```python
def sma(source, length):
  return sum(source) / length

sma.window = 10
print(sma.window)
print(getattr(sma, "missing", "n/a"))

def tag(fn):
  fn.tagged = True
  return fn

@tag
def step():
  pass
print(step.tagged)
```

```text Output
10
n/a
True
```

## Higher-order functions

Functions that take or return functions.

```python
def apply(f, x):
  return f(x)

print(apply(lambda n: n * n, 5))
print(apply(abs, -10))
```

```text Output
25
10
```

```python
# Returning a function
def make_adder(n):
  return lambda x: x + n

add5 = make_adder(5)
add10 = make_adder(10)

print(add5(3))
print(add10(3))
```

```text Output
8
13
```

## Closures

An inner function captures the variables of its enclosing scope by reference.

```python
def counter():
  count = 0
  def step():
    nonlocal count
    count += 1
    return count
  return step

tick = counter()
print(tick())
print(tick())
print(tick())
```

```text Output
1
2
3
```

Because capture is by reference, every lambda in a loop sees the same loop variable. Bind the current value through a default argument.

```python
def make_adders(n):
  return [lambda x, i=i: x + i for i in range(n)]

add0, add1, add2 = make_adders(3)
print(add0(10), add1(10), add2(10))
```

```text Output
10 11 12
```

### global and nonlocal

Assignment inside a function creates a local unless declared otherwise. `nonlocal name` rebinds the nearest enclosing function's variable. That is the shared cell the `counter` closure above relies on. `global name` rebinds the module-level variable.

```python
total = 0

def bump(n):
  global total
  total += n

bump(3)
bump(4)
print(total)
```

```text Output
7
```

Reading an outer variable needs no declaration. Only rebinding does.

```python
BASE = 10

def add_base(n):
  return n + BASE

print(add_base(5))
```

```text Output
15
```

## Recursion

```python
def factorial(n):
  if n < 2:
    return 1
  return n * factorial(n - 1)

print(factorial(10))
```

```text Output
3628800
```

```python
# Mutual recursion
def is_even(n):
  return True if n == 0 else is_odd(n - 1)

def is_odd(n):
  return False if n == 0 else is_even(n - 1)

print(is_even(10), is_odd(10))
```

```text Output
True False
```

<Note>
Pure functions are memoized after two calls with the same arguments. The VM detects purity statically (no I/O, no mutation, no raise, no yield) and confirms it at runtime. Any call that performs a side effect, including a builtin like `print` passed as a first-class value, marks the call impure and skips the cache, so memoization never drops an effect. Naive recursion runs at memoized cost with no source changes. See [Design](/implementation/design) for the full model.
</Note>

## Generators

A function containing `yield` produces its sequence lazily. Pull values with `next()` or iterate with `for`.

```python
def squares(n):
  for i in range(n):
    yield i * i

for x in squares(5):
  print(x)
```

```text Output
0
1
4
9
16
```

```python
def gen():
  yield 1
  yield 2

g = gen()
print(next(g))
print(next(g))
```

```text Output
1
2
```

```python
# Materialize a generator
def naturals(limit):
  n = 1
  while n <= limit:
    yield n
    n += 1

print(list(naturals(5)))
```

```text Output
[1, 2, 3, 4, 5]
```

### yield from

Delegate to another generator or any iterable.

```python
def nums():
  yield from range(3)
  yield from [10, 20]

print(list(nums()))
```

```text Output
[0, 1, 2, 10, 20]
```

`yield from` is also an expression. It evaluates to the subgenerator's return value, so a generator can pass a result back to its delegating caller.

```python
def sub():
  yield 1
  yield 2
  return 'done'

def outer():
  result = yield from sub()
  print('returned', result)

print(list(outer()))
```

```text Output
returned done
[1, 2]
```

<Note>
Generators are one-way producers. `gen.send(value)`, `gen.throw(exc)`, and `gen.close()` are not exposed. For bidirectional flow, use the [cooperative scheduler](/language/async) and pass values through arguments and return values.
</Note>

## Generator expressions

A generator inline.

```python
print(sum(x * x for x in range(5)))
print(max(i for i in [3, 1, 4, 1, 5]))
```

```text Output
30
5
```

## Decorators

A decorator wraps another callable. It applies to both functions and classes (see [Classes](/language/classes#class-decorators)).

```python
def trace(f):
  def wrapped(*args):
    print(f"calling with {args}")
    return f(*args)
  return wrapped

@trace
def add(a, b):
  return a + b

print(add(3, 4))
```

```text Output
calling with (3, 4)
7
```

Stacked decorators apply bottom-up.

```python
def double_result(f):
  return lambda *a: f(*a) * 2

def add_one(f):
  return lambda *a: f(*a) + 1

@double_result
@add_one
def base(x):
  return x

# base(5) -> add_one -> 6 -> double_result -> 12
print(base(5))
```

```text Output
12
```

A parameterized decorator is a factory. The outer function takes the decorator arguments and returns the actual decorator. The wrapped function captures both scopes.

```python
def repeat(n):
  def decorator(fn):
    def wrapped(x):
      for i in range(n):
        fn(x)
    return wrapped
  return decorator

@repeat(3)
def greet(name):
  print(f"hi {name}")

greet("world")
```

```text Output
hi world
hi world
hi world
```
