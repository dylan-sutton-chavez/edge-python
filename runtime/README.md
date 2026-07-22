# Edge Python Runtime

JS half of Edge Python: hosts `compiler.wasm` in a Web Worker, resolves and registers `.py` / `.wasm` modules, dispatches native calls. Drive it programmatically with `createWorker`, or declaratively with the `<edge-python>` HTML element.

## Development

Requires [Deno v2](https://deno.com/) and Playwright's Chromium (installed on first run).

```bash
deno lint runtime/ # lint
deno run -A npm:playwright install --with-deps chromium # install Chromium (once)
deno test --allow-all runtime/tests/runtime.test.js # run tests
```

## Install

No install, the official CDN serves both the runtime and matching `compiler.wasm`:

```js
import { createWorker } from "https://cdn.edgepython.com/runtime/src/index.js";
// Local checkout: import { createWorker } from "../../runtime/src/index.js";
```

## Usage

```js
const worker = await createWorker({
    wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
    integrity: true, // default: IDB + lockfile CAS
    imports: { dom: "./dom.wasm" }, // bare-name shortcut, optional
    loaders: [], // opt-in module loaders, optional
});

let stdout = "";
worker.onOutput((chunk) => { stdout += chunk; }); // raw byte stream; concatenate, don't add newlines

const { out, ms } = await worker.run(`
    from dom import query, set_text
    set_text(query("#app"), "hello")
`);

worker.dispose();
```

## HTML element (`<edge-python>`)

Declarative alternative to `createWorker`: include the script, drop a tag, and a `.py` file runs. The element wraps `createWorker` on the page's main thread.

```html
<script type="module" src="https://cdn.edgepython.com/runtime/src/element.js"></script>
<edge-python entry="./app/main.py" packages="./app/packages.json"></edge-python>
```

Importing `element.js` auto-registers the tag. On connect, the element reads its attributes and `packages.json`, spawns the worker, runs `entry` if present, then fires a `ready` event. `compiler.wasm` loads from the CDN automatically. Modules load lazily: only what a run actually imports is fetched, host libraries included.

| Attribute | Description |
|---|---|
| `entry` | Optional URL of a `.py` file to run on connect. Omit it to drive the worker via `run()`. Resolved against the document. |
| `packages` | Optional `packages.json` URL. Its `host` and `imports` fields declare the modules to load (see below). |

### Programmatic use

The element publishes its worker on `el.worker` once `ready` fires, so the full [`Worker`](#worker) API drives the same VM. The element itself adds no methods.

```js
const el = document.querySelector("edge-python");
await new Promise((r) => el.addEventListener("ready", r, { once: true }));
el.worker.onOutput((chunk) => process.stdout.write(chunk)); // raw chunks, no added newline
await el.worker.run("print(1 + 1)"); // 2
```

### Registration

Where `customElements` is absent (Deno, Node, SSR), append `?setElement=false` to the script URL to skip the auto-call, then register manually with the exported `defineElement(tag = "edge-python")`, where custom tags must contain a hyphen:

```js
import { defineElement } from "https://cdn.edgepython.com/runtime/src/element.js?setElement=false";
defineElement("edge-py");
```

### Importing host libraries

Host libraries (DOM, etc.) are plain-JS modules whose handlers run on the **page's main thread**, because they touch `document` / `window`, which the worker can't reach. Declare them in the `host` field of `packages.json`:

```json
{
  "host": {
    "dom": "/host/dom/src/index.js"
  }
}
```

Each `host` entry maps a name to an ESM URL (resolved against the `packages.json` location). The element passes these to `createWorker` as `hostModules`; the module is `import()`ed lazily the first time a run imports that name, never at connect, so an unused host library is never fetched. The ESM exports its handler factory under the host name (or as `default`), so `export const dom` answers `from dom import ...`:

```python
# app/main.py
from dom import query, set_text
set_text(query("#app"), "hello")
```

The element reads the same `packages.json` for the standard `imports` field too: those bare-name `.py` / `.wasm` modules are passed to `createWorker`'s `imports`. So one manifest drives both directions, `host` to the main thread and `imports` to the worker. Together they are the declarative form of the [`mainThreadModules`](#main-thread-modules) and `imports` options.

## API

### `createWorker(opts)` -> `Promise<Worker>`

Spawns a Web Worker, loads `compiler.wasm` inside it, returns a proxy.

| Option | Type | Default | Description |
|---|---|---|---|
| `wasmUrl` | `string` | — | URL of `compiler.wasm`. |
| `integrity` | `boolean` | `true` | When `true`, use IDB + lockfile to cache and verify fetched module bytes. Falls back to in-memory cache (with `console.warn`) if IDB is unavailable. |
| `imports` | `Record<string, string>` | `null` | Bare-name shortcut: maps Python bare names (`from <name> import ...`) to URLs of `.py` / `.wasm` modules. Replaces the need for a physical `packages.json` for simple projects. |
| `loaders` | `string[]` | `[]` | URLs of module loader plugins. Each loader is a `.js` file with a default export `{ match, load }`. See [Writing a loader](#writing-a-loader). |
| `mainThreadModules` | `Record<string, factory \| object>` | `{}` | Main-thread modules supplied as in-memory factories/objects, registered eagerly. Use `hostModules` instead when you have URLs and want lazy loading. See [Main-thread modules](#main-thread-modules). |
| `hostModules` | `Record<string, string>` | `{}` | Main-thread host libraries by URL (`name -> ESM url`), `import()`ed lazily the first time a run imports the name. The `<edge-python>` element fills this from the `host` field. |
| `defaults` | `boolean` | `true` | Seed the resolution table with the official packages so they resolve by bare name without a `packages.json`: std `json` / `re` (worker `.wasm`) and host `dom` / `network` / `storage` / `time` (main-thread ESM). Lazy, an unused default is never fetched. Set `false` to opt out. URLs live in `src/defaults.js`. |
| `version` | `string` | `null` | Optional lockfile version key. When present, mismatches with the stored version invalidate the cache before run. Useful to pin cache to a deploy/commit. |

### `Worker`

The returned object exposes:

| Member | Type | Description |
|---|---|---|
| `integrityActive` | `boolean` | `true` iff IDB cache opened successfully. Inspect after `createWorker` to detect silent fallback. |
| `loadMs` | `number` | Wall time to load + compile `compiler.wasm`. |
| `run(src, opts?)` | `(string, {entryDir?, baseUrl?}) => Promise<{out, ms}>` | Execute a Python source string. The runtime does not auto-invoke `main`, scripts that define `async def main()` must drive it themselves with a trailing `run(main())`. Top-level scripts (no `main`) execute under the implicit module-body coroutine, so `receive()`, `sleep()`, etc. still work without wrapping. `entryDir` is a prefix joined to relative import specs; `baseUrl` overrides the base for URL resolution (defaults to the worker's `location.href`). Resolves with stdout (concatenated `print()` lines if no `onOutput`) and wall time. |
| `onOutput(handler)` | `(chunk: string) => void` | Streaming output callback fired once per `print()`, with the raw bytes (body + its `end`, no newline added). Concatenate chunks; don't join with `\n`. |
| `reset()` | `() => Promise<void>` | Clear registered modules without rebooting the worker. |
| `clearCache()` | `() => Promise<void>` | Wipe IDB CAS + lockfile (or memory cache). Next run re-fetches everything. |
| `pushEvent(message)` | `(string) => void` | Wake a paused `receive()` in the running script with `message`. Fire-and-forget. Browser bridges fire `CustomEvent("edge-python-event")` on `window`, which `createWorker` routes through `pushEvent` automatically. |
| `saveState()` | `() => Promise<Uint8Array>` | Snapshot the paused program (see [State snapshots](#state-snapshots)) as a portable blob. Rejects when no run is paused. |
| `restoreState(blob)` | `(Uint8Array) => Promise<{out, ms}>` | Boot from a saved blob and continue the program from where it was saved. Resolves like `run()` when it finishes. |
| `stateGlobals()` | `() => Promise<object>` | `{name: repr}` of the paused program's module-level bindings. |
| `stateStack()` | `() => Promise<array>` | Per-coroutine `{state, function, ip, frames}` of the paused program. |
| `setPreemptInterval(n)` | `(number) => Promise<void>` | Yield every `n` loop back-edges so a program with no suspension point stays pausable. Each yield is an event-loop round trip, so a small `n` pauses sooner and costs more. Defaults to `0`, which disables it. |
| `pause()` | `() => Promise<boolean>` | Park the running program so `saveState()` can capture it. Resolves `true` once parked, `false` when no run reached a park (already finished, or none started). Needs `setPreemptInterval` unless the program suspends on its own. |
| `resume()` | `() => Promise<void>` | Continue a program parked by `pause()`. |
| `dispose()` | `() => void` | Terminate the worker. Subsequent calls fail. |

### State snapshots

While a program is parked, its entire interpreter state, heap, globals, suspended coroutines and call frames included, can be serialized. A program parks either on its own (`receive()`, `sleep()`, `frame()`, a host call) or because the host asked it to:

```js
const blob = await worker.saveState(); // Uint8Array, persist anywhere
// ... later, any page load, same runtime version ...
const done = worker.restoreState(blob); // continues from the pause point
worker.pushEvent("hello"); // steer the restored copy
await done;
```

The blob embeds the source and a compiler fingerprint: `restoreState` re-parses, verifies both, and rejects blobs saved by a different program or compiler version. One blob can be restored any number of times, each restore is an independent copy. 

A program that never suspends is snapshottable too, once the host arms preemption:

```js
await worker.setPreemptInterval(50_000); // yield every 50k loop back-edges
worker.run("n = 0\nwhile True:\n    n += 1"); // no receive(), no sleep()
await worker.pause();                    // resolves true, parked mid-loop
const blob = await worker.saveState();
worker.resume();
```

Preempts land on loop back-edges where the VM can unwind. Function and method bodies, `for` / `while` loops, comprehensions and `try` / `with` blocks all qualify; class bodies, module top level, builtin callbacks (`sort(key=...)`, a generator drained by `list()`), `__init__` / `__call__`, and single long native operations run past the interval and yield at the next reachable back-edge. That delays a pause; it never corrupts one.

Host-side resources are not part of the snapshot: DOM node handles, sockets and in-flight host calls do not survive; programs that only hold Python state (plus queued but unconsumed events, which are preserved) restore exactly.

Size ceiling: the blob holds the source **plus the entire live heap and coroutine state**, and `restoreState` loads it through the runtime's source buffer, capped at 1 MiB (`1 << 20`). `saveState` is uncapped, so a program with a large live heap can produce a blob it returns happily but `restoreState` later rejects with `snapshot exceeds 1048576 bytes`. The limit bites only on restore; keep snapshotted state well under the ceiling.

## Writing a loader

A loader is a `.js` file with a default export:

```js
export default {
    /** Inspect the compiled module; return true if this loader handles it. */
    match(module) {
        const names = WebAssembly.Module.exports(module).map(e => e.name);
        return names.includes('my_marker_export');
    },

    /** Load the module and return its callable surface. */
    async load(module, ctx) {
        /**
         * ctx.compilerExports, compiler.wasm instance exports (wasm_alloc, host_edge_*, etc.)
         * ctx.rt, handle codec helpers (decodeStr, encodeInt, ...)
         * ctx.fetchedSources, Map of already-fetched spec -> bytes
         * ctx.loaders, full loader list (in order)
         */

        return {
            kind: 'wasmpdk' | 'capability',
            names: ['fn1', 'fn2', ...],
            fns: [fn1Impl, fn2Impl, ...],
        };
    },
};
```

Two valid `kind` values:

- **`wasmpdk`**, each `fn` is a wasm export with signature `(g_argv, argc, g_out) -> i32` reading from its own linear memory. Each fn must be annotated with `__edge_alloc`, `__edge_memory`, and (optionally) `__edge_free` (the built-in loader does this automatically). The dispatcher stages argv in guest memory, copies the result handle back, and frees the staging when the guest exports `__edge_free`.

- **`capability`**, each `fn` is a plain JS function `(handles: number[]) => number` taking u32 handles in compiler's memory and returning a u32 result handle. The dispatcher calls it directly without staging.

The built-in Path A wasm-pdk loader is always tried last as fallback; custom loaders run first in order.

## Main-thread modules

Engine runs in a Web Worker, so handlers can't reach `document` / `window`. `mainThreadModules`: a pure-JS module declares its handlers, the runtime synthesises the native registration so Python can `from <name> import ...`, each call defers to main transparently. Python sees a regular synchronous call.

Factory `(ctx) => handlers` or `{name: handler}`. Factory form receives `{ pushEvent }` so async callbacks (events, observers, file reads) can wake a paused `receive()`.

```js
const dom = ({ pushEvent }) => {
    const nodes = [];
    const alloc = (n) => { nodes.push(n); return nodes.length - 1; };
    return {
        query: (sel) => alloc(document.querySelector(sel)),
        set_text: (h, txt) => { nodes[h].textContent = txt; },
        // async handlers call pushEvent(jsonDetail) to wake a paused receive()
    };
};

const worker = await createWorker({ wasmUrl: "...", mainThreadModules: { dom } });
```

Supported values: `None`, `bool`, `int` (i64, range-limited by JS Number), `float`, `str`, `bytes` (`Uint8Array`), plus nested `list` / `dict` (str keys) of these. Opaque references (DOM nodes, files, observers) to integer IDs in a main-thread registry (the `alloc` / `node` pattern).

Per-call overhead: one `postMessage` round-trip (around 0.1 to 0.4 ms in modern browsers). Fine for UI-rate workloads. For tight per-frame loops over thousands of fine-grained ops, prefer a Worker-side capability (Path A `.wasm`).

## Worker bootstrap

When the runtime is cross-origin (runtime served from `cdn.edgepython.com/runtime/`), Chromium rejects `new Worker(crossOriginUrl)` even with `type: 'module'`. `createWorker` spawns from a same-origin **Blob URL** that dynamically `import()`s the cross-origin module. Same-origin imports use the direct path; `createWorker` auto-selects from `import.meta.url`. The Blob bootstrap buffers any `postMessage` arriving before `worker.js` installs its handler.

## Module fetch lifecycle

`load` runs once per Worker; `run` can be called many times. `compiler.wasm` is compiled once at `load`; a fresh instance is created per `run` so VM state cannot leak. Resolution is lazy: the compiler classifies each import and only the modules a run actually uses get fetched. Bare names resolve against the manifest chain (built-in defaults < user `packages.json`); manifests are resolution tables, not download lists, so a declared-but-unused package is never downloaded. Module bytes (`.py` / `.wasm` / `packages.json`) are cached across runs in the same Worker, prefetch skips fetched specs, 404'd manifests are remembered. Use `clearCache()` to drop both caches.

A spec the prefetch can't fetch or register (wrong scheme, a `.wasm` served as HTML, a malformed binary) aborts the run before it starts with a clear error, with an `https://` hint for `http://` or schemeless URL specs, instead of letting the VM fail later with `not registered`.

## Layout

| Path | Purpose |
|---|---|
| `src/index.js` | Public API. `createWorker` factory (main-thread). |
| `src/element.js` | Public `<edge-python>` custom element. Wraps `createWorker`; reads `host` / `imports` from `packages.json`. |
| `worker/engine.js` | Internal orchestrator (Worker only). `load`, `run`, `pushEvent`, `reset`, `clearCache`, `dispose`, host-call delegates. |
| `src/env.js` | The 4 `env.*` imports `compiler` declares: `host_print`, `host_call_native`, `host_fetch_bytes`, `host_now_ns`. |
| `src/native.js` | Native module loader extension point + built-in Path A (wasm-pdk) loader + `nativeTable`. |
| `src/prefetch.js` | Lazy BFS over the dependency graph; resolves and registers only the modules a run uses. |
| `src/defaults.js` | Built-in base manifest: official std + host packages, resolvable by bare name. |
| `src/fetch.js` | CAS-backed fetch with lockfile integrity check. |
| `src/specs.js` | URL/spec helpers mirroring `compiler::modules::packages::manifest`. |
| `src/rt.js` | Handle codec wrappers (`decodeStr`, `encodeInt`, ...) for loaders. |
| `src/cache/{memory,idb}.js` | In-memory (per-Worker) and IndexedDB (persistent) cache backends. |
| `worker/worker.js` | Web Worker entry; postMessage protocol. |

## License

MIT OR Apache-2.0
