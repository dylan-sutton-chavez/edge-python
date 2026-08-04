---
title: "Native engine"
description: "The CLI's default in-process engine, its module resolution, built-in host modules, snapshots, and the std plugin libraries."
---

`edge run`, `edge repl`, and `edge test` execute in-process by default: no browser, no local server, millisecond startup. The [`--web` flag](/reference/cli#global-flags) restores the Chromium-driven runtime for scripts that need the browser. Web-only surface fails fast with a pointer — `import dom` and a parked `frame()` both report `requires the web runtime (run with --web)`.

Everything runs under [sandbox limits](/reference/limits-and-errors#sandbox-limits) with a real wall clock, so `sleep()` and timeouts wait in real time, matching the web runtime.

## Module resolution

Imports resolve like the web runtime. Quoted paths load from disk relative to the importing file, bare names go through the [`packages.json`](/reference/imports#packagesjson) walk-up (`--packages <f>` replaces it), and `https://` specs (including `#sha256-` integrity fragments) download once into `~/.cache/edge-native`. A `.so` target loads as a native plugin; the official std `.wasm` specs written by `edge add` swap transparently to their `.so` twin, so one manifest serves both engines. A downloaded plugin is pinned by a `.lock` sidecar on first fetch, like the browser lockfile; later runs refuse on drift until the cache entry is removed. With no manifest entry, `json`, `re`, `math`, and `struct` default to the CDN `.so` for the host architecture and `test` to the CDN `test.py`.

Two modules are built into the binary, mirroring the [web host packages](/reference/packages) where a native equivalent makes sense. `time` carries the clocks (`time`, `time_ns`, `monotonic`, `perf_counter`, `sleep`) and the UTC calendar functions (`gmtime`, `localtime`, `mktime`, `strftime`, `strptime`, `asctime`, `ctime`; there is no timezone database, so `tzname()` is `"UTC"`). `network` exposes the blocking `fetch(url, options_json?)` with the web reply shape (`{id, ok, status, headers, body}`) plus `fetch_text` and `fetch_json`. A manifest entry with the same module name always wins.

## Run flags

| Flag | Effect |
| --- | --- |
| `--events <f>` | Each line (file or FIFO) feeds one `receive()`; end of input parks the script |
| `--save-state <f>` | On a wait the engine cannot serve, write a [snapshot](/language/snapshots) and exit 0 |
| `--restore-state <f>` | Boot from a snapshot instead of a script and keep running |
| `--preempt <n>` | Yield every `n` loop back-edges and resume, mirroring the web preempt interval |

## std packages as native libraries

Each tagged GitHub Release attaches the std packages as native plugin libraries for aarch64 and x86_64, as `.so` for Linux and `.dylib` for macOS (`json-<arch>.so`, `json-<arch>.dylib`, ...), and the CDN serves the same assets under `https://cdn.edgepython.com/native/<contract>/` (the runtime contract the CLI was built for, `0.1.0` today), refreshed on every push to `main`. They are the same crates shipped as `.wasm` [plugin modules](/reference/writing-modules), compiled for the host platform, with a glibc 2.17 floor on Linux. Each exports the [plugin ABI](/reference/wasm-abi) surface (`__edge_abi_version`, `__fn_<name>`, the allocator pair) and leaves the six `env` imports (`edge_op`, `edge_encode`, ...) undefined. The CLI provides them itself, re-exporting the `edge_*` symbols through `-rdynamic` so `dlopen` binds a plugin straight to the engine's shared bridge. An embedding host can do the same, or supply its own implementations. Either way the engine is single-threaded by contract: the bridge keeps its handles, error stash, and live VM pointer in process-wide statics, so the VM and every `edge_*` call must stay on one thread.

CLI releases carry the same glibc 2.17 floor (linked through `cargo-zigbuild`), so Linux ships one build rather than a static fallback: a fully static binary links musl's stub `dlopen`, which always fails, and a build that silently drops the native engine is worse than one that says so. The installer checks your glibc against that floor and, when the host is older or musl-based, explains the floor and points at `cargo install --path cli` instead of installing something that cannot load plugins.

## Building from source

```bash
cargo clippy --lib --features native # lint the engine module
cd cli && cargo build --release # the CLI embeds the native engine
cd std/json && cargo build --profile native # any std package as a .so
```

The `native` profile inherits `release` and raises `opt-level` to 3. The release profile tunes for `.wasm` size at a real runtime cost, a trade that makes no sense off the wire. CI builds each std target in [`actions/native`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/.github/actions/native) through `cargo-zigbuild` and adds the same nightly size flags as the `.wasm` release. End-to-end coverage lives in `cli/tests/native.rs`.
