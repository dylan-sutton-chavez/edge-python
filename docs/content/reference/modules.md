---
title: "Modules"
description: "The compile-time import system, packages.json, the four ways to ship your own module, and the native engine."
---

Every import is resolved at compile time. The compiler asks the host for each module, flattens it into the bytecode, and the VM never fetches anything at run time. The host (the browser runtime, the CLI, or your own embedder) decides what each name means.

A module is one of two flavors. Both use the same import syntax, and the host's resolver picks the flavor per spec.

| Flavor | What it is |
|---|---|
| Code module | A `.py` file. Its top level runs once at startup, and its exports live on a module object shared by every importer. |
| Native module | A `.wasm` plugin over the [WASM module ABI](/reference/wasm-abi), a native `.so` plugin loaded by the CLI, or bindings a host registers through the `Resolver` trait. |

## Syntax

```python
from json import dumps, loads                  # bare name, resolved through packages.json or the defaults
from .lib.helpers import slugify               # relative to the importing file (./lib/helpers.py)
from ..shared.util import chunks               # one dir up per extra dot (../shared/util.py)
from lib.helpers import slugify as sl          # absolute from the nearest packages.json dir
import math                                    # binds the module itself, use math.sqrt(2.0)
from utils import *                            # every export becomes a flat name in scope
```

Name lists can span lines inside parentheses, with an optional trailing comma.

