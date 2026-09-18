---
title: "Modules"
description: "The compile-time import system, packages.json, the three ways to ship your own module, and the CLI."
---

Every import is resolved at compile time. The compiler asks the host for each module, flattens it into the bytecode, and the VM never fetches anything at run time. The host (the JS host, the CLI, or your own embedder) decides what each name means.

A module is one of two flavors. Both use the same import syntax, and the host's resolver picks the flavor per spec.

| Flavor | What it is |
|---|---|
| Code module | A `.py` file. Its top level runs once at startup, and its exports live on a module object shared by every importer. |
| Native module | A `.wasm` plugin over the [ABI](/reference/abi), or bindings a host registers through the `Resolver` trait. |

## Syntax

```python
from json import dumps, loads                  # bare name, declared in packages.json
from .lib.helpers import slugify               # relative to the importing file (./lib/helpers.py)
from ..shared.util import chunks               # one dir up per extra dot (../shared/util.py)
from lib.helpers import slugify as sl          # absolute from the nearest packages.json dir
import math                                    # binds the module itself, use math.sqrt(2.0)
from utils import *                            # every export becomes a flat name in scope
```

Name lists can span lines inside parentheses, with an optional trailing comma.

Dots map to directories and the `.py` suffix is implicit. A leading dot anchors the spec at the importing file, a dotted name anchors at the nearest `packages.json` directory, and a plain name must be declared in `packages.json`. That includes the official packages, nothing resolves without a manifest entry, and `edge add <name>` writes the entry for each official name. The CLI keeps embedded copies of the standard packages, so their entries need no network there. Two forms do not work: `from . import x` (Edge has no packages, so a bare dot names nothing) and dynamic imports (no `__import__`, no `importlib`, and the module set is fixed per compilation).

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
  "system": { "dom": "./dom/index.js" }
}
```

- `imports`: bare name to spec (path or URL).
- `extends`: a directory whose `packages.json` is consulted when a name is not declared locally. Use it for monorepo sub-packages that share the parent's dependencies. Omit it for hermetic libraries. Cycles in the chain fail at compile time.
- `system`: name to JS module URL, for [system libraries](#system-libraries) that run on the JS host's main thread, outside the worker. The compiler folds each name into the import table as a main-thread spec. Loading the JS is the JS host's job.
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

The JS host fetches the raw bytes, hashes them, and refuses to run on a mismatch. The diagnostic shows both digests:

```text
error: integrity check failed for 'https://example.com/utils.py'
 expected sha256-deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567
 got sha256-36e4838513e46116f258c86b494eaa826d64fa0a9abdf36e8720a31b3d2862e2
```

Only `sha256` is supported. Other prefixes fail with `unrecognized integrity fragment`. Both hosts enforce the pin the same way, the JS host in its fetch layer and the CLI in its module fetcher.

In a browser the JS host additionally caches every fetched module in IndexedDB, in a `cas` store (hash to bytes) and a `lockfile` store (spec to hash). Repeat runs make no network requests. If a locked URL later serves different bytes, the run fails with an `integrity drift` error showing both digests. `clearCache()` on the worker wipes both stores. Without IndexedDB, as in Deno, fetched modules stay in memory for the worker's lifetime and no lockfile is kept. The CLI does the same on disk, see [The CLI](#the-cli).

## Resolution errors

A bad import is a compile-time diagnostic with the statement's source position, never a catchable runtime exception:

```text
error: module 'utils' is not provided by this host and no packages.json declares it
  --> main.py:1:6
   |
 1 | from utils import f
   |      ^^^^^

error: module 'json' has no export 'badname'
  --> main.py:2:6
```

The first diagnostic is the same for a typo, for an official package you forgot to declare, and for a module the other host provides, such as `actor` in the JS host. It also covers modules Edge Python does not ship, like `os` or `sys`. They parse for syntactic compatibility and are then rejected here, before any code runs.

## Standard packages and system libraries

<a id="standard-packages"></a>
<a id="system-libraries"></a>

The official libraries each have their own page. Standard packages:

- [json](/packages/std/json), [math](/packages/std/math), [re](/packages/std/re), [struct](/packages/std/struct), [test](/packages/std/test)

System libraries:

- [dom](/packages/system/dom), [network](/packages/system/network), [storage](/packages/system/storage), [time](/packages/system/time), [actor](/packages/system/actor)

None of them resolves on its own. A project declares each one it uses, and `edge add` writes the entry. Standard packages go to `imports` as `https://cdn.edgepython.com/std/<name>.wasm` (`test` ships as `test.py`, `dom` as a facade at `https://cdn.edgepython.com/js/builtins/dom/entry.py`), the other system libraries go to `system` as `https://cdn.edgepython.com/js/builtins/<name>/index.js`. The JS host fetches those URLs and caches them. The CLI resolves the standard package URLs to the copies embedded in the binary and the `system` names `time`, `network`, and `actor` to its Rust implementations, so the same manifest serves both hosts and needs no network in the CLI. A page that drives `createWorker` directly passes the same entries through `imports` and `systemModules`, or a `mainThreadModules` object for an in-page module.

