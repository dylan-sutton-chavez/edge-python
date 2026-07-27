# Edge Python Test

`test` package for [Edge Python](https://edgepython.com): a tiny test harness written in pure Edge Python (not a wasm native module). Fixtures, test registration, exception assertions, and a runner that reports pass/fail and exits with a status code.

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

run()  # prints PASS/FAIL lines and a summary, then raises SystemExit(0 if all passed else 1)
```

## Surface

| Name | Signature | Purpose |
|------|-----------|---------|
| `fixture` | `@fixture` on `def` | Register a fixture under its `__name__`; injected by keyword into tests that name it. |
| `test` | `@test(description, *uses)` | Register a test; `uses` names fixtures passed in fresh per run. |
| `raises` | `with raises(ExcType):` | Context manager asserting the block raises `ExcType` (or a subclass): swallows it, re-raises a mismatch, and fails if nothing is raised. |
| `run` | `run()` | Run every registered test, print `PASS`/`FAIL`/`ERROR` lines and a summary, then `raise SystemExit(1 if any failed else 0)`. |

A failing `assert` is reported as `FAIL`; any other exception as `ERROR` with its class name. Fixtures are instantiated fresh for each test (no caching across tests). `raises` accepts a class or a tuple of classes.

## Exit code

`run()` raises `SystemExit` so a host can read pass/fail as a process exit code (`0` all passed, `1` otherwise). `SystemExit` is caught by `except SystemExit` / `except BaseException` / a bare `except`, but not by `except Exception`. See [Limits and errors](https://edgepython.com/reference/limits-and-errors).

## Limitations

- Flat fixtures: no autouse, scopes, parametrize, or teardown.
- `raises` matches by class (built-in hierarchy or single-level user inheritance); no message-pattern matching.
- A single module-level registry, so `run()` executes every test defined in the program.

## Distribution

Pure Edge Python source (no `cargo` build); the package entry is `src/entry.py`, imported as a code module via `packages.json`. This differs from the wasm packages (`re`, `math`, `json`), whose entry is `src/lib.rs` built to `<pkg>.wasm`.

License: Apache-2.0
