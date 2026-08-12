---
title: "ABI"
description: "The wire contract a plugin module must follow to be importable by Edge Python."
---

> **Sealed contract, plugin ABI v1.** Every signature, op code, tag, and error kind here is the public contract for plugin modules, shipped as `.wasm` over CDN or loaded natively via `dlopen`. New host packages arrive as new `Op` values, never new imports. A future wire-level break would ship as `env_v2.*` without removing v1. This contract is distinct from the `compiler<->host` interface embedders declare for [host capabilities](/reference/modules#host-capability), which are not bound by the 6-import limit here.

A plugin module imported via `from "<url>" import <names>` follows the contract below, compiled to `.wasm` for the CDN or to a native library for the CLI. The API is handle-based: the host owns all values, the guest sees only opaque `u32` handles, and one dispatch primitive (`edge_op`) covers every operation. New types, methods, and language features reach existing modules with no ABI change.

## Guest export shape

Every function the script can call is exposed as an export with this signature:

```rust
extern "C" fn <name>(argv: *const u32, argc: u32, out: *mut u32) -> i32;
```

| Field | Meaning |
|---|---|
| `argv` | Pointer (in **guest** linear memory) to an array of `argc` host-managed handles: one per positional argument, plus a trailing kwargs slot. |
| `argc` | Positional argument count plus one, the trailing kwargs slot (handle `0` when no `name=value` arguments were passed). |
| `out` | Pointer (in **guest** linear memory) where the guest writes ONE handle for the return value. |
| return | `0` = success, `1` = error (host pulls the error via `edge_take_error` immediately), `2` = deferred (the host parks the coroutine and routes the result in later by call id). |

`argv` handles are host-owned and live for the call. Handles the guest creates via `edge_encode` or `edge_op` are guest-owned until released. The guest must `edge_release` each before returning, except the one written into `*out`.

The `#[plugin_fn]` macro exports free functions as `__fn_<name>` and the host strips the prefix. It also recognizes the `__const_<name>` and `__class_<Name>_<method>` conventions described below. Plain-named exports work too.

## Required guest exports

Every guest module MUST export, in addition to its user functions:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn __edge_alloc(size: u32) -> *mut u8;

#[unsafe(no_mangle)]
pub extern "C" fn __edge_abi_version() -> u32;
```

`__edge_alloc` lets the host stage `argv` arrays in guest linear memory before invoking each export.

Guests SHOULD also export `__edge_free(ptr: *mut u8, size: u32)` so the host can release that staging after each call. The host treats it as optional, and without it staging accumulates for the instance's lifetime.

`__edge_abi_version` returns the wire-format version (currently `1`). The CLI's native loader reads it and refuses a mismatch. The browser shim does not read it yet, because every loader targets version 1. The check becomes load-bearing when v2 ships.

The reference `wasm-pdk` crate emits all three symbols automatically. `EDGE_ABI_VERSION` lives in the shared `wasm-abi` crate (no_std, zero deps) so the host and every PDK read the same value.

## Host imports (6 functions)

The guest declares these from `env`:

```rust
fn edge_op(op: u32, recv: u32, name_ptr: *const u8, name_len: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32;

fn edge_encode(tag: u32, ptr: *const u8, len: u32) -> u32;

fn edge_decode(h: u32, out_tag: *mut u32, dst: *mut u8, dst_max: u32) -> i32;

fn edge_release(h: u32);

fn edge_take_error(out_kind: *mut u32, dst: *mut u8, dst_max: u32) -> i32;

fn edge_throw(kind: u32, msg_ptr: *const u8, msg_len: u32);
```

### `edge_op`

Universal dispatch. Returns `0` with a fresh handle in `*out` on success, `1` on error.

### `edge_encode`

Wraps a primitive in a fresh handle (rc=1, release when done). `ptr`/`len` describe bytes in guest memory, and the host copies. Returns `0` on an invalid tag or when called outside a run.

### `edge_decode`

Writes the value's tag at `*out_tag` and copies bytes into `dst[..dst_max]`. Returns bytes copied (`>= 0`), or `-bytes_needed` if the buffer was too small (the tag is still written, so re-allocate and retry). On an invalid handle or a non-transit value (set, instance, cyclic composite), returns `0` with `*out_tag = 0xFFFFFFFF`. Walk those with `edge_op`.

### `edge_release`

Decrements a refcount. No-op for handle `0` or an already-released handle.

### `edge_take_error`

Drains the most recent error from a `1`-returning call. Writes the kind at `*out_kind` and copies the UTF-8 message into `dst[..dst_max]`. Returns bytes copied (`>= 0`), `-bytes_needed` if the buffer was too small (the error stays pending), or `-1` if no error is pending.

### `edge_throw`

Stashes an error visible after the guest returns `1`. Use it when the error did not originate from a `1`-returning `edge_op` (for example a typed `Result::Err` from user code). Overwrites any pending error. The guest must immediately return `1`.

## Op codes

| Op | Value | Meaning |
|---|---|---|
| `Call` | 0 | `recv.<name>(args...)` -> handle |
| `GetAttr` | 1 | `recv.<name>` -> handle |
| `SetAttr` | 2 | `recv.<name> = args[0]` -> handle (None) |
| `GetItem` | 3 | `recv[args[0]]` -> handle |
| `SetItem` | 4 | `recv[args[0]] = args[1]` -> handle (None) |
| `Len` | 5 | `len(recv)` -> handle (Int) |
| `Iter` | 6 | `iter(recv)` -> handle (iterator List) |
| `IterNext` | 7 | `next(iter)` -> handle, or `1` + `StopIteration` on end |
| `NewDict` | 8 | construct empty dict, recv and name ignored, argc=0 -> handle |
| `NewList` | 9 | construct empty list, recv and name ignored, argc=0 -> handle |
| `TypeOf` | 10 | runtime type of recv -> handle (Str with the type name) |
| `NewTuple` | 11 | construct tuple from `argv` items, recv and name ignored -> handle |
| `NewSet` | 12 | construct set from `argv` items, unhashable item -> error |
| `NewFrozenSet` | 13 | construct frozenset from `argv` items, unhashable item -> error |

`Op::Iter` materialises the receiver into a List handle: a set yields items in hash-table order, a dict yields keys, a str splits to single-char strings. `Op::IterNext` advances it.

`TypeOf` returns the builtin type names: `"int"`, `"float"`, `"str"`, `"bytes"`, `"list"`, `"dict"`, `"set"`, `"tuple"`, `"NoneType"`, `"bool"`, `"object"` (user instance), and so on.

Values `14..u32::MAX` are reserved. Old hosts return `1` with kind `Runtime`.

## Tags (for `edge_encode` / `edge_decode`)

| Tag | Value | Layout |
|---|---|---|
| None | 0 | payload ignored |
| Bool | 1 | 1 byte (0/1) |
| Int | 2 | 16 bytes little-endian i128 |
| Float | 3 | 8 bytes IEEE 754 little-endian |
| Bytes | 4 | UTF-8 bytes -> `str`, non-UTF-8 is rejected |
| Raw | 5 | bytes -> `bytes`, no UTF-8 validation |
| List | 6 | TLV: `count:u32le`, then `count` nodes -> `list` |
| Dict | 7 | TLV: `count:u32le`, then `count` key,value node pairs -> `dict` |

### Composite transit (TLV)

`List` / `Dict` payloads nest values as TLV nodes, each framed `tag:u32le len:u32le payload[len]` with any transit tag inside. A dict crosses whole in one call.

- Dict keys must be `str`. Any other key makes the whole value non-transit.
- Nesting caps at depth 32. Malformed or deeper input is rejected (`edge_encode` returns handle `0`).
- Cyclic values cannot serialize. `edge_decode` reports them as non-transit (`0xFFFFFFFF`).
- `tuple` flattens to `List` on the wire.

The canonical codec is `wasm_abi::WireValue` (`encode_body` / `decode_body`), shared by the compiler and `wasm-pdk`.

Sets and frozensets construct via `NewSet` / `NewFrozenSet`. Remaining composites (instance, callable, iterator) construct via `edge_op(Call, type_handle, ...)` and operate through the indexing ops.

## Error kinds (for `edge_take_error`)

| Kind | Value | Maps to |
|---|---|---|
| Type | 0 | `TypeError` |
| Value | 1 | `ValueError` |
| Runtime | 2 | `RuntimeError` |
| Attribute | 3 | `AttributeError` |
| Index | 4 | `IndexError` |
| Key | 5 | `KeyError` |
| Custom | 6 | the message carries the user-defined kind name |

## Worked example, recommended Rust path with `wasm-pdk`

The `wasm-pdk` crate provides the `#[plugin_fn]` proc macro that expands to wire-conformant exports. Authors write normal Rust:

```rust
// slugify-mod/src/lib.rs
#![no_std]
#![no_main]
extern crate alloc;

use alloc::string::String;
use wasm_pdk::*;

wasm_pdk::module!(); // expands to #[global_allocator] + #[panic_handler]

#[plugin_fn]
fn slugify(s: String) -> String {
  s.to_lowercase().replace(' ', "-")
}

#[plugin_fn]
fn repeat_n(s: String, n: i64) -> Result<String> {
  if n < 0 { return Err(Error::Value("repeat count must be non-negative".into())); }
  Ok(s.repeat(n as usize))
}

#[plugin_fn]
fn sum_ints(items: Handle) -> Result<i64> {
  let it = items.iter()?;
  let mut total: i64 = 0;
  while let Some(item) = it.iter_next()? {
    total += i64::from_handle(item.raw())?;
  }
  Ok(total)
}
```

`Cargo.toml`:

```toml
[package]
name = "slugify-mod"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
wasm-pdk = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.1.0" }
```

Pinning to a tag (rather than `branch = "main"`) gives reproducible builds and a known wire-ABI version. A module compiled against `wasm-pdk` from release `vX.Y.Z` is binary-compatible with the `compiler.wasm` of the same release. Bump `tag` and run `cargo update -p wasm-pdk` to upgrade.

Build:

```bash
cargo build --release --target wasm32-unknown-unknown
# -> target/wasm32-unknown-unknown/release/slugify_mod.wasm
```

Use it from a script, through a `packages.json` alias:

```json
{ "imports": { "slugify_mod": "./slugify_mod.wasm" } }
```

```python
from slugify_mod import slugify, repeat_n, sum_ints

print(slugify("Hello World"))   # hello-world
print(repeat_n("ha", 3))        # hahaha
print(sum_ints([1, 2, 3, 4]))   # 10

try:
  print(repeat_n("nope", -1))
except ValueError as e:
  print("caught:", e)           # caught: repeat count must be non-negative
```

### Exposing Rust structs as classes

`#[plugin_class]` + `#[plugin_methods]` expand to `__class_<Name>_<method>` exports. The host detects them by naming convention and synthesises a class. State lives in a guest-side `BTreeMap<id, T>`, and each instance carries an `__rust_id` attribute the methods use to look themselves up.

```rust
#[plugin_class]
pub struct Slugger { parts: Vec<String> }

#[plugin_methods]
impl Slugger {
  #[plugin_ctor]
  pub fn new() -> Self { Self { parts: Vec::new() } }
  pub fn add(&mut self, s: String) { self.parts.push(s.to_lowercase()); }
  pub fn build(&self) -> String { self.parts.join("-") }
  pub fn pop(&mut self) -> Option<String> { self.parts.pop() }
  pub fn repeat(&self, n: i64) -> Result<String> {
    if n < 0 { return Err(Error::Value("n must be non-negative".into())); }
    Ok(self.parts.join("-").repeat(n as usize))
  }
}
```

From Edge Python, same manifest alias:

```python
from slugify_mod import Slugger
s = Slugger()
s.add("Hello")
print(s.build())   # hello
```

Method returns of `T`, `Option<T>`, and `Result<T>` are all supported. Instances live until the run ends. There is no `__del__` dispatch.

### Exposing module constants

A `.wasm` exports only functions. A value attribute like `math.pi` ships as a zero-arg `#[plugin_const]` export. The host calls it once at import and binds the result as a module attribute, a value rather than a callable:

```rust
#[plugin_const]
fn pi() -> f64 { core::f64::consts::PI }
```

```python
import math
print(math.pi)   # a float, not a call
```

```text Output
3.141592653589793
```

### Variadic functions

A trailing `Args` param captures every positional argument past the fixed ones, for `*args`-style signatures:

```rust
#[plugin_fn]
fn hypot(coords: Args) -> Result<f64> {
  let mut sum = 0.0;
  for h in &coords.0 { let x = f64::from_handle(h.raw())?; sum += x * x; }
  Ok(libm::sqrt(sum))
}
```

## Worked example, raw, no SDK

The same module without the macro, for Zig, C, or hand-written Rust:

```rust
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{boxed::Box, vec};

#[global_allocator]
static A: lol_alloc::LeakingPageAllocator = lol_alloc::LeakingPageAllocator;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { core::arch::wasm32::unreachable() }

#[link(wasm_import_module = "env")]
unsafe extern "C" {
  fn edge_op(op: u32, recv: u32, name_ptr: *const u8, name_len: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32;
  fn edge_encode(tag: u32, ptr: *const u8, len: u32) -> u32;
  fn edge_release(h: u32);
}

const OP_CALL: u32 = 0;
const TAG_BYTES: u32 = 4;

/// Required by the host for staging argv arrays.
#[unsafe(no_mangle)]
pub extern "C" fn __edge_alloc(size: u32) -> *mut u8 {
  Box::into_raw(vec![0u8; size as usize].into_boxed_slice()) as *mut u8
}

/// Releases an `__edge_alloc` buffer after the call; sizes must match.
#[unsafe(no_mangle)]
pub extern "C" fn __edge_free(ptr: *mut u8, size: u32) {
  if ptr.is_null() || size == 0 { return; }
  drop(unsafe { Box::from_raw(core::slice::from_raw_parts_mut(ptr, size) as *mut [u8]) });
}

#[unsafe(no_mangle)]
pub extern "C" fn slugify(argv: *const u32, argc: u32, out: *mut u32) -> i32 {
  if argc != 2 { return 1; } // 1 positional + trailing kwargs slot
  let input = unsafe { *argv };

  // 1) input.lower()
  let mut lower: u32 = 0;
  if unsafe { edge_op(OP_CALL, input, b"lower".as_ptr(), 5, core::ptr::null(), 0, &mut lower) } != 0 {
    return 1;
  }

  // 2) lower.replace(" ", "-")
  let space = unsafe { edge_encode(TAG_BYTES, b" ".as_ptr(), 1) };
  let dash  = unsafe { edge_encode(TAG_BYTES, b"-".as_ptr(), 1) };
  let argv2 = [space, dash];
  let r = unsafe { edge_op(OP_CALL, lower, b"replace".as_ptr(), 7, argv2.as_ptr(), 2, out) };

  // 3) Release intermediate handles. The result handle in *out transfers to the host.
  unsafe { edge_release(space); edge_release(dash); edge_release(lower); }
  r
}
```

Same `Cargo.toml` as the `wasm-pdk` example, minus `wasm-pdk`, plus `lol_alloc = "0.4"`. Plain-named exports like this `slugify` are picked up as free functions. The module imports from scripts the same way.

## How the host loads it

For `from <name> import <names>` where the manifest maps `<name>` to a `.wasm` URL, the host:

1. Fetches the bytes, verifying any `#sha256-...` fragment.
2. Instantiates with the 6 host imports.
3. Walks the export table.
4. Marshals args as handles.
5. Propagates results.

The reference browser shim is `web/src/native.js` in the repo. The CLI's [native engine](/reference/modules#the-native-engine) implements the same six imports over `dlopen`. WASI hosts and Rust embedders mirror the shape.

## Constraints and caveats

- **Refcounted handles.** The guest releases every handle it creates via `edge_encode` / `edge_op`, except the one returned through `*out`. The host releases `argv` handles.
- **`edge_decode` covers primitives plus `list` / `tuple` / `dict`** (TLV-encoded). Sets, instances, and cyclic values return `TAG_INVALID`. Walk those with `edge_op`.
- **Trailing kwargs slot.** Every plugin call carries one extra `u32` after the positional argv: handle `0` when the caller passed no `name=value` arguments, otherwise a `dict` handle holding the pairs. The `#[plugin_fn]` macro folds it into a `Kwargs` parameter if declared (`fn foo(a: Handle, kw: Kwargs)`). Otherwise it is silently absorbed.
- **Invoking a caller-supplied callable.** From the guest, `edge_op(Call, recv, "__call__", ...)` invokes `recv` directly. Lambdas, builtins, classes, and bound methods all route through the same dispatch the language uses. Use this for hooks like `default`, `object_hook`, `parse_int`.
- **Reentrance supported.** A guest's `edge_op` runs while the VM is paused on the script's `CallExtern`. Method dispatch routes through the same method table the language uses internally, so adding a method there makes it visible to existing modules with no recompile.
- **Error-as-status, not panic.** Returning `1` does not abort the host. The host pulls the error and raises it as a typed exception.
- **Memory ownership.** The host reads guest linear memory only at well-defined copy points. Guest-internal allocations stay private.
- **ABI v1 leaks about 8 bytes per host call** in guest linear memory. A single worker session caps at roughly 500 k plugin calls, so recycle the worker periodically for unbounded streaming. The bundled std packages recycle their own per-call memory through a static pool and stay flat.

## Author conveniences

The `wasm-pdk` crate (Plugin Development Kit), bundled in this repo and publishable independently of `compiler.wasm`, provides:

- `#[plugin_fn]`: typed Rust function -> wire-conformant export.
- `#[plugin_const]`: zero-arg fn -> module constant via the `__const_<name>` export convention.
- `#[plugin_class]` / `#[plugin_methods]` / `#[plugin_ctor]`: expose a Rust struct as a class via the `__class_<Name>_<method>` export convention.
- `module!()`: expands to `#[global_allocator]` + `#[panic_handler]`.
- `module_fixed_pool!()` (or `module_fixed_pool!(bytes)`): the same, but allocating from a fixed-size static pool (4 MiB by default) that never calls `memory.grow`. The bundled `std` packages use it.
- `FromValue` / `IntoValue` with primitive impls (`i64`, `i128`, `f64`, `bool`, `String`, `&str`, `Bytes`, `Option<T>`, `Handle`, `Value`). `i64` rejects out-of-range values with `ValueError`, so use `i128` for the full range. `Bytes` maps to `bytes` over the `Raw` tag. `Vec<Value>` and `Vec<f64>` cross a whole sequence in one TLV transit instead of per-item ops.
- `Handle` with `Drop`-driven release plus `call`, `get_attr` / `set_attr`, `get_item` / `set_item`, `len`, `iter` / `iter_next`, `new_dict` / `new_list`, `new_tuple` / `new_set` / `new_frozenset`, `type_of`.
- `Args`: trailing variadic positional params as borrowed handles. Declare it as the last param before any `Kwargs`.
- `Kwargs`: thin wrapper around the trailing kwargs handle, with `get::<T>(name)` for primitive kwargs and `get_handle(name)` for callables, tuples, and dicts.
- `PluginCell<T>`: single-threaded interior mutability cell for static plugin state.
- `__edge_alloc` / `__edge_free` / `__edge_abi_version` emitted automatically.

The macro emits the worked-example boilerplate. Writing it manually costs about 25 lines for the first function and about 5 per additional one.

Community PDKs (uncoordinated releases, each tracking this sealed spec): Zig (`wasm-pdk-zig`), AssemblyScript (`wasm-pdk-as`), C (`wasm-pdk.h`).

## Snapshot exports

Distinct from the sealed plugin imports above, these are exports on `compiler.wasm` itself, part of the host-driver surface an embedder calls to freeze and revive a paused run. The host-facing feature is [Snapshots](/language/snapshots). They reuse the linear-memory buffers and the packed status word of the run lifecycle (`run_start` / `run_resume` / `run_push_event`).

| Export | Signature | Meaning |
|---|---|---|
| `save_state` | `() -> i64` | Serialise the parked run into an internal buffer. Returns the blob length, or `-1` when nothing is parked. |
| `snapshot_ptr` | `() -> *const u8` | Pointer to the blob left by the last `save_state`. |
| `restore_state` | `(len: usize) -> u32` | Boot a VM from a blob staged in the source buffer and overlay its state. Returns the same packed status word as `run_start`. |
| `state_globals` | `() -> usize` | Write the parked run's module-level bindings as JSON into the out buffer. Returns its byte length. |
| `state_stack` | `() -> usize` | Write the parked run's coroutines as JSON into the out buffer. Returns its byte length. |
| `set_preempt_interval` | `(n: u32)` | Yield `PREEMPTED` every `n` loop back-edges so a program with no suspension point stays snapshottable. Defaults to `0`, disabled. Applies to the next `run_start` / `restore_state`. |

Buffers are the run lifecycle's: `src_ptr()` (1 MiB input), `out_ptr()` (1 MiB output), and `snapshot_ptr()` for the blob.

- **Save.** Drive to a pause (`run_start`, then `run_resume` until a `PENDING_*` status), call `save_state()`, and read that many bytes at `snapshot_ptr()` when the result is non-negative.
- **Preempt.** With a non-zero `set_preempt_interval`, `run_start` / `run_resume` also return kind `7` (`PREEMPTED`). The run is parked and snapshottable, and needs no host action. Call `run_resume` to continue, or `save_state()` first to freeze a program that never suspends on its own.
- **Restore.** Boot a fresh instance and register the same host modules (the embedded source is re-parsed, so its imports must resolve), write the blob into `src_ptr()`, then call `restore_state(len)` and drive it with `run_resume` like any other run.
- **Inspect.** Call `state_globals()` or `state_stack()` and read that many UTF-8 bytes at `out_ptr()`, one JSON value each.

### Blob layout

Little-endian, self-contained, versioned.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic, `0x4E535045` |
| 4 | 4 | format version, currently `1` |
| 8 | 8 | fingerprint, structural hash of the bytecode |
| 16 | 8 | source length in bytes |
| 24 | N | source, UTF-8 |
| 24+N | rest | serialised VM state (heap, stacks, scheduler, pending) |

`restore_state` re-parses the embedded source, recomputes the fingerprint, and rejects any blob whose fingerprint does not match the freshly compiled chunk. This pins each blob to one program and one compiler build. The whole blob must fit the 1 MiB source buffer. The serializer is `src/vm/snapshot.rs` in the repo, with internals in [Design](/implementation/design).

## Consuming the release from a Rust crate

The `edge-python` crate builds as a `cdylib`. A Rust host can instantiate `compiler.wasm` and call the exports above directly, the same `.wasm` that ships to browsers, with the host owning I/O. The crate builds no wasm of its own and fetches nothing at build time, so `cargo build` stays offline and reproducible. Take the artifact from the tagged GitHub Release, or from `https://cdn.edgepython.com/compiler.wasm` for the current `main`.

```toml
# Downstream Cargo.toml
[dependencies]
edge-python = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.1.0" }
```

Vendor the matching `compiler.wasm` next to your own sources and pin it by checksum, the same way the CLI pins native plugins it downloads. A release asset is immutable and the CDN path is not, so a checksum is the only thing that ties a build to a known engine.

To add native modules from a Rust host, implement the `Resolver` trait. See [Modules](/reference/modules).

## See also

- [Modules](/reference/modules): import resolution on the script side, packages.json, integrity verification, and the delivery paths.
- [Snapshots](/language/snapshots): freezing and resuming a paused run from the host.