Dots map to directories and the `.py` suffix is implicit. A leading dot anchors the spec at the importing file, a dotted name anchors at the nearest `packages.json` directory, and a plain name must be declared in `packages.json` or be one of the [official defaults](#defaults). Two forms do not work: `from . import x` (Edge has no packages, so a bare dot names nothing) and dynamic imports (no `__import__`, no `importlib`, and the module set is fixed per compilation).

## Module semantics

Modules are singletons. The same canonical spec is fetched, parsed, and initialized once, and every importer sees the same object. Mutating a module attribute is visible to all consumers. Inside a module's top level, `__name__` is bound to its canonical spec, so an `if __name__ == "__main__":` block is skipped on import. A module's helpers stay private to it: attribute access goes through the module object, not the importer's globals.

```python
import math
import math as m
print(math is m)
print(import_module("math") is m)
print(__name__)
```

```text Output
True
True
__main__
```

`import_module(name)` looks up a module bound by a plain `import` in the current scope, so it can dispatch among modules already imported. A module pulled in only with `from x import ...` is not visible to it. An import cycle (`a.py` imports `b.py` imports `a.py`) raises `RuntimeError: circular import` at startup.

## packages.json

Bare names resolve through `packages.json`, the only manifest name. All fields are optional.

```json
{
  "imports": { "utils": "./lib/utils.py", "fastmath": "./vendor/fastmath.wasm" },
  "extends": "..",
  "host": { "dom": "./dom/index.js" }
}
```

- `imports`: bare name to spec (path or URL).
- `extends`: a directory whose `packages.json` is consulted when a name is not declared locally. Use it for monorepo sub-packages that share the parent's dependencies. Omit it for hermetic libraries. Cycles in the chain fail at compile time.
- `host`: name to JS module URL, for [host libraries](#host-libraries) that run on the browser's main thread. The compiler folds each name into the import table as a main-thread spec. Loading the JS is the runtime's job.
- Unknown keys are ignored. Values must be strings or objects of strings. Numbers, arrays, and booleans are rejected. Supported string escapes are `\"`, `\\`, `\/`, `\n`, `\t`, `\r`. `\uXXXX` is not supported, so paste UTF-8 literally.

Resolution follows four rules.

1. **Walk-up.** A bare name is resolved against the nearest `packages.json`, walking up from the importing file's directory. Each manifest is a package boundary, the same pattern as Node's `node_modules` discovery. The chain is capped at 32 hops.
2. **Hermetic.** The nearest manifest wins. If it does not declare the name and has no `extends`, compilation fails. A deep dependency cannot borrow a parent's aliases.
3. **Relative to the importer.** A leading-dot spec resolves against the file that contains the import, so a transitively imported `lib/a.py` doing `from .b import g` finds `lib/b.py`.
4. **Spec shapes.** A spec containing `://` or starting with `/` is used as is. A spec starting with `./` or `../` is joined against the importer's directory. Any other spec with a `/` is joined against the nearest `packages.json` directory. Anything else is a bare name for the walk-up.

## Integrity

Append `#sha256-<64 hex chars>` to a spec in `packages.json` to pin its content:

```json
{ "imports": { "utils": "https://example.com/utils.py#sha256-deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567" } }
```

```python
from utils import normalize
```

The runtime fetches the raw bytes, hashes them, and refuses to run on a mismatch. The diagnostic shows both digests:

```text
error: integrity check failed for 'https://example.com/utils.py'
 expected sha256-deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567
 got sha256-36e4838513e46116f258c86b494eaa826d64fa0a9abdf36e8720a31b3d2862e2
```

Only `sha256` is supported. Other prefixes fail with `unrecognized integrity fragment`. Both runtimes enforce the pin the same way, the browser in its fetch layer and the CLI in its native resolver.

The browser runtime additionally caches every fetched module in IndexedDB, in a `cas` store (hash to bytes) and a `lockfile` store (spec to hash). Repeat runs make no network requests. If a locked URL later serves different bytes, the run fails with an `integrity drift` error showing both digests. `clearCache()` on the worker wipes both stores. The CLI's native engine does the same on disk, see [the native engine](#the-native-engine).

## Resolution errors

A bad import is a compile-time diagnostic with the statement's source position, never a catchable runtime exception:

```text
error: alias 'utils' not declared in './packages.json'
  --> main.py:1:6
   |
 1 | from utils import f
   |      ^^^^^

error: module 'json' has no export 'badname'
  --> main.py:2:6
```

This also covers modules Edge Python does not ship, like `os` or `sys`. They parse for syntactic compatibility and are then rejected here, before any code runs.

## Standard packages and host libraries

<a id="standard-packages"></a>
<a id="host-libraries"></a>

The official libraries each have their own page. Standard packages:

- [json](/packages/std/json), [math](/packages/std/math), [re](/packages/std/re), [struct](/packages/std/struct), [test](/packages/std/test)

Host libraries:

- [dom](/packages/host/dom), [network](/packages/host/network), [storage](/packages/host/storage), [time](/packages/host/time)

## Defaults

The browser runtime and the CLI both resolve the official names by bare name with no `packages.json` at all: the standard packages `json`, `re`, `math`, `struct`, `test` and the host libraries `dom`, `network`, `storage`, `time`. Three rules:

- **Lazy.** A default is fetched only when a run actually imports it.
- **Overridable.** Your `packages.json`, `imports`, or `hostModules` entry wins for the same name, so you can pin a version or URL.
- **Opt-out.** Pass `defaults: false` to `createWorker` to disable the defaults entirely.

Defaults are a feature of these two runtimes, not of the compiler. `compiler.wasm` stays hermetic and resolves bare names only through the manifest the host provides. The prebuilt assets live at `https://cdn.edgepython.com/std/<name>.wasm` (`test` ships as `test.py`, `dom` as a facade at `web/builtins/dom/entry.py`) and `https://cdn.edgepython.com/web/builtins/<name>/index.js`.

## The `<edge-python>` element

The declarative alternative to `createWorker`. Include the script, drop a tag, and a `.py` file runs.

```html
<script type="module" src="https://cdn.edgepython.com/web/src/element.js"></script>
<edge-python entry="./app/main.py" packages="./app/packages.json"></edge-python>
```

Importing `element.js` auto-registers the tag. On connect the element reads its attributes and the manifest, spawns the worker, runs `entry` if present, then fires a `ready` event. After `ready` it publishes the worker on `el.worker`, so the full programmatic API drives the same VM. Modules load lazily, only what a run actually imports is fetched.

| Attribute | Description |
|---|---|
| `entry` | Optional URL of a `.py` file to run on connect, resolved against the document. Omit it to drive the worker with `el.worker.run()`. |
| `packages` | Optional `packages.json` URL. One manifest drives both directions, `host` for main-thread libraries and `imports` for worker-side modules. |
| `wasm` | Optional absolute `compiler.wasm` URL, for self-hosting or pinning a build. Defaults to the CDN. |

Where `customElements` is absent (Node, Deno, SSR), append `?setElement=false` to the script URL and register manually with the exported `defineElement(tag)`. When the runtime is served cross-origin, the worker spawns from a same-origin Blob URL that imports the cross-origin module, because Chromium rejects `new Worker()` on a cross-origin URL.

## Writing your own modules

Four delivery paths, by decreasing reach:

| Path | Distribution | Binding language | What it can see |
|---|---|---|---|
| CDN wasm | Publish a `.wasm`, any host loads it by URL | Rust with `wasm-pdk`, or Zig, C, AssemblyScript | Transit values only |
| Native plugin | A `.so` or `.dylib` the CLI loads in-process | Rust, the same crate as the wasm build | Transit values only |
| Host capability | A custom `compiler.wasm` plus a matching host runtime | Rust, or any wasm32 target, inside the embedder | Transit values plus host services (DOM, FS, crypto) |
| JS host module | Plain ESM on the browser main thread | JavaScript | Transit values plus `window` and `document` |

Transit values are `None`, `bool`, `int` (128-bit), `float`, `str`, `bytes`, and nested `list` / `dict`. The exact wire tags are in the [WASM module ABI](/reference/wasm-abi).

### CDN wasm

The contract is the [WASM module ABI](/reference/wasm-abi), language-agnostic and sealed. Rust authors use the `wasm-pdk` crate's macros (`#[plugin_fn]`, `#[plugin_class]`, and friends) and write plain Rust. Other languages use community PDKs or hand-written wire boilerplate. The script side imports it through a manifest alias:

```json
{ "imports": { "slugify_mod": "https://example.com/slugify_mod.wasm" } }
```

```python
from slugify_mod import slugify
print(slugify("Hello World"))
```

One source gives two builds. The same crate compiles to `.wasm` for CDN distribution and to a native plugin for the CLI.

### Native plugin

The CLI's native engine loads `.so` (Linux) and `.dylib` (macOS) plugins with `dlopen`. A plugin exports the same ABI surface as the wasm build and leaves the six `env` imports undefined. The CLI supplies them itself, re-exporting its `edge_*` bridge symbols through `-rdynamic` so `dlopen` binds the plugin straight to the engine. The loader reads `__edge_abi_version` and refuses a version mismatch, a check the browser shim skips because every loader currently targets version 1. The engine is single-threaded by contract: the bridge keeps its handles and the live VM pointer in process-wide statics, so the VM and every `edge_*` call must stay on one thread.

Build any std package as a native plugin with `cargo build --profile native`. The profile inherits `release` and raises `opt-level` to 3.

### Host capability

Some work cannot live in a CDN module because it happens outside the WASM sandbox. A `.wasm` plugin sees only the six sealed `env` imports and has no channel to the host. A host capability closes that gap: you ship a custom `compiler.wasm` that declares additional `env` imports, plus a host runtime that implements them. The scripts import the capability as an ordinary native module.

This is the pattern `print` and `input` already use: `print` calls the embedder's `host_print` import. A browser distribution can register a `dom` module whose operations bridge to JS through its private imports. A WASI distribution can register `fs` against `wasi_snapshot_preview1`.

It is a distribution pattern, not a third module flavor. Scripts still see code modules and native modules. The public language surface and the plugin ABI stay untouched, and vanilla `compiler.wasm` keeps working for everyone who does not load your runtime.

### JS host module

To reach main-thread browser surface (DOM, dialogs, `FileReader`, observers) without shipping a custom compiler, ship the capability as plain JavaScript. A module is a factory `(ctx) => handlers`, or a plain `{name: handler}` object. The factory receives `{ pushEvent }`, which async callbacks use to wake a paused `receive()`. Each call is decoded in the worker, shipped to the main thread, executed, and encoded back. A handler that returns a Promise runs concurrently with other coroutines under `gather`, and a rejection raises a catchable exception in the calling coroutine only.

```js
// dom.js
export const dom = ({ pushEvent }) => {
  const nodes = [];
  const alloc = (n) => { nodes.push(n); return nodes.length - 1; };
  return {
    query: (sel) => alloc(document.querySelector(sel)),
    set_text: (h, txt) => { nodes[h].textContent = txt; },
    bind_event: (h, type, msg) => {
      nodes[h].addEventListener(type, (e) => pushEvent(JSON.stringify({ msg, type: e.type })));
    },
  };
};
```

```html
<script type="module">
  import { createWorker } from "https://cdn.edgepython.com/web/src/index.js";
  import { dom } from "./dom.js";

  const worker = await createWorker({
    wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
    mainThreadModules: { dom },
  });
  await worker.run(await (await fetch("./script.py")).text());
</script>
```

Handlers take decoded JS values and return plain JS values. Opaque objects like DOM nodes model as integer IDs into a registry the handlers own, the `alloc` pattern above. The per-call cost is a `postMessage` round trip, invisible at UI rate. The official [host libraries](#host-libraries) are reference implementations.

## The native engine

`edge run`, `edge repl`, and `edge test` execute in-process by default. No browser, no server, millisecond startup. The `--web` flag hosts the browser runtime in headless Chromium for scripts that need it. Browser-only modules fail fast: `import dom` and `import storage` are compile-time errors reading `module 'dom' requires the web runtime (run with --web)`, and a parked `frame()` reports the same at run time. Everything runs under the [sandbox limits](/reference/limits-and-errors) with a real wall clock, so `sleep()` and timeouts wait in real time.

### Module resolution

Relative imports load from disk relative to the importing file, dotted imports from the nearest `packages.json` dir. Bare names go through the `packages.json` walk-up. Manifest URLs download once into `~/.cache/edge-native` (`$XDG_CACHE_HOME` is honored) with a 64 MB cap. A downloaded file is pinned by a `.lock` sidecar holding its SHA-256, and later runs refuse on drift until you remove the cache entry. A `.so` or `.dylib` target loads as a native plugin. The official std `.wasm` specs that `edge add` writes swap transparently to their `.so` twins, so one manifest serves both engines. With no manifest entry, `json`, `re`, `math`, and `struct` default to the CDN `.so` for the host architecture and `test` to the CDN `test.py`. Set `EDGE_STD_DIR` to a local checkout to serve these from disk instead.

Two modules are compiled into the binary. `time` carries the clocks and the calendar functions, always UTC (there is no timezone database, so `tzname()` is `"UTC"`). `network` exposes a blocking `fetch(url, options_json?)` returning `{id, ok, status, headers, body}`, plus `fetch_text` and `fetch_json`. A manifest entry with the same name always wins over a built-in.

### Run flags

| Flag | Effect |
|---|---|
| `--events <f>` | Each line of the file (or FIFO) feeds one `receive()`. End of input parks the script. |
| `--save-state <f>` | On a wait the engine cannot serve, write a [snapshot](/language/snapshots) to the file and exit 0. |
| `--restore-state <f>` | Boot from a snapshot instead of a script and keep running. An unreadable file exits 2. |
| `--preempt <n>` | Yield every `n` loop back-edges and resume, so a program with no suspension point stays snapshottable. |

These flags are native-only and reject `--web`. Without `--save-state`, a script parked on a wait the engine cannot serve prints an error and exits 1.

### std packages as native libraries

Each tagged release attaches the std packages as native plugins for aarch64 and x86_64 (`json-x86_64.so`, `json-aarch64.dylib`, and so on), and the CDN serves the same assets under `https://cdn.edgepython.com/native/<contract>/`, where `<contract>` is the runtime contract version (`0.1.0` today). They are the same crates shipped as `.wasm` plugins, compiled for the host platform with a glibc 2.17 floor on Linux. That floor is why Linux ships one build instead of a static fallback: a fully static binary links musl's stub `dlopen`, which always fails. The installer checks your glibc and points at `cargo install --path cli` when the host is too old.

### Building from source

```bash
cargo clippy --lib --features native    # lint the engine module
cd cli && cargo build --release         # the CLI embeds the native engine
cd std/json && cargo build --profile native   # any std package as a native plugin
```

## See also

- [WASM module ABI](/reference/wasm-abi): the wire contract behind `.wasm` and native plugins.
- [CLI](/reference/cli): `edge add` writes manifest entries, `edge build` vendors packages for offline use.
- [Limits and errors](/reference/limits-and-errors): the sandbox profile both engines run under.
