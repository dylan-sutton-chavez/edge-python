---
title: "Imports"
description: "Importing modules in Edge Python: syntax, resolution, and the two module flavors."
---

Supports `import`, `from <spec> import <names>`, and `from <spec> import *`. Resolution is compile-time. The VM never learns what a module is. Bytecode flattens every import into the sequence that materialises the exports.

## The two flavors

| Flavor | What it is | How it dispatches |
|---|---|---|
| **Code module** | A `.py` file | Top level runs once at VM init in its own slot frame; bindings live in a `HeapObj::Module` shared by every importer via `OpCode::LoadModule`. |
| **Native module** | A `.wasm` binary following the [WASM module ABI](/reference/wasm-abi) (URL/path), or Rust closures via the `Resolver` trait. See [Writing modules](/reference/writing-modules). | `CallExtern` (named import) or `HeapObj::Module` carrying `HeapObj::Extern` callables (`import X`). |

Same `import` syntax for both. The host's resolver decides flavor per spec.

Native modules also cover host packages, bindings shipped by the embedder (DOM in browser, FS in WASI). See [Writing modules / Path B](/reference/writing-modules#path-b-host-capability) for the custom-embedder variant. See [Path C](/reference/writing-modules#path-c-js-host-module) for the plain-JS variant that runs on the page's main thread without a custom embedder.

## Syntax

```python
# Bare-name imports, resolved via the host's import map / packages.json
from json import dumps, loads
from utils import normalize

# String-form imports, explicit URLs or local paths, no map needed
from "./lib/helpers.py" import slugify
from "https://example.com/utils.py" import normalize
from "https://example.com/math.wasm" import add

# Aliases via `as`
from math import sqrt as root
from utils import normalize as n

# Parenthesized name lists, multi-line, optional trailing comma
from utils import (
  slugify,
  normalize as n,
  titlecase,
)

# Plain `import X`, binds the module under its name; access exports via `.`
import math
print(math.sqrt(2.0))

# Star imports, every export becomes a flat name in scope
from utils import *
print(slugify("Hello world"))
```

The names above (`json`, `utils`, `math`) are illustrative. `json` is an [official standard package](/reference/packages#json) that the browser runtime resolves by [default](/reference/packages#defaults). `utils` and `math` stand in for your own modules. Apart from the official defaults, every bare name must be declared in `packages.json` or supplied as a quoted path/URL.

## How resolution works

1. Compiler scans the source for every `from <spec> ...`.
2. For each spec, asks the host `Resolver` to materialise as `Resolved::Native { bindings, classes, consts, canonical }` or `Resolved::Code { src, canonical }`.
3. Natives become direct-dispatch entries. Code modules parse to a fresh `SSAChunk`. Both register under the canonical spec, so first-class references and `import_module()` lookups resolve uniformly. See [Syntax, Imports](/implementation/syntax).

Modules are singletons across the compilation unit: same canonical spec -> one `SSAChunk`, top level runs once, one `HeapObj::Module` shared by every importer. Two files importing `./util.py` see the same module. `mod is mod_alias` is true. Module-attr mutations are observed by every consumer. Inside the module's top level, `__name__` is bound to the canonical spec, so `if __name__ == "__main__":` skips when imported.

Helpers and constants stay private: they live in the module's slot frame, reached via attribute access on the `HeapObj::Module` Val, not through the parent chunk's name table. `a.f` calling `helper` resolves through `a`'s attrs, not the importer's globals.

The runtime never fetches. The host (browser, WASI, embedded Rust) brings the bytes. The compiler accepts them through `Resolver`.

## packages.json

Bare-name imports resolve through an import map in `packages.json` (next to the entry script). The manifest is always `packages.json`, with no `edge.json` or other variants.

```json
{
  "imports": {
    "utils": "./lib/utils.py",
    "helpers": "https://example.com/helpers.py",
    "math": "./vendor/math.wasm"
  }
}
```

Schema:

- Top-level value is a JSON object. Empty `{}` is valid.
- `imports` (optional): alias -> spec string.
- `extends` (optional): directory whose `packages.json` is consulted when an alias isn't found locally.
- `host` (optional): name -> JS module URL. Read by the browser runtime's `<edge-python>` element to load [host libraries](/reference/packages#host-libraries) on the main thread (DOM, network, storage…). The compiler folds each `host` name into the import table as a main-thread spec (`mt:<name>`); loading the module is the runtime's job. See the [runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js).
- Unknown top-level keys silently ignored (forward-compatible).
- Booleans, numbers, arrays at any level are rejected.
- String escapes: `\"`, `\\`, `\/`, `\n`, `\t`, `\r`. `\uXXXX` not supported. Paste UTF-8 literally.

`from utils import x` resolves to `./lib/utils.py` relative to the entry script. `from math import add` loads `.wasm` per the [wire format](/reference/wasm-abi).

`packages.json` is optional. Scripts can use string-form paths directly without project config, and the browser runtime resolves the [official packages](/reference/packages#defaults) by bare name without it.

### Walk-up resolution

Bare-name imports resolve against the nearest `packages.json`, walking up from the importing file's directory. Each `packages.json` defines a package boundary. Files under its directory belong to that package. The manifest is the sole authority for what bare names mean inside. Sub-directories may carry their own `packages.json`, the same pattern as Node's `node_modules` discovery or Rust's per-crate `Cargo.toml`.

```
my_app/
├── packages.json <- root manifest
├── main.py # bare imports here resolve via root manifest
├── lib/
│   ├── packages.json <- sub-package manifest
│   └── helper.py # bare imports here resolve via lib/ first
└── ...
```

- **Bare-name imports** (`from utils import x`) walk up looking for `packages.json`. The nearest manifest found wins; following its `extends` chain is capped at 32 hops. Over that: `packages.json walk-up exceeded <cap> hops resolving '<name>'`.
- **Hermetic by default**: if the nearest manifest doesn't declare the alias, compilation fails (`alias '<name>' not declared in '<manifest>'`). No silent fall-through. This prevents a deep transitive dep from borrowing parent aliases.
- **`host` entries declare too**: a manifest's `host` field ([Path C](/reference/writing-modules#path-c-js-host-module)) grants those bare names to the files under it, same walk-up rules.
- **`extends` opts in to inheritance**: `"extends": ".."` re-runs the search from the extended directory when the alias isn't local. Cycles detected at compile time (`circular extends chain in packages.json`).
- **Quoted relative paths** (`from "./helpers.py" import f`) resolve against the importing file; transitively-imported `lib/a.py` doing `from "./b.py" import g` finds `lib/b.py`.

Spec classification (handled by the resolver):

| Spec shape | Example | Resolution |
|---|---|---|
| URL (contains `://`) | `https://x.com/u.py` | Used as-is; passed to `fetch_bytes` |
| Absolute (`/`-prefixed) | `/usr/share/lib.py` | Used as-is |
| Relative (`./` or `../`) | `./util.py` | Joined against importer's directory |
| Bare name | `utils` | Walk-up `packages.json` resolution |

#### `extends`

```json
// lib/packages.json
{
  "extends": "..",
  "imports": { "db": "./postgres.py" }
}
```

`db` is local to `lib/`. Anything else falls through to `../packages.json`. Use it for monorepo sub-packages that share upstream deps with the parent. Omit it for hermetic libraries that should be unaffected by consumer declarations.

### Diamond imports and cycles

When multiple paths import the same module, it's fetched, parsed, and initialised once. The parser caches `SSAChunk` per canonical spec. The VM's `init_modules` walk dedupes by spec. Cycles (`a.py` -> `b.py` -> `a.py`) surface as runtime `circular import`.

## Host responsibilities

The compiler is a WebAssembly module. Fetching bytes is the host's job:

- Browser runtime ([`js/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js)) uses `fetch()` in a Web Worker.
- WASI hosts use their FS/network APIs.
- Embedders pre-stage modules for `Resolver`.

| Scenario | Who fetches |
|---|---|
| Browser playground | `js/` package, pre-fetches every spec the script imports. `.py` files register via `register_code_module`; `.wasm` files instantiate via `WebAssembly.instantiate` and register exports via `register_native_module`. |
| WASI runtime | Host program reads `.py` files from disk / network using `wasi_snapshot_preview1`. `.wasm` modules can be loaded via the runtime's WebAssembly engine. |
| Production deploy | `edge build` bundles the runtime, `compiler.wasm`, scripts, and imported packages into a self-contained `dist/` ([CLI](/reference/cli)). |

URL imports work for both `.py` and `.wasm` modules as long as your host supports them:

```python
from "https://example.com/utils.py" import normalize
from "https://example.com/math.wasm" import add
```

Or via packages.json alias:

```json
{ "imports": { "utils": "https://example.com/utils.py" } }
```

```python
from utils import normalize
```

### Integrity verification

Append `#sha256-<64 hex chars>` to any URL spec to require a content match:

```python
from "https://example.com/utils.py#sha256-deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567" import normalize
```

The compiler asks the host for raw bytes (`Resolver::fetch_bytes`), computes SHA-256, and refuses to compile on mismatch. The diagnostic surfaces both digests:

```text
integrity check failed for 'https://example.com/utils.py'
 expected sha256-deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567
 got sha256-feedface9876543210fedcba9876543210fedcba9876543210fedcba98765432
```

Verification lives in the compiler, not the host: every host inherits it uniformly. Hosts without `fetch_bytes` surface a clean "not supported" error rather than silently bypassing.

Only `sha256` supported. Other prefixes (`md5-...`, `sha384-...`) fail with `unrecognized integrity fragment`. The hex body must be exactly 64 chars.

## Lockfile and content-addressed cache

The browser worker auto-generates a lockfile and a content-addressed cache (both in IndexedDB). This is a side-effect of running scripts that import URLs. Repeat runs go to zero network round-trips. Upstream drift is detected on demand.

| File | Who writes it | Purpose |
|---|---|---|
| `packages.json` | the user | declares aliases (the manifest) |
| `packages.lock.json` (logical, in IDB) | the worker | records every fetched spec -> SHA-256 hash |
| `cas/<hash>` (per blob, in IDB) | the worker | bytes content-addressed by SHA-256 |

### What runs do

* **First run, cold**: every URL fetched, hashed, written to CAS, recorded in lockfile.
* **Subsequent runs**: each spec looked up in lockfile; bytes served from CAS without touching the network. Identical content under different URLs deduplicates (hash is the key).
* **`clearCache()`**: wipes map, CAS, lockfile.

### Drift detection

If a previously-locked URL serves different bytes than its recorded hash (upstream changed, or partial cache eviction triggered re-fetch), the worker fails with both digests visible:

```text
integrity drift for 'https://cdn.foo/kit/index.py'
  locked: sha256-abc123...
  remote: sha256-zzz999...
```

Same primitive as inline `#sha256-...` integrity, applied automatically to every imported URL. Explicit hashes in source are still honoured and fail at compile time before any code runs.

Non-browser hosts make their own caching choices: `Resolver` sees only `(spec -> Resolved)` and `(spec -> bytes)`.

## Sandbox

| Module type | Sandbox |
|---|---|
| Code modules (`.py`) | Full, code only calls host packages it imports |
| Native modules (`.wasm`) | Full, WASM isolates by construction; runs in its own linear memory |
| Host capabilities (custom embedder) | Trust boundary is the host process, runs with embedder privileges |

Scripts always execute inside the compiler's WASM sandbox. `.wasm` native modules add their own WASM layer. Host capabilities are part of the embedder's distribution.

## What doesn't work

- **Dynamic imports**: no `__import__`, no `importlib`. Module set is fixed per compilation. Use `import_module(name)` to dispatch among modules already statically imported.
- **Relative imports**: `from . import x` not supported. Use quoted relative specs (`from "./foo.py" import x`) or aliases in `packages.json`.
- **Dotted submodule auto-discovery**: `import a.b.c` parses but isn't auto-walked. Each spec must be declared in `packages.json` or supplied as a quoted path/URL.

## Errors

Resolution errors surface as parse-time diagnostics with the import statement's source position:

```text
error: module 'unknown' not found (no resolver configured)
   --> main.py:1:6
    |
  1 | from unknown import f
    |      ^^^^^^^

error: module 'json' has no export 'badname'
   --> main.py:2:6
    |
  2 | from json import badname
    |      ^^^^
```

Runtime errors from native bindings (e.g., `upper()` with a non-string argument) propagate normally as `VmErr`.

## See also

- [Official packages](/reference/packages): the ready-made modules and the default registry.
- [Writing modules](/reference/writing-modules): the three delivery paths for native modules.
- [WASM module ABI](/reference/wasm-abi): the wire contract behind `.wasm` imports.
