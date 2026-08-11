---
title: "test (web, native)"
description: "A unit-test harness written in pure Edge Python."
---

`test` is a unit-test harness written in pure Edge Python, not a compiled plugin. Import it by bare name, both runtimes resolve it with no manifest. To pin a different version, use a `packages.json` alias, see [Modules](/reference/modules#packagesjson).

`@fixture` registers a factory under its function's name. `@test(description, *uses)` registers a test and the fixtures to inject by keyword. `raises(ExcType)` is a context manager asserting the block raises, and it accepts a class or a tuple of classes. `run()` executes every registered test, prints a `PASS -`, `FAIL -`, or `ERROR` line per test plus a summary, and raises `SystemExit(1)` if anything failed, else `SystemExit(0)`. Fixtures are flat and built fresh per test, with no scopes, autouse, or parametrization. One module-level registry holds every test, so `run()` executes all of them.

```python
from test import fixture, test, raises, run

@fixture
def user():
    return {"name": "Ana"}

@test("user has a name", "user")
def test_name(user):
    assert user["name"] == "Ana"

@test("division by zero raises")
def test_div():
    with raises(ZeroDivisionError):
        1 / 0

run()
```

```text Output
PASS - user has a name
PASS - division by zero raises
2 passed, 0 failed
```

Fixtures are built fresh per test, so one test cannot leak state into another:

```python
from test import fixture, test, run

@fixture
def items():
    return []

@test("first sees an empty list", "items")
def test_first(items):
    assert items == []

@test("second also sees an empty list", "items")
def test_second(items):
    items.append(1)
    assert len(items) == 1

run()
```

```text Output
PASS - first sees an empty list
PASS - second also sees an empty list
2 passed, 0 failed
```

A failed assertion prints a `FAIL -` line with the error, and `run()` exits nonzero:

```python
from test import test, run

@test("two plus two")
def test_add():
    assert 2 + 2 == 4

@test("this one fails")
def test_fail():
    assert "a" in "zzz", "letter missing"

run()
```

```text Output
PASS - two plus two
FAIL - this one fails (AssertionError: letter missing)
1 passed, 1 failed
```

[`edge test`](/reference/cli#edge-test-test-runner) discovers `*_test.py` files and drives `run()` for you, reading the verdict from the exit code.
