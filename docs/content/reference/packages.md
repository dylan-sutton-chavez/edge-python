---
title: "Official packages"
description: "The ready-made modules maintained alongside Edge Python, what they are, where they live, and how to import them."
---

Edge Python ships no bundled stdlib (see [What it is](/getting-started/what-it-is)). Every module is external. This page is the catalog of the **official, ready-to-use packages** maintained alongside the compiler. You don't write these yourself: import them and go. To build your own, see [Writing modules](/reference/writing-modules).

Two families, matching two of the three [delivery paths](/reference/writing-modules):

| Family | Source | Form | Where it runs | Path |
|---|---|---|---|---|
| **Standard packages** | [`std/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std) | `.wasm` (Rust) | Inside the WASM sandbox, any host | [Path A](/reference/writing-modules#path-a-wasm-module-by-url) |
| **Host libraries** | [`host/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host) | Plain ESM (JS) | Browser main thread | [Path C](/reference/writing-modules#path-c-js-host-module) |

Standard packages are host-agnostic (they run wherever WASM runs). Host libraries are browser-only: they reach `document` / `window` / `localStorage`, surfaces that don't exist in a non-browser host.

## Standard packages

Language-agnostic `.wasm` plugins over the [WASM module ABI](/reference/wasm-abi). Import by bare name (the browser runtime resolves the official ones by default; see [Defaults](#defaults)), by URL, or via a `packages.json` alias. The host fetches the `.wasm` and treats its exports as native bindings.

### `json`

JSON serialization and deserialization, full CPython `json.loads` / `json.dumps` kwargs parity (`object_hook`, `parse_float`, `indent`, `sort_keys`, `ensure_ascii`, `default`, and more).

```python
from json import dumps, loads

data = loads('{"name":"ada","tags":["math","cs"]}')
print(data["name"]) # ada
print(dumps({"k": [1, 2, 3], "ok": True})) # {"k":[1,2,3],"ok":true}
```

Pre-built `.wasm` is served from `https://cdn.edgepython.com/std/json.wasm`. Full API: [`std/json/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/json).

> **`json` is an external `.wasm` package, not built into `compiler.wasm`**. The browser runtime resolves it by [default](#defaults), so `from json import ...` works with no manifest.

### `re`

Regular expressions, a CPython `re` subset on a compact backtracking engine. Unicode aware `\d` `\w` `\s` and `(?i)` without shipping Unicode tables, plus capture groups, backreferences, lookahead, and fixed width lookbehind. A step budget raises `RuntimeError` on catastrophic backtracking instead of hanging, so a degrading pattern is reported rather than freezing the worker.

```python
from re import search, sub, findall

print(search(r'(\d+)-(\d+)', 'order 12-34')) # 12-34
print(sub(r'\s+', '_', 'a  b   c')) # a_b_c
print(findall(r'\w+', 'one two three')) # ['one', 'two', 'three']
```

Functions: `match`, `search`, `fullmatch`, `findall`, `groups`, `span`, `sub`; `compile(pattern)` pre-parses a pattern and exposes the same operations as methods. Flags go inline (`(?i)`, `(?s)`, `(?m)`). Pre-built `.wasm` is served from `https://cdn.edgepython.com/std/re.wasm`. Full API: [`std/re/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/re).

### `struct`

CPython-style `struct` subset: pack primitive values into `bytes` and back. Standard sizes, `<`/`>`/`!`/`=` byte-order prefixes, repeat counts, codes `x b B ? h H i I q Q f d`. The binary fast lane for bulk numeric data — a packed buffer crosses the host boundary once and the receiving side reads it with zero parsing (in JS, straight into a `Float32Array`).

```python
from struct import pack, unpack, calcsize

buf = pack("3f", 92.5, -115.25, 0.75) # 12 bytes exactly
print(unpack("3f", buf)) # [92.5, -115.25, 0.75]
print(calcsize("!hh")) # 4
```

`unpack` returns a list (not a tuple), and there is no native-alignment mode. Full API: [`std/struct/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/struct).

### `math`

CPython-style `math`, scalar transcendentals on `libm` (no platform libc) with CPython domain errors (`sqrt(-1)` raises `ValueError: math domain error`).

- Module constants: `pi`, `e`, `tau`, `inf`, `nan`
- Integer helpers: `factorial`, `gcd`, `lcm`, `isqrt`, `comb`, `perm`
- Tuple returns: `modf`, `frexp`
- Variadic: `hypot` and `gcd`

A packed-f64 batch path (`sqrt_all`, `fsum_all`, and friends) processes a whole `bytes` buffer in one host crossing for bulk work — including binary kernels (`add_all`, `mul_all`, `dot_all`, `matvec`) that pair with `struct` for small numeric training loops.

```python
from math import sqrt, pi, hypot, factorial

print(sqrt(2)) # 1.4142135623730951
print(pi) # 3.141592653589793 (a value, not a call)
print(hypot(3, 4, 12)) # 13.0
print(factorial(5)) # 120
```

Integers are bounded by the VM's `i128`, so `factorial`, `comb`, `perm`, and `lcm` raise `ValueError` past that range. There is no `complex` / `cmath`. Pre-built `.wasm` is served from `https://cdn.edgepython.com/std/math.wasm`. Full API: [`std/math/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/math).

### `test`

A tiny unit-test harness written in pure Edge Python, not a Rust `.wasm` module: fixtures, test registration, exception assertions, and a runner that reports pass/fail and sets the exit code. It leans only on language built-ins (`assert`, `issubclass`, `SystemExit`), so it needs no host capability and runs wherever the VM runs.

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

run() # prints PASS/FAIL lines and a summary, then raises SystemExit(0 if all passed, else 1)
```

`@fixture` registers a `def` under its name and injects it by keyword into the tests that ask for it. `@test(description, *uses)` registers a test plus the fixtures it pulls. `raises(ExcType)` is a context manager asserting the block raises `ExcType` (a subclass, or any type in a tuple). `run()` executes every registered test, prints `PASS` / `FAIL` / `ERROR` and a summary, then raises `SystemExit(1 if any failed, else 0)` so a host can read the result as a process exit code. [`edge test`](/reference/cli) drives `run()` for you across every `*_test.py` file.

Unlike the other standard packages, `test` ships as **pure Edge Python source** (`src/entry.py`), not a compiled `.wasm`, so there is no `cargo` build. It is served from `https://cdn.edgepython.com/std/test.py`. The browser runtime resolves it by default, importing the `.py` directly (see [Defaults](#defaults)). Full API: [`std/test/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/test).

## Host libraries

Plain-JS capabilities that run on the browser's main thread. Register them:

- declaratively via the `host` field of [`packages.json`](/reference/imports#packagesjson) (with the `<edge-python>` element)
- programmatically via `createWorker({ hostModules })`
- by default with no config at all (see [Defaults](#defaults))

No `.wasm`, no Rust, no build step. Each call defers to the main thread over `postMessage` (around 0.1 to 0.4 ms). Python sees a synchronous call. The ESM loads lazily, the first time a run imports it.

### `dom`

Full browser DOM surface: queries, mutation, events, forms, files, observers, animations, layout, media, SVG, dialog, fullscreen, pointer lock.

```python
from dom import query, set_text, bind_event

set_text(query("#title"), "Hello")
bind_event(query("#btn"), "click", "clicked")
```

Representative handlers: `query`, `query_all`, `create_element`, `append_child`, `set_text`, `set_html`, `set_attribute`, `add_class`, `set_style`, `rect`, `bind_event`, `form_data`, `validity`, `get_files`, `file_read_text`, `observe_intersection`, `animate`, `media_play`, `show_modal`. Full API: [`host/dom/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host/dom).

### `network`

HTTP fetch, WebSocket, and Server-Sent Events. HTTP calls suspend the coroutine and compose with `gather` / `with_timeout`. WS/SSE stream events through `receive()`.

```python
from network import fetch_text, ws_open, ws_send

body = fetch_text("https://example.com") # suspends until the response arrives
sock = ws_open("wss://example.com/socket", "ws")
ws_send(sock, "hello")
```

Handlers: `fetch`, `fetch_text`, `fetch_json`, `abort_request`, `ws_open`, `ws_send`, `ws_close`, `ws_state`, `sse_open`, `sse_close`, `sse_state`. Full API: [`host/network/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host/network).

> **Subject to CORS in the browser.** `fetch`, `fetch_text`, and `fetch_json` run the browser's `fetch()` from a Web Worker, so the same-origin policy applies: a cross-origin target must return `Access-Control-Allow-Origin`, or the request is blocked and the call raises (indistinguishable from a network failure). `fetch_text`/`fetch_json` also raise on a non-2xx status. CORS is a browser rule, not an Edge Python one — it doesn't apply on non-browser hosts.

### `storage`

Persistent client-side storage: `localStorage`, `sessionStorage`, `IndexedDB`. KV handlers are synchronous. IndexedDB handlers suspend like `network`'s `fetch`.

```python
from storage import local_set, local_get, idb_open, idb_put

local_set("theme", "dark")
print(local_get("theme")) # dark
db = idb_open("notes", 1, '{"stores":["items"]}')
idb_put(db, "items", "1", '{"title":"hello"}')
```

Handlers: `local_get/set/remove/clear/keys`, `session_*` (same surface), `idb_open`, `idb_put`, `idb_get`, `idb_delete`, `idb_keys`, `idb_close`. Full API: [`host/storage/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host/storage).

### `time`

Wall and monotonic clocks, sleep, and calendar formatting, a sandbox-friendly subset of CPython's `time`. Clock reads are synchronous. `sleep` suspends the coroutine like `network`'s `fetch`, composing with `gather` / `with_timeout`. A `struct_time` crosses as a JSON nine-tuple string. Decode it with `json` to read fields.

```python
from time import time, sleep, strftime, gmtime

print(time()) # seconds since the epoch
sleep(0.1) # suspends, resumes after ~100ms
print(strftime("%Y-%m-%d", gmtime(0))) # 1970-01-01
```

Handlers: `time`, `time_ns`, `monotonic`, `monotonic_ns`, `perf_counter`, `perf_counter_ns`, `sleep`, `gmtime`, `localtime`, `mktime`, `strftime`, `strptime`, `asctime`, `ctime`, `timezone`, `altzone`, `daylight`, `tzname`. CPU, thread, and POSIX clocks (`process_time`, `clock_gettime`, `tzset`, the `CLOCK_*` constants) are intentionally out of scope. Full API: [`host/time/README.md`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host/time).

## How to load them

| You have... | Do |
|---|---|
| Any official package, browser runtime | Just `from <name> import ...`, the runtime resolves the official std/host packages by default. Declare it only to pin a different version, or opt out with `defaults: false` |
| A standard `.wasm` package (e.g., `json`) | Quoted URL `from "https://.../json.wasm" import ...`, or a `packages.json` `imports` alias |
| A host library (e.g., `dom`, `network`, `storage`), `<edge-python>` element | Add it to the `host` field of `packages.json` |
| A host library, programmatic `createWorker` | Pass its URL in `hostModules` (lazy) or an in-memory factory in `mainThreadModules` (eager) |

```json
{
  "imports": { "json": "https://cdn.edgepython.com/std/json.wasm" },
  "host": { "dom": "./dom/src/index.js" }
}
```

One manifest drives both directions: `imports` for worker-side `.py` / `.wasm` modules, `host` for main-thread libraries. See [Imports](/reference/imports) for resolution semantics and the full `packages.json` schema, and the [runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js) for `<edge-python>` attributes and `createWorker` options.

### Defaults

The browser runtime ships a built-in base manifest, so the official packages resolve by bare name with **no `packages.json` at all**: the std packages (`json`, `re`, `math`, `struct`, and the pure-Python `test`) and the host libraries (`dom`, `network`, `storage`, `time`). `dom` resolves to a pure-Python façade over the internal `_dom` host module, adding opt-in mutation batching. Three rules:

- **Lazy.** A default is fetched only when a run actually imports it. Unused defaults never hit the network.
- **Overridable.** Your `packages.json` (or `imports` / `hostModules`) wins for the same name, so you can pin a specific version or URL.
- **Opt-out.** Pass `defaults: false` to `createWorker` to disable the base manifest entirely (e.g. offline or non-browser embedders).

Defaults are a convenience of the browser runtime, not the compiler: `compiler.wasm` stays hermetic and resolves bare names only through the manifest the host provides. Non-browser hosts decide their own defaults, if any.

## See also

- [Imports](/reference/imports), import syntax, `packages.json`, integrity verification.
- [Writing modules](/reference/writing-modules), build your own package (the three delivery paths).
- [WASM module ABI](/reference/wasm-abi), the contract standard `.wasm` packages implement.
