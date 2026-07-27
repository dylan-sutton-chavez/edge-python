# Edge Python Standard Packages

Official standard-library packages for [Edge Python](https://edgepython.com). Most are a Rust crate compiled to `wasm32-unknown-unknown` against the [wasm-pdk](https://github.com/dylan-sutton-chavez/edge-python/tree/main/pdk) ABI; hosts load the resulting `.wasm` over the standard plugin contract, no custom embedder, no Rust on the consumer side. A package can also ship as pure Edge Python source (`src/entry.py`), imported as a code module with no `cargo` build (e.g. `test`).

The folder name IS the package name. A native package builds to `<name>/target/wasm32-unknown-unknown/release/<name>.wasm` (when `<name>` is a Rust keyword the crate — and artifact — name differs, e.g. `struct` builds `edge_struct.wasm`; the runner accepts any single `.wasm` in `release/`); a pure-Python package has `src/entry.py` and no build artifact. Each package's `<name>.json` corpus sits in its folder; cases in it are automatically prefixed with `from <name> import *\n` before dispatch, so the corpus only contains the code being tested.

## Packages

| Folder | Description |
|--------|-------------|
| `json` | JSON serialization/deserialization, see [`json/README.md`](json/README.md) |
| `re` | Regular expressions, a subset with capture, backreferences, lookaround, and a ReDoS step budget, see [`re/README.md`](re/README.md) |
| `math` | CPython-style math over libm, integer ops, and a packed-f64 batch fast path, see [`math/README.md`](math/README.md) |
| `struct` | Pack values into fixed-width binary `bytes` driven by format strings, see [`struct/README.md`](struct/README.md) |
| `test` | Tiny unit-test harness in pure Edge Python (fixtures, `raises`, runner with exit code), see [`test/README.md`](test/README.md) |

## Build + test

Native packages build independently; the agnostic runner asserts against the produced `.wasm`, or against `src/entry.py` for a pure-Python package. From `std/`:

```bash
# Build a native package's .wasm artifact (skip for pure-Python packages like test).
( cd json && cargo build --release --target wasm32-unknown-unknown )

# One command, drives all corpora through the shared runner.
deno test --allow-all tests/
```

The runner discovers packages by walking `std/` for `<name>/<name>.json` corpora. CI runs this matrix via `.github/workflows/`.

## Adding a new stdpkg

For a native (wasm) package:

1. Create `<name>/` under `std/` with `Cargo.toml` (`name = "<name>"`, or a non-keyword variant like `edge-struct` when `<name>` is a Rust keyword, `crate-type = ["cdylib"]`, `wasm-pdk` dep) and a `src/lib.rs` exporting via `#[plugin_fn]`.
2. Drop `<name>/<name>.json` with the corpus (Edge Python source + expected `output` / `error` per case).
3. Run `cargo build --release --target wasm32-unknown-unknown` inside the package folder.
4. Run `deno test --allow-all tests/` from the repo root.

For a pure-Python package, skip the crate: add `<name>/src/entry.py` plus `<name>/<name>.json`, then run `deno test --allow-all tests/`. The runner routes `.py` packages to their source and skips the wasm build.

No edits to `tests/`.

## License

MIT OR Apache-2.0