```json
{
  "imports": { "json": "https://cdn.edgepython.com/std/json.wasm" },
  "system": { "network": "https://cdn.edgepython.com/js/builtins/network/index.js" }
}
```

Modules load lazily, an entry a run never imports is never fetched, and an entry that points at a different URL pins that version.

## The `<edge-python>` element

The declarative alternative to `createWorker`. Include the script, drop a tag, and a `.py` file runs.

```html
<script type="module" src="https://cdn.edgepython.com/js/src/element.js"></script>
<edge-python entry="./app/main.py" packages="./app/packages.json"></edge-python>
```

Importing `element.js` auto-registers the tag. On connect the element reads its attributes and the manifest, spawns the worker, runs `entry` if present, then fires a `ready` event. After `ready` it publishes the worker on `el.worker`, so the full programmatic API drives the same VM. Modules load lazily, only what a run actually imports is fetched. Without `packages` the element runs with no modules at all, every import fails.

| Attribute | Description |
|---|---|
| `entry` | Optional URL of a `.py` file to run on connect, resolved against the document. Omit it to drive the worker with `el.worker.run()`. |
| `packages` | The `packages.json` URL, required for any import. One manifest drives both directions, `system` for main-thread libraries and `imports` for worker-side modules. |
| `wasm` | Optional absolute `compiler.wasm` URL, for self-hosting or pinning a build. Defaults to the CDN. |

The element needs a browser. Where `customElements` is absent (Node, Deno, SSR), append `?setElement=false` to the script URL and register manually with the exported `defineElement(tag)`. When the JS host is served cross-origin, the worker spawns from a same-origin Blob URL that imports the cross-origin module, because Chromium rejects `new Worker()` on a cross-origin URL.

## Writing your own modules

Three delivery paths, by decreasing reach:

| Path | Distribution | Binding language | What it can see |
|---|---|---|---|
| CDN wasm | Publish a `.wasm`, the JS host loads it by URL | Rust with `wasm-pdk`, or Zig, C, AssemblyScript | Transit values only |
| System capability | A custom `compiler.wasm` plus a matching host | Rust, or any wasm32 target, inside the embedder | Transit values plus host services (DOM, FS, crypto) |
| JS system module | Plain ESM on the JS host's main thread | JavaScript | Transit values plus the host globals, `window` and `document` in a browser |

Transit values are `None`, `bool`, `int` (128-bit), `float`, `str`, `bytes`, and nested `list` / `dict`. The exact wire tags are in the [ABI](/reference/abi).

### CDN wasm

The contract is the [ABI](/reference/abi), language-agnostic and sealed. Rust authors use the `wasm-pdk` crate's macros (`#[plugin_fn]`, `#[plugin_class]`, and friends) and write plain Rust. Other languages use community PDKs or hand-written wire boilerplate. The script side imports it through a manifest alias:

```json
{ "imports": { "slugify_mod": "https://example.com/slugify_mod.wasm" } }
```

```python
from slugify_mod import slugify
print(slugify("Hello World"))
```

The official std packages are these same `.wasm` files, embedded in the CLI at build time.

### System capability

Some work cannot live in a CDN module because it happens outside the WASM sandbox. A `.wasm` plugin sees only the six sealed `env` imports and has no channel to the host. A system library closes that gap: you ship a custom `compiler.wasm` that declares additional `env` imports, plus a host that implements them. The scripts import the capability as an ordinary native module.

This is the pattern `print` and `input` already use: `print` calls the embedder's `host_print` import. A browser distribution can register a `dom` module whose operations bridge to JS through its private imports. A WASI distribution can register `fs` against `wasi_snapshot_preview1`.

It is a distribution pattern, not a third module flavor. Scripts still see code modules and native modules. The public language surface and the plugin ABI stay untouched, and vanilla `compiler.wasm` keeps working for everyone who does not load your host.

### JS system module

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
  import { createWorker } from "https://cdn.edgepython.com/js/src/index.js";
  import { dom } from "./dom.js";

  const worker = await createWorker({
    wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
    mainThreadModules: { dom },
  });
  await worker.run(await (await fetch("./script.py")).text());
