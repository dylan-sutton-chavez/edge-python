---
title: "Async"
description: "Cooperative coroutines: run, sleep, frame, gather, with_timeout, cancel, receive."
---

Cooperative concurrency via `async def` coroutines and `await` / `yield`. No preemption: a coroutine runs until it yields, sleeps, awaits, or returns. The scheduler is single-threaded. Concurrency comes from interleaving, not parallelism.

No `asyncio` module. These primitives are top-level builtins: `run`, `sleep`, `frame`, `gather`, `with_timeout`, `cancel`, `receive`.

```python
import asyncio # compile-time error: module 'asyncio' not found, concurrency lives in the VM, not a module.
```

```python
# Idiomatic edge-python: call the primitives directly.
async def main():
  sleep(0.01)
  return "ok"

print(run(main()))
```

```text Output
ok
```

## Two kinds of callables

A `def` body executes immediately. An `async def` body returns a coroutine value that does nothing until driven with `run` / `gather`. Only coroutines are cancellable (`cancel`) and can suspend on real time (`sleep`).

A plain `def` inside a coroutine (or at module top-level) can still call yielding builtins (`sleep`, `receive`, deferred host calls). The scheduler snapshots the helper's frame, suspends the call chain, and re-enters the helper on resume, so its return value lands at the original call site. The module body runs as an implicit coroutine, so top-level statements suspend the same way. From the caller, a sync helper that internally sleeps is indistinguishable from one that doesn't.

```python
def routine():
  return 1

async def coro():
  return 1

print(routine()) # 1
print(coro()) # <coroutine> (does not run yet)
print(run(coro())) # 1 (run drives it to completion)
```

## Driving coroutines

`run(coro)` executes a single coroutine to completion and returns its value.

```python
async def square(n):
  return n * n

print(run(square(5)))
```

```text Output
25
```

`run(c1, c2, ...)` accepts multiple coroutines. They run concurrently. The call returns the first argument's result.

## await

Inside an `async def`, `await coro` runs the coroutine to completion and resolves to its value (or re-raises its error). It works across suspension: the awaiting coroutine parks while the awaited one sleeps or makes a host call, then resumes with the result. Multiple awaits compose in one expression.

```python
async def fetch(n):
  sleep(0) # suspends, then resumes
  return n * 10

async def main():
  a = await fetch(1)
  return a + await fetch(2)

print(run(main()))
```

```text Output
30
```

## Sleeping

`sleep(seconds)` suspends until `seconds` of wall time pass. Without a host time hook, a virtual clock advances logically. Coroutines interleave deterministically with no real wait (useful for tests).

```python
async def task(name):
  print(f"{name} step 1")
  sleep(0) # yield to the scheduler
  print(f"{name} step 2")

run(task("a"), task("b"))
```

```text Output
a step 1
b step 1
a step 2
b step 2
```

## gather

`gather(*coros)` runs each concurrently and returns a list of results in argument order. If any raises, an error propagates after all peers terminate — currently the last one raised, not necessarily the first in argument order. Survivors are not auto-cancelled.

```python
async def fetch(name, delay):
  sleep(delay)
  return name + "!"

print(gather(fetch("a", 0.05), fetch("b", 0.02), fetch("c", 0.03)))
```

```text Output
['a!', 'b!', 'c!']
```

The total wall time is `max(delays)`, not the sum: `b` and `c` overlap with `a`'s sleep.

### Errors

```python
async def good(): return 1
async def bad():  raise ValueError

try:
  gather(good(), bad())
except ValueError:
  print("caught")
```

```text Output
caught
```

### Concurrent host calls

Deferred host calls (e.g. `network.fetch`) run concurrently under `gather`. Each parks its coroutine, the host resolves them in parallel, and every result is routed back to the exact coroutine that issued it. A failed call raises only in its own coroutine, so a `try/except` lets the rest of the batch finish.

```python
from network import fetch_text

async def status(url):
  try:
    fetch_text(url)
    return "ok"
  except:
    return "failed"

# The bad URL raises inside its own coroutine; the others still resolve.
print(gather(status("https://api.github.com/zen"), status("https://nope.invalid/x")))
```

```text Output
['ok', 'failed']
```

> In a browser host, `fetch_text` runs the browser's `fetch()` inside a Web Worker, so it is subject to [CORS](/reference/packages#network): the target must send `Access-Control-Allow-Origin`. `api.github.com` does; many hosts (e.g. `example.com`) don't, and a blocked request raises just like a network failure.

## with_timeout

`with_timeout(seconds, coro)` runs `coro`, or raises `TimeoutError` on deadline. The coro is cancelled on timeout.

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

`with_timeout` evaluates the coroutine eagerly: it's a call, not an awaitable.

## cancel

`cancel(coro)` flags a registered coroutine for cancellation. On its next scheduler tick it transitions to `Cancelled` and stops. The body does not observe a `CancelledError`. Cancellation is cooperative and silent.

A coroutine in a tight synchronous loop without `await`/`sleep` cannot be cancelled until it yields:

```python
async def loop_forever():
  for i in range(1_000_000):
    pass # no yield, not cancellable here
  sleep(0) # cancellable from this point on
```

For deadline-driven cancellation use `with_timeout`.

## Exception types

| Exception | When |
|---|---|
| `TimeoutError` | `with_timeout` deadline expired |
| `CancelledError` | reserved for user-thrown; not auto-raised by `cancel()` |

Both live in the built-in exception namespace and match `except` clauses normally.

## Limitations

* **No preemption**, `while True: pass` inside a coroutine blocks the scheduler.
* **Silent cancellation**, `cancel(coro)` stops the coro; the body doesn't see `CancelledError`. Use `with_timeout` for deadline-as-exception.
* **Cooperative host loop**, the scheduler suspends to the host when it can't progress synchronously (pending timer/frame/event). The embedder resumes via `run_start` / `run_resume` / `run_push_event`, and can serialize a parked run for later with `save_state` / `restore_state` (see [Snapshots](/language/snapshots)). The legacy non-suspending `run` cannot resume. Code using `sleep(n>0)`, `frame()`, or an empty `receive()` must run via the driver loop. Statements after a top-level `run()` don't execute after a yield.
* **`async for`** works against any `for`-iterable plus coroutines and async generators (`async def` with `yield`). Each iteration resumes to the next yield. No `__aiter__` / `__anext__` dispatch on user classes. Write an `async def` generator instead. Behaviour over lists/tuples/dicts is identical to regular `for`.
* **`async with`** reuses sync dispatch (`__enter__` / `__exit__`). `__aenter__` / `__aexit__` aren't consulted. For async setup/teardown, use `try` / `finally` with explicit `await`.
* **No async comprehensions**, `[x async for x in it]` unsupported.
* **No `gen.send` / `throw` / `close`**, generators and coroutines are one-way producers. For bidirectional flow, use `run` / `gather` and pass messages via args.
* **`receive()` blocks indefinitely**, empty queue + no `run_push_event` leaves the coro parked in `WaitingEvent`. Pair with `with_timeout` for a deadline.

## Time capability

The scheduler reads from `vm.time_hook`. WASM hosts wire it to `Date.now() * 1e6` via the `host_now_ns` import. Native hosts use `std::time::Instant`. Without a hook, `sleep` advances a virtual clock so deterministic tests interleave correctly.
