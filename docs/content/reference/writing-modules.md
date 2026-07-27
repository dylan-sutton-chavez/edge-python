---
title: "Writing modules"
description: "Three paths to extend Edge Python: a `.wasm` module loaded by URL, a host capability bundled in a custom compiler, or a plain JS module that runs on the page's main thread."
---

Edge Python has no bundled stdlib. Three ways to add native functionality:

| Path | Distribution | Type coverage | Maintenance |
|---|---|---|---|
| **`.wasm` module via URL** ([WASM ABI](/reference/wasm-abi)) | Publish `.wasm` to a CDN; any host loads dynamically | Transit values (None, bool, i128, f64, bytes/str, list, dict) | Reference [`wasm-pdk`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/pdk) (Rust), community PDKs, or hand-written wire boilerplate |
| **Host capability** | Custom `compiler.wasm` (additional host imports declared) + host runtime they bridge to | Transit values + access to host services (DOM, FS, fetch) through embedder host imports | You own embedder + host runtime; bindings travel together |
| **JS host module** | Plain ESM via `createWorker` (`mainThreadModules` eager, `hostModules` lazy) or the `host` field of `packages.json` | Transit values (same as Path A) | Pure JS; no Rust, no `.wasm`, no build step |

`.wasm` matches the marketplace pattern (`from "https://x.wasm" import f` works in any host). Host capability is for runtime distributions that own their `compiler.wasm` and expose host services to scripts (the same pattern `print` and `input` use). JS host modules keep upstream `compiler.wasm` untouched while exposing main-thread surface (DOM, dialogs, FileReader, observers, anything `window.*`).

## Path A: `.wasm` module by URL

