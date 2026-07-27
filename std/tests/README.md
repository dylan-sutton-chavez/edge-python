# Tests

Shared test harness for every stdpkg, modeled on [`edge-python/runtime/tests`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/runtime/tests). `index.html` is pure markup: it loads the runtime's `<edge-python>` tag and nothing else. `std.test.js` boots one tag per package and drives it through the tag's programmatic API (`run`, `onOutput`).

## Layout

```
├── index.html
└── std.test.js
```

The `<pkg>/` folders and their `<pkg>.json` corpora stay agnostic to testing. The driver synthesizes the `packages.json` and prepends `from <pkg> import *\n`, so a corpus only holds the code under test.

## How it works

The driver serves the repo over `http://localhost` (a secure context, so the runtime's integrity check works), synthesizes a `packages.json` (the package wasm), and boots one `<edge-python packages="...">`. After its `ready` event it captures stdout with `el.worker.onOutput(...)` and runs each snippet with `el.worker.run(src)`, reading the trace for error cases. One worker is reused for the whole corpus, so the whole real system (runtime + `compiler.wasm` + the package wasm) is exercised end to end.

```bash
( cd json && cargo build --release --target wasm32-unknown-unknown ) # native packages only
deno test --allow-all tests/
```

`STDPKG=<name>` narrows discovery to a single package; CI uses it to fan out the matrix.

## Package kinds

The synthesized manifest follows the package's structure: a `src/entry.py` is imported as a code module; otherwise the built `<pkg>.wasm` is imported. The `.py` takes precedence when both exist.

| Has | Imported entry | Needs `cargo build` |
|---|---|---|
| `src/entry.py` | the `.py` (code module) | no |
| `Cargo.toml` + `src/lib.rs` (no `.py`) | the `<pkg>.wasm` | yes |

A pure-Python package (e.g. `test`) ships its source; native packages (`json`, `re`, `math`, `struct`) build to wasm first.

## Corpus shape

Each `<pkg>/<pkg>.json` is an array of cases. Per case:

| Field | Type | Purpose |
|---|---|---|
| `src` | string | Edge Python source. Driver prepends `from <pkg> import *\n`. |
| `output` | string[] | Expected stdout lines. |
| `error` | string | Expected substring of the trace (use instead of `output`). |
