---
title: "Snapshots"
description: "Freeze a paused program to a portable blob and resume it later, anywhere."
---

A paused program can be frozen whole and brought back to life later, even on a different page load, machine, or day. `saveState` captures the entire interpreter in a portable blob that holds the heap, every global, each suspended coroutine with its call frames, and the scheduler. `restoreState` boots a fresh VM, pours that state back in, and the program continues from exactly where it stopped, as if it had never paused.

The blob embeds the source and a structural fingerprint of the compiled bytecode, so `restoreState` re-parses and verifies both, then rejects any blob that belongs to a different program or a different compiler build. One blob restores any number of times, each an independent copy.

## Where a run can freeze

A run freezes at a suspension point, and there are two ways to reach one.

The program can reach it on its own: an empty `receive()`, a `sleep(n>0)`, a `frame()`, or a pending host call (see [Async](/language/async)). At those points the VM unwinds to a clean serializable state and stays there until the host wakes it.

The host can also force one. With `setPreemptInterval(n)` the VM yields every `n` loop back-edges, so `pause()` stops a program that has no suspension point at all, including a bare `while True:` that only counts. The program needs no cooperation and no rewriting. See [Pause from the host](#pause-from-the-host).

Either way `saveState` rejects while the run is executing, because a snapshot is only coherent at a suspension point.

## What cannot be preempted

A preempt lands at a loop back-edge and only where the VM can unwind, so three cases keep running past their interval and yield at the next reachable back-edge instead.

A class body and an imported module's init are not preemptible, so `while` loops there run to the end (the entry script's top level runs as the module-body coroutine and preempts normally). Neither is user code a builtin re-enters: a `sort(key=...)` or `sorted(key=...)` callback, a generator body being drained by `list()` or `sum()`, and the `__init__` / `__call__` that run while an object is being constructed or invoked. And a single long native operation has no back-edge inside it, so sorting a huge list or `"x" * 10**8` runs to completion before the next yield.

Everything else preempts, including function and method bodies at any call depth, `for` and `while` loops, comprehensions, and loops inside `try` or `with` blocks.

None of this changes results. An unpreemptible stretch delays the pause; it never corrupts one.

## A program that pauses

A shopping cart that accumulates state across events and parks on `receive()` every loop.

```python
items = []
total = 0
while True:
    msg = receive() # parks here until the host pushes an event
    if msg == "checkout":
        break
    price = int(msg)
    items.append(price)
    total += price
    print(f"added {price}, total {total}")
print(f"done: {len(items)} items, {total} total")
```

Here `items` and `total` live in the VM heap, and the only freeze point is the `receive()` line.

## Save, persist, restore