</script>
```

Handlers take decoded JS values and return plain JS values. Opaque objects like DOM nodes model as integer IDs into a registry the handlers own, the `alloc` pattern above. The per-call cost is a `postMessage` round trip, invisible at UI rate. The official [system libraries](#system-libraries) are reference implementations. `mainThreadModules` registers an object the page already holds, `systemModules` maps a name to an ESM URL the host imports on first use, and a `system` entry in `packages.json` is the same thing declared in the manifest.

A module that reaches a browser global loads anywhere and fails where the global is missing. In a JavaScript runtime without a page, the first `dom` call raises `module 'dom' needs 'document', missing in this runtime` in the calling coroutine, the runtime's own error surfaced by the host, and the same shape covers any module and any missing global. `frame()` there rejects the run with `frame() needs requestAnimationFrame, missing in this runtime`, and `createWorker` without Web Workers throws `createWorker needs Worker, missing in this runtime`.

## The CLI

`edge run`, `edge repl`, `edge test`, and `edge actor` run the same `compiler.wasm` the JS host loads, precompiled for the host machine and executed under wasmtime. No browser, no server, millisecond startup. Modules that need a browser fail fast, a declared `dom` or `storage` is a compile-time error reading `module 'dom' requires a browser`, and a parked `frame()` reports the same at run time. Everything runs under the [sandbox limits](/reference/limits-and-errors) with a real wall clock, so `sleep()` and timeouts wait in real time.

### Module resolution

Relative imports load from disk relative to the importing file, dotted imports from the nearest `packages.json` dir. Bare names go through the `packages.json` walk-up, nothing resolves without an entry. Manifest URLs download once into `~/.cache/edge/modules` (`$XDG_CACHE_HOME` is honored) with a 64 MB cap. A downloaded file is pinned by a `.lock` sidecar holding its SHA-256, and later runs refuse on drift until you remove the cache entry.

The standard packages are built into the binary. The official CDN URLs that `edge add` writes (`https://cdn.edgepython.com/std/<name>.wasm`, `test.py` for `test`) resolve to the embedded copies with no network, so one manifest serves both hosts. Any other `.wasm` target fails at compile time with `requires the JS host`, and a `.so` or `.dylib` target with `is not supported, ship a .wasm`, the CLI loads no plugin from disk or over the network.

A `system` entry resolves by name, the URL only matters to the JS host. `time` carries the clocks and the calendar functions, always UTC (there is no timezone database, so `tzname()` is `"UTC"`). `network` exposes `fetch(url, options_json?)` returning `{id, ok, status, headers, body}`, plus `fetch_text` and `fetch_json`, each suspending the coroutine until the response lands. The WebSocket and Server-Sent Events names are not exported, importing them fails at compile time. `actor` is the message passing of an [actor pool](/reference/actors). `dom` and `storage` declared in `system` fail at compile time with `module 'dom' requires a browser`.

### Run flags

| Flag | Effect |
|---|---|
| `--events <f>` | Each line of the file (or FIFO) feeds one `receive()`. End of input parks the script. |
| `--save-state <f>` | On a wait the engine cannot serve, write a [snapshot](/language/snapshots) to the file and exit 0. |
| `--restore-state <f>` | Boot from a snapshot instead of a script and keep running. An unreadable file exits 2. |
| `--preempt <n>` | Yield every `n` loop back-edges and resume, so a program with no suspension point stays snapshottable. |

Without `--save-state`, a script parked on a wait the engine cannot serve prints an error and exits 1.

### Untrusted code

Code you do not trust goes to an [eval group](/reference/actors#untrusted-code), where each message runs in a fresh wasm instance with a memory cap and a ten second CPU deadline, on every operating system. A snippet imports nothing, a bundle imports only what its own `packages.json` declares.

### Building from source

```bash
cargo wasm                                                              # compiler.wasm
(cd std/json && cargo build --release --target wasm32-unknown-unknown)  # and re, math, struct
cd cli && cargo build --release                                         # embeds them precompiled
```

`cli/build.rs` reads the artifacts from those paths, or from `EDGE_COMPILER_WASM` and `EDGE_STD_DIR`, and fetches nothing.

## See also

- [ABI](/reference/abi): the wire contract behind `.wasm` plugins.
- [CLI](/reference/cli): `edge add` writes manifest entries, `edge build` vendors packages for offline use.
- [Limits and errors](/reference/limits-and-errors): the sandbox profile both hosts run under.
