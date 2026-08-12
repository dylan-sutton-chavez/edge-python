---
title: "Async"
description: "Cooperative coroutines: run, sleep, frame, gather, with_timeout, cancel, receive."
---

Concurrency is cooperative. `async def` creates coroutines and the scheduler interleaves them on a single thread. There is no preemption between coroutines. A coroutine runs until it yields, sleeps, awaits, or returns. The host can still force a pause, see [Snapshots](/language/snapshots). Concurrency comes from interleaving, not parallelism.

There is no `asyncio` module. The primitives are top-level builtins: `run`, `sleep`, `frame`, `gather`, `with_timeout`, `cancel`, `receive`.

```python
async def main():
  sleep(0.01)
  return "ok"

print(run(main()))
```

```text Output
ok
```

## Two kinds of callables

A `def` body executes when called. An `async def` body returns a coroutine value that does nothing until driven with `run` or `gather`. Only coroutines are cancellable (`cancel`) and can suspend on real time (`sleep`).

```python
def routine():
  return 1

async def coro():
  return 1

print(routine()) # 1
print(coro()) # <coroutine> (does not run yet)
print(run(coro())) # 1 (run drives it to completion)
```

```text Output
1
<coroutine>
1
```

A plain `def` called from a coroutine can still call yielding builtins like `sleep` and `receive`. The scheduler snapshots the helper's frame, suspends the call chain, and re-enters the helper on resume, so its return value lands at the original call site. The module body runs as an implicit coroutine, so top-level statements suspend the same way.

```python
def helper():
  sleep(0)
  return "from helper"

async def main():
  return helper()

print(run(main()))
```

```text Output
from helper
```

## run

`run(coro)` executes a single coroutine to completion and returns its value.

```python
async def square(n):
  return n * n

print(run(square(5)))
```

```text Output
25
```

`run(c1, c2, ...)` accepts multiple coroutines. They run concurrently and the call returns the first argument's result.

```python
async def a():
  return "first"
async def b():
  return "second"

print(run(a(), b()))
```

```text Output
first
```

## await

Inside an `async def`, `await coro` runs the coroutine to completion and resolves to its value, or re-raises its error. The awaiting coroutine parks while the awaited one sleeps or makes a host call, then resumes with the result.

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

## sleep

`sleep(seconds)` suspends until `seconds` of wall time pass. Without a host time hook, a virtual clock advances logically and coroutines interleave deterministically with no real wait. That is useful for tests.

`sleep(0)` yields to the scheduler without waiting.

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

`gather(*coros)` runs each coroutine concurrently and returns a list of results in argument order. If any coroutine raises, `gather` re-raises after all peers have terminated. Survivors are not auto-cancelled.

```python
async def fetch(name, delay):
  sleep(delay)
  return name + "!"

print(gather(fetch("a", 0.05), fetch("b", 0.02), fetch("c", 0.03)))
```

```text Output
['a!', 'b!', 'c!']
```

The total wall time is `max(delays)`, not the sum. `b` and `c` overlap with `a`'s sleep.

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

Deferred host calls (for example `fetch_text` from the `network` package) run concurrently under `gather`. Each parks its coroutine, the host resolves them in parallel, and every result is routed back to the exact coroutine that issued it. A failed call raises only in its own coroutine, so a `try` / `except` lets the rest of the batch finish.

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

In a browser host, `fetch_text` runs the browser's `fetch()` inside a Web Worker and is subject to CORS. The native engine multiplexes the same calls on its own reactor, so `gather` overlaps them there too, with no CORS. See [network](/packages/host/network).

## with_timeout

`with_timeout(seconds, coro)` runs `coro` and raises `TimeoutError` if the deadline passes first. The coroutine is cancelled on timeout.

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

## cancel

`cancel(coro)` flags a registered coroutine for cancellation. On its next scheduler tick it raises `CancelledError` at the suspension point, runs every enclosing `finally`, and stops. It cannot be caught or suppressed.

A coroutine in a tight synchronous loop without `await` or `sleep` cannot be cancelled until it yields:

```python
async def loop_forever():
  for i in range(1_000_000):
    pass # no yield, not cancellable here
  sleep(0) # cancellable from this point on
```

For deadline-driven cancellation use `with_timeout`.

## frame

`frame()` parks the coroutine until the host's next render frame. Browser embedders hook `requestAnimationFrame`. Use it for animation loops at display refresh rate. It needs a web host. The native CLI has no render frame to wait for.

```python
from dom import set_attribute

async def animate(node):
  for i in range(60):
    set_attribute(node, "style", f"transform: translateX({i}px)")
    frame() # resumes on the next rendered frame
```

## receive

`receive()` pops the oldest message from the host event queue. When the queue is empty it parks the coroutine until the host pushes one (`pushEvent` from JS, `run_push_event` in the ABI). Messages are arbitrary strings, DOM event names from `bind_event` or anything the embedder sends. A parked `receive()` is also a natural pause point for [snapshots](/language/snapshots).

```python
async def main():
  while True:
    msg = receive() # parks until the host pushes an event
    print(f"got {msg}")

run(main())
```

## async for and async with

`async for` works against any `for`-iterable plus coroutines and async generators (an `async def` with `yield`). Each iteration resumes the source to its next yield. Behaviour over lists, tuples, and dicts is identical to regular `for`. There is no `__aiter__` / `__anext__` dispatch on user classes. Write an `async def` generator instead.

```python
async def gen():
  for i in range(3):
    yield i

async def main():
  async for x in gen():
    print(x)
  async for y in [10, 20]:
    print(y)

run(main())
```

```text Output
0
1
2
10
20
```

`async with` reuses the sync dispatch (`__enter__` / `__exit__`). `__aenter__` / `__aexit__` are not consulted. For async setup and teardown, use `try` / `finally` with explicit `await`.

## Exception types

| Exception | When |
|---|---|
| `TimeoutError` | `with_timeout` deadline expired |
| `CancelledError` | raised by `cancel()`, runs `finally` but cannot be caught |

`TimeoutError` matches `except` normally. `CancelledError` subclasses `BaseException`, so `except Exception` does not catch it.

## Limitations

- **No background tasks.** There is no `create_task`. All concurrency is structural: a coroutine only progresses while awaited inside `run(...)` or `gather(...)`.
- **No preemption between coroutines.** `while True: pass` inside a coroutine blocks the scheduler. The host can still force a pause via the preempt interval, see [Snapshots](/language/snapshots).
- **No suspending in a cancelled `finally`.** A `finally` running from `cancel()` cannot `await` or `sleep`, doing so raises `RuntimeError`.
- **Cooperative host loop.** The scheduler suspends to the host when it cannot progress synchronously (pending timer, frame, or event). The embedder resumes via `run_start` / `run_resume` / `run_push_event`, and can serialize a parked run with `save_state` / `restore_state`. See [Snapshots](/language/snapshots) and the [ABI](/reference/abi).
- **No async comprehensions.** `[x async for x in it]` is a parse error.
- **No `gen.send` / `throw` / `close`.** Generators and coroutines are one-way producers. For bidirectional flow, use `run` / `gather` and pass messages via arguments.
- **`receive()` can park indefinitely.** An empty queue with no `run_push_event` leaves the coroutine waiting. Pair with `with_timeout` for a deadline.

## Time

The scheduler reads wall time from a host hook. WASM hosts wire it to `Date.now()` via the `host_now_ns` import. Native hosts use `std::time::Instant`. Without a hook, `sleep` advances a virtual clock so deterministic tests interleave correctly.

To run many of these programs side by side as message-passing tasks, see [Workers](/reference/workers).
