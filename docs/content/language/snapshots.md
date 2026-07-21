---
title: "Snapshots"
description: "Freeze a paused program to a portable blob and resume it later, anywhere."
---

A paused program can be frozen whole and brought back to life later, even on a different page load, machine, or day. `saveState` captures the entire interpreter in a portable blob that holds the heap, every global, each suspended coroutine with its call frames, and the scheduler. `restoreState` boots a fresh VM, pours that state back in, and the program continues from exactly where it stopped, as if it had never paused.

The blob embeds the source and a structural fingerprint of the compiled bytecode, so `restoreState` re-parses and verifies both, then rejects any blob that belongs to a different program or a different compiler build. One blob restores any number of times, each an independent copy.

## Where a run can freeze

The program decides where it can be frozen, never the host. A snapshot is only possible while the run is parked at a suspension point the code itself reached, meaning an empty `receive()`, a `sleep(n>0)`, a `frame()`, or a pending host call (see [Async](/language/async)). At those points the VM unwinds to a clean serializable state. There is no preemption, so a program with no such yield point can never be snapshotted, and `saveState` rejects whenever nothing is parked.

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

## Restore if present, else start fresh

The common pattern resumes a saved session when one exists and otherwise begins a new run.

```js
const saved = await store.get("cart"); // Uint8Array or undefined
if (saved) {
  worker.restoreState(saved); // resume exactly where the user left off
} else {
  worker.run(cartSrc); // brand-new session
}
```

## Serve a snapshot

A backend can hand a client a blob and let it continue locally, so the server only stores bytes and never runs the VM.

```js
const res = await fetch(`/saves/${userId}`);
if (res.ok) {
  const blob = new Uint8Array(await res.arrayBuffer());
  worker.restoreState(blob); // continue the user's server-held session
} else {
  worker.run(cartSrc); // no save yet, so start fresh
}
```

A blob is program- and build-pinned, so a snapshot served to a client on a different compiler build is rejected cleanly rather than silently misinterpreted.

## Inspect without resuming

`stateGlobals` and `stateStack` read a parked run without waking it, which suits a "resume?" screen or debugging a restored blob.

```js
await worker.stateStack();
// [{ state: "waiting_event", function: "<module>", ip: 12, frames: [] }]

await worker.stateGlobals();
// { items: "[10, 25]", total: "35" }   values are reprs
```

The `state` field reads `"waiting_event"` for a parked `receive()`, `"sleeping"` for a `sleep()`, and so on.

## See also

Pure Python state restores identically, while live host resources (DOM handles, sockets, pending host calls) are not captured and must be recreated after restoring. The blob also carries the whole heap, so it must fit the runtime's 1 MiB buffer to restore.

- [Runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/runtime#state-snapshots) for the worker and element API and the size limit.
- [Design](/implementation/design#snapshots) for the serializer internals.
- [WASM module ABI](/reference/wasm-abi#snapshot-exports) for the `compiler.wasm` exports and blob layout.
