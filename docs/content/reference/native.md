---
title: "Native port"
description: "Host-less native builds of the engine and std packages, their release assets, runtime contract, and source builds."
---

Each tagged GitHub Release attaches native aarch64 builds alongside `compiler.wasm`, and the CDN serves the same assets under `https://cdn.edgepython.com/native/`, refreshed on every push to `main`. No host runtime ships with them yet. The executable runs with engine defaults plus a real wall clock, and the std libraries wait for an embedding host to load them.

| Asset | Contents |
| --- | --- |
| `edge-aarch64.tar.gz` | `edge-aarch64`, the engine as a host-less script runner |
| `json-aarch64.so` `re-aarch64.so` `math-aarch64.so` `struct-aarch64.so` | std packages as native plugin libraries |

## The runner

`edge-aarch64 file.py` parses and runs one script. The exit-code contract matches [`edge run`](/reference/cli#edge-run-run-a-python-file). Output streams to stdout, an uncaught error prints its traceback to stderr and exits 1, and `raise SystemExit(code)` exits cleanly with that code. Everything executes under [sandbox limits](/reference/limits-and-errors#sandbox-limits). A wall clock drives the scheduler, so `sleep()` and timeouts wait in real time, matching the web runtime.

The missing host shows up in three places.

- Every `import` fails with `module not found (no resolver configured)`, so there is no module loading of any kind ([imports](/reference/imports)).
- [`input()`](/reference/builtins#input) drains piped stdin, one line per call, and an empty buffer raises `RuntimeError`.
- `frame()`, `receive()`, and native-module calls park the scheduler with no host to resume it, and the runner exits 1 with `script suspended awaiting ...`.

## std packages as native libraries

The `.so` assets are the same crates shipped as `.wasm` [plugin modules](/reference/writing-modules), compiled for the host platform instead. Each exports the [plugin ABI](/reference/wasm-abi) surface (`__edge_abi_version`, `__fn_<name>`, the allocator pair) and leaves the six `env` imports (`edge_op`, `edge_encode`, ...) undefined for the embedding host to provide at load time.

## Building from source

```bash
cargo build --profile native --no-default-features --features native --bin edge-native
cd std/json && cargo build --profile native # any std package
```

The `native` profile inherits `release` and raises `opt-level` to 3. The release profile tunes for `.wasm` size at a real runtime cost, a trade that makes no sense off the wire. CI builds each target in [`actions/native`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/.github/actions/native) and adds the same nightly size flags as the `.wasm` release, which cuts the runner about 22% and the std libraries about 3x versus the plain commands above.