The round trip runs from the host through [`createWorker`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/runtime#api).

```js
import { createWorker } from "https://cdn.edgepython.com/runtime/src/index.js";

// Wait until the VM is parked on an event so saveState() can capture it.
async function untilParked(worker) {
  for (let i = 0; i < 200; i++) {
    const stack = await worker.stateStack();
    if (stack.some((c) => c.state === "waiting_event")) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error("run never parked");
}

// Session 1 starts the program and adds a couple of items.
const worker = await createWorker();
worker.onOutput((chunk) => console.log(chunk));

worker.run(cartSrc); // never awaited, it parks on receive() and stays pending
await untilParked(worker);
worker.pushEvent("10"); // added 10, total 10
await untilParked(worker);
worker.pushEvent("25"); // added 25, total 35
await untilParked(worker);

const blob = await worker.saveState(); // Uint8Array holding the whole VM
worker.dispose(); // user closes the tab, run() is abandoned
```

```js
// Session 2 is a fresh page load that resumes where the user left off.
const worker = await createWorker();
worker.onOutput((chunk) => console.log(chunk));

const done = worker.restoreState(blob); // comes back parked on receive() with total 35
await untilParked(worker);
worker.pushEvent("5"); // added 5, total 40, continued from 35 not 0
worker.pushEvent("checkout"); // done: 3 items, 40 total
const { out } = await done; // resolves like run() once the program finishes
```

The whole point shows in that last total of **40** rather than 0, because the snapshot restored the heap and the program continued instead of restarting.

## Pause from the host

`setPreemptInterval(n)` makes the VM yield every `n` loop back-edges. Nothing else changes: the yields are invisible, the run continues on its own, and the host only notices when it asks to stop. `pause()` then resolves once the run is actually parked, and `resume()` lets it go again.

```python
# counter.py, no suspension point anywhere
n = 0
while True:
    n += 1
    if n % 1_000_000 == 0:
        print(n)
```

```js
const worker = await createWorker();
await worker.setPreemptInterval(50_000); // ~1 yield per 50k loop iterations

worker.run(counterSrc); // never awaited, it runs forever

await worker.pause(); // resolves true once parked, mid-loop
const blob = await worker.saveState();
await worker.stateGlobals(); // { n: "3170000" }
worker.resume(); // keeps counting from 3170000
```

Pick `n` for how fast you need a pause to land. Each yield is one event-loop round trip, which browsers clamp to a few milliseconds, so the interval decides how many you pay: a tight `i = i + 1` loop runs about a million iterations per second, so `n = 1_000` yields every millisecond of work and costs several times the runtime, while `n = 1_000_000` yields about once a second and costs almost nothing. `0` turns preemption off and leaves the program cooperative-only, which is the default.

## Save when the tab closes

The tag spins up the worker and exposes it on `el.worker`; everything else is calls on that.

```html
<edge-python></edge-python>

<script type="module">
  import "https://cdn.edgepython.com/runtime/src/element.js";

  const el = document.querySelector("edge-python");
  const store = await caches.open("edge-python");
  const KEY = "/session";

  el.addEventListener("ready", async () => {
    const worker = el.worker;
    await worker.setPreemptInterval(50000);
    const hit = await store.match(KEY);
    hit
      ? worker.restoreState(new Uint8Array(await hit.arrayBuffer()))
      : worker.run(await fetch("./counter.py").then((r) => r.text()));

    const checkpoint = async () => {
      if (!await worker.pause()) return; // the run already finished
      await store.put(KEY, new Response(await worker.saveState()));
      worker.resume();
    };

    setInterval(checkpoint, 5000);
    addEventListener("visibilitychange", () => document.hidden && checkpoint());
  });
</script>
```

The interval is what actually protects the session. `visibilitychange` is the last event that reliably fires on mobile, but the handler is async and the browser can kill the page before the write lands, so the rolling checkpoint is the guarantee and the handler only shortens what a close costs.

## Download it to a file

`saveState` returns a `Uint8Array` of opaque bytes, which the browser can offer as a download.

```js
const blob = await worker.saveState();
const file = new Blob([blob], { type: "application/octet-stream" });
const url = URL.createObjectURL(file);
Object.assign(document.createElement("a"), { href: url, download: "cart.snapshot" }).click();
URL.revokeObjectURL(url);
```

Reload it from a file the user picks.

```js
const bytes = new Uint8Array(await fileInput.files[0].arrayBuffer());
worker.restoreState(bytes);
```

Any other store works the same way, because IndexedDB keeps a `Uint8Array` directly and a server accepts the raw bytes.

```js
await store.put("cart", await worker.saveState()); // IndexedDB
await fetch("/saves/cart", { method: "PUT", body: await worker.saveState() }); // server
```

## The same blob from the CLI

The [native engine](/reference/native#run-flags) works with snapshots as plain files, no JS involved: `edge run --save-state cart.bin` writes the blob when the script parks on a wait the CLI cannot serve, and `edge run --restore-state cart.bin` boots from it and keeps running. `--preempt <n>` is `setPreemptInterval(n)` as a flag, and `--events <f>` feeds `receive()` line by line. The blob layout is the same one `saveState` produces here.

## Restore if present, else start fresh

The common pattern resumes a saved session when one exists and otherwise begins a new run. A backend works the same way as a local store: it only holds bytes and never runs the VM.

```js
const res = await fetch(`/saves/${userId}`); // or any local store
if (res.ok) {
  const blob = new Uint8Array(await res.arrayBuffer());
  worker.restoreState(blob); // continue the user's held session
} else {
  worker.run(cartSrc); // no save yet, so start fresh
}
```

## Inspect without resuming

`stateGlobals` and `stateStack` read a parked run without waking it, which suits a "resume?" screen or debugging a restored blob.

```js
await worker.stateStack();
// [{ state: "waiting_event", function: "<module>", ip: 12, frames: [] }]

await worker.stateGlobals();
// { items: "[10, 25]", total: "35" }   values are reprs
```

The `state` field reads `"waiting_event"` for a parked `receive()`, `"sleeping"` for a `sleep()`, and so on. A run held by `pause()` reads `"ready"`, because a preempted coroutine is not waiting on anything, it is simply not being stepped.

## Limits

Pure Python state restores identically, while live host resources (DOM handles, sockets, pending host calls) are not captured and must be recreated after restoring. The blob also carries the whole heap, so it must fit the runtime's 1 MiB buffer to restore ([blob layout](/reference/wasm-abi#snapshot-exports)).

## See also

- [Runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/runtime#state-snapshots) for the worker and element API and the size limit.
- [Design](/implementation/design#snapshots) for the serializer internals.
- [WASM module ABI](/reference/wasm-abi#snapshot-exports) for the `compiler.wasm` exports and blob layout.