Contract: the [WASM module ABI](/reference/wasm-abi), language-agnostic, three scalar types. Rust authors use the bundled [`wasm-pdk`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/pdk) macros ([the surface](/reference/wasm-abi#author-conveniences)). Other languages use community PDKs or hand-roll the boilerplate.

Worked examples (with and without the SDK), encoding tables, and language-specific snippets: [WASM module ABI](/reference/wasm-abi). Script side:

```python
from "./my_edge_mod.wasm" import add
print(add(2, 3)) # -> 5
```

## Path B: host capability

Some native functionality can't live in a CDN-distributed `.wasm` (Path A): the work happens outside the WASM sandbox. DOM mutation, WASI filesystem I/O, native crypto. Path A modules see only the sealed 6 `env.*` imports. They have no channel to the host runtime. Path B closes that gap.

A host capability is shipped as part of a custom embedder. The embedder declares additional host imports beyond the sealed plugin ABI. These imports are the embedder's private contract with its host runtime, not part of the public plugin contract.

Precedent: `print(...)` calls the embedder's `host_print` import. `input()` drains a buffer the host fills via `set_input`. The same shape generalises. A browser-host distribution can register `dom` as a native module whose `query`, `set_text`, `append_child` operations bridge to JS through embedder-specific host imports. A WASI-host distribution can register `fs` against `wasi_snapshot_preview1`. Scripts see them as ordinary native modules:

```python
from dom import document, query # browser host
from fs import read_text, write # WASI host
```

### What ships in a host-capability distribution

| Artifact | Role |
|---|---|
| Custom `compiler.wasm` | Vanilla `compiler` plus declared additional host imports |
| Host runtime | Browser shim / WASI loader / native binary that provides those imports |
| Pure-Python wrappers (`.py`) (optional) | Ergonomic surface on top of the raw bridge, shipped as a code module |

Users opt in by loading the custom `compiler.wasm` and matching host runtime together. Vanilla `compiler.wasm` keeps working for everyone else.

### Sketch

```rust
// custom compiler.wasm declares an extra env import beyond the sealed plugin set
#[link(wasm_import_module = "env")]
unsafe extern "C" {
  fn host_dom_op(opcode: u32, ptr: *const u8, len: u32) -> u32;
}

// And exposes a `dom` module whose operations bridge through it.
```

The custom `compiler.wasm` declares `env.host_dom_op` alongside the standard `env.host_print` / `env.host_call_native` / `env.host_fetch_bytes` / `env.host_now_ns`. The host runtime supplies the implementation.

### Why this is not a third module flavor

Scripts still see two flavors (code and native; see [Imports](/reference/imports)). Path B is a distribution pattern that ships additional bridges through the embedder. The compiler dispatches them the same way as built-in operations. The public language surface and the [WASM module ABI](/reference/wasm-abi) stay untouched.

## Path C: JS host module

Browsers run the engine in a Web Worker (no `document`, no `window`). Path C bridges it: a capability ships as plain JavaScript, registers with `createWorker({ mainThreadModules })`, and runs on the main thread. The runtime synthesises the native module registration, so Python can `from <name> import ...`. Each call is decoded in the Worker, shipped to main via `postMessage`, executed against `document`/`window`/etc., and the result encoded back. Python sees a synchronous call.

Async handlers (returning a `Promise`) run concurrently when several coroutines call them under `gather`. Each result is routed back to the coroutine that issued it. A rejected handler raises a catchable exception in that one coroutine without disturbing its peers.

Registration (eager `mainThreadModules` — shown below —, lazy `hostModules` / `packages.json` `host`, or the built-in defaults): see [How to load them](/reference/packages#how-to-load-them).

No `.wasm`, no Rust, no build step.

### Sketch

A module is a factory `(ctx) => handlers` (or `{name: handler}`). The factory receives `{ pushEvent }` so async callbacks (event listeners, observers, `FileReader`, animation `finished`) can wake a paused `receive()`.

```js
// dom.js
export const dom = ({ pushEvent }) => {
  const nodes = [];
  const alloc = (n) => { if (n == null) return -1; nodes.push(n); return nodes.length - 1; };
  const node = (h) => nodes[h];

  return {
    query: (sel) => alloc(document.querySelector(sel)),
    set_text: (h, txt) => { node(h).textContent = txt; },
    bind_event: (h, type, msg) => {
      node(h).addEventListener(type, (e) => {
        pushEvent(JSON.stringify({ msg, type: e.type, target_id: e.target.id }));
      });
    },
  };
};
```

```html
<script type="module">
  import { createWorker } from "https://cdn.edgepython.com/runtime/src/index.js";
  import { dom } from "./dom.js";

  const worker = await createWorker({
    wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
    mainThreadModules: { dom },
  });
  await worker.run(await (await fetch("./script.py")).text());
</script>
```

```python
from dom import query, set_text, bind_event
bind_event(query("#btn"), "click", "click")
async def main():
  while True:
    receive()
    set_text(query("#btn"), "clicked")
run(main())
```

Or skip the manual wiring: the browser runtime's `<edge-python>` element loads these declaratively from a `host` field in `packages.json`. See the [runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js).

Handlers take decoded JS values and return plain JS values. Supported tags: `None`, `bool`, `int` (i128; a JS `Number` within ±2^53, `BigInt` beyond), `float`, string, `bytes` (`Uint8Array`), plus nested `list` / `dict`. Opaque object references (DOM nodes, files, observers) model as integer IDs into a main-thread registry the handlers own (the `alloc` / `node` pattern above).

### Trade-offs vs Path B

| | Path B | Path C |
|---|---|---|
| Compiler artifact | Custom per capability set | Vanilla upstream |
| Composition | Embed-time | Load-time, by import |
| Binding language | Rust (or C/Zig, any wasm32 target) compiled into the embedder | JavaScript, transit values only |
| Per-op overhead | Native call through embedder host import | `postMessage` round-trip (around 0.1 to 0.4 ms) |
| Threading model | Wherever the embedder runs | Main thread (handlers reach `document`) |
| Build pipeline | `cargo` | None |

Pick Path C when the capability needs main-thread browser surface (DOM, dialogs, observers, FileReader) and per-op latency is acceptable, invisible for UI-rate workloads (around 50 to 200 ops/frame). Reach for Path B when tight per-frame loops dominate or the capability lives in a non-browser host (WASI, native).

Reference implementation: [`host/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/host).

## Choosing between the three paths

| You want... | Use |
|---|---|
| Publish a module any Edge Python user can `from "<url>" import` without rebuilding | Path A (`.wasm` ABI) |
| Wrap a C/Zig/AS library | Path A (any wasm32-targeting language works) |
| Expose host services (DOM, FS, native crypto) bundled into your own runtime distribution | Path B (host capability) |
| Expose browser-main-thread APIs (DOM, dialogs, observers) without shipping a custom embedder | Path C (JS host module) |

## See also

- [WASM module ABI](/reference/wasm-abi), the wire format spec for Path A.
- [Imports](/reference/imports), script-side semantics, packages.json, integrity verification.
