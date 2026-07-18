---
title: "WASM module ABI"
description: "The wire format a `.wasm` module must follow to be importable by Edge Python."
---

> **Sealed contract, plugin ABI v1.** Every signature, op code, tag, and error kind here is the public contract for CDN-distributed `.wasm` plugin modules (Path A). New host packages arrive as new `Op` values, never new imports; a future wire-level break would ship as `env_v2.*` without removing v1. Distinct from the `compiler<->host` interface embedders declare (see [host packages](/reference/writing-modules#path-b-host-capability)), embedders aren't bound by the 6-import limit here.

A `.wasm` module imported via `from "<url>" import <names>` follows the contract below.

The API is handle-based:

- The host owns all values.
- The guest sees only opaque `u32` handles.
- One dispatch primitive (`edge_op`) covers every operation.

New types, methods, and language features reach existing modules with no ABI change.

## Guest export shape

Every function the script can call is exposed as an `extern "C"` symbol (the `#[plugin_fn]` macro names free functions `__fn_<name>` and the host strips the prefix):

```rust
extern "C" fn <name>(argv: *const u32, argc: u32, out: *mut u32) -> i32;
```

| Field | Meaning |
|---|---|
| `argv` | Pointer (in **guest** linear memory) to an array of `argc` host-managed handles, one per positional argument. |
| `argc` | Positional argument count. |
| `out` | Pointer (in **guest** linear memory) where the guest writes ONE handle for the return value. |
| return | `0` = success, `1` = error (host pulls the error via `edge_take_error` immediately). |

`argv` handles are host-owned and live for the call. Handles the guest creates via `edge_encode` or `edge_op` are guest-owned until released. The guest must `edge_release` each before returning, except the one written into `*out`.

## Required guest exports

Every guest module MUST export, in addition to its user functions:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn __edge_alloc(size: u32) -> *mut u8;

#[unsafe(no_mangle)]
pub extern "C" fn __edge_abi_version() -> u32;
```

`__edge_alloc` lets the host stage `argv` arrays in guest linear memory before invoking each export.

Guests SHOULD also export `__edge_free(ptr: *mut u8, size: u32)` so the host can release that staging after each call. The host treats it as optional; without it, staging accumulates for the instance's lifetime.

`__edge_abi_version` returns the wire-format version (currently `1`). The host MUST read this once at instantiation and refuse unknown versions. Otherwise a v2 host would silently decode garbage from a v1 module.

At v1 every loader targets 1, so the bundled `compiler.wasm` shim does not yet read the symbol. The check becomes load-bearing when v2 ships.

The reference `wasm-pdk` crate emits both symbols automatically. `EDGE_ABI_VERSION` lives in the shared `wasm-abi` crate (no_std, zero deps) so host and every PDK read the same value.

## Host imports (6 functions)

Guest declares from `env`:

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

Wraps a primitive in a fresh handle (rc=1; release when done). `ptr`/`len` describe bytes in guest memory; the host copies.

### `edge_decode`

Writes the value's tag at `*out_tag` and copies bytes into `dst[..dst_max]`. Returns bytes copied (`>= 0`), or `-bytes_needed` if the buffer was too small (re-allocate and retry). On invalid handle or non-primitive, returns `0` with `*out_tag = 0xFFFFFFFF` (composites go through `edge_op`).

### `edge_release`

Decrement refcount. No-op for handle `0` or already-released.

### `edge_take_error`

Drain the most recent error from a `1`-returning `edge_op`. Writes kind at `*out_kind`, copies the UTF-8 message into `dst[..dst_max]`. Returns bytes copied (`>= 0`), `-bytes_needed` if the buffer was too small (error stays pending), or `-1` if no error pending.

### `edge_throw`

Stash an error visible after the guest returns `1`. Use it when an error did not originate from a `1`-returning `edge_op` (e.g., a typed `Result::Err` from user code). Overwrites any pending error. The guest must immediately return `1`.

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
| `IterNext` | 7 | `next(iter)` -> handle, or `1`+`StopIteration` on end |
| `NewDict` | 8 | construct empty dict; recv and name ignored, argc=0 -> handle |
| `NewList` | 9 | construct empty list; recv and name ignored, argc=0 -> handle |
| `TypeOf` | 10 | runtime type of recv -> handle (Str with the type name) |
| `NewTuple` | 11 | construct tuple from `argv` items; recv and name ignored -> handle |
| `NewSet` | 12 | construct set from `argv` items; unhashable item -> error |
| `NewFrozenSet` | 13 | construct frozenset from `argv` items; unhashable item -> error |

`Op::Iter` materialises the receiver into a List handle:

- set: items in hash-table iteration order
- dict: yields keys
- str: splits to single-char strings

`Op::IterNext` advances it.

`NewDict` / `NewList` construct empty composites. `NewTuple` / `NewSet` / `NewFrozenSet` construct from the `argv` items in one call.

`TypeOf` returns names matching the Python builtin: `"int"`, `"float"`, `"str"`, `"bytes"`, `"list"`, `"dict"`, `"set"`, `"tuple"`, `"NoneType"`, `"bool"`, `"object"` (user instance), etc.

Values `14..u32::MAX` are reserved. Old hosts return `1` with `kind=Runtime`.

## Tags (for `edge_encode` / `edge_decode`)

| Tag | Value | Layout |
|---|---|---|
| None | 0 | payload ignored |
| Bool | 1 | 1 byte (0/1) |
| Int | 2 | 16 bytes little-endian i128 |
| Float | 3 | 8 bytes IEEE 754 little-endian |
| Bytes | 4 | UTF-8 bytes -> `str`; non-UTF-8 is rejected |
| Raw | 5 | bytes -> `bytes`, no UTF-8 validation |

List and dict construct via `NewList` / `NewDict`. Tuple, set, and frozenset construct via `NewTuple` / `NewSet` / `NewFrozenSet`. Remaining composites (instance, callable, iterator) construct via `edge_op(Call, type_handle, ...)` and operate via indexing ops.

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
#![no_std] #![no_main]
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
wasm-pdk = { path = "../../wasm-pdk" }   # in-repo example; external authors use the git+tag form below
```

Build:

```bash
cargo build --release --target wasm32-unknown-unknown -p slugify-mod # -> target/wasm32-unknown-unknown/release/slugify_mod.wasm   (around 80 KB stripped)
```

### Exposing Rust structs as Python classes

`#[plugin_class]` + `#[plugin_methods]` expand to `__class_<Name>_<method>` exports. The host detects them by naming convention and synthesises a `HeapObj::Class`. State lives in a guest-side `BTreeMap<id, T>`. Each instance carries an `__rust_id` attribute the methods use to look itself up.

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

From Edge Python:

```python
from "./slugify_mod.wasm" import Slugger
s = Slugger()
s.add("Hello")
print(s.build()) # -> hello
```

Method returns of `T`, `Option<T>`, and `Result<T>` are all supported. Instances live until the worker run ends. There is no `__del__` dispatch.

### Exposing module constants

A `.wasm` exports only functions. A value attribute like `math.pi` ships as a zero-arg `#[plugin_const]` export. The host calls it once at import and binds the result as a module attribute (a value, not a callable):

```rust
#[plugin_const]
fn pi() -> f64 { core::f64::consts::PI }
```

```python
import math
print(math.pi) # -> 3.141592653589793 (a float, not a call)
from math import tau # also works under `from ... import *`
```

### Variadic functions

A trailing `Args` param captures every positional past the fixed ones, for `*args`-style signatures:

```rust
#[plugin_fn]
fn hypot(coords: Args) -> Result<f64> {
  let mut sum = 0.0;
  for h in &coords.0 { let x = f64::from_handle(h.raw())?; sum += x * x; }
  Ok(libm::sqrt(sum))
}
```

### Consuming `wasm-pdk` from your own crate

Not on crates.io. Depend from GitHub, pinned to a release tag:

```toml
[dependencies]
wasm-pdk = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.1.0" }
```

Cargo resolves `wasm-abi` and `wasm-pdk-macros` transitively. Pinning to a tag (vs `branch = "main"`) gives reproducible builds and a known wire-ABI version. Your module compiled against `wasm-pdk vX.Y.Z` is binary-compatible with the `compiler.wasm` of the same release. Bump `tag` + `cargo update -p wasm-pdk` to upgrade. Use `branch = "main"` only for unreleased iteration.

Use it from a script:

```python
from "./slugify_mod.wasm" import slugify, repeat_n, sum_ints

print(slugify("Hello World")) # -> hello-world
print(repeat_n("ha", 3)) # -> hahaha
print(sum_ints([1, 2, 3, 4])) # -> 10

try:
  print(repeat_n("nope", -1))
except ValueError as e:
  print("caught:", e) # -> caught: repeat count must be non-negative
```

## Worked example, raw, no SDK

Same module without the macro (for Zig / C / hand-written Rust):

### Rust source (`src/lib.rs`)

```rust
#![no_std] #![no_main]
extern crate alloc;
use alloc::{boxed::Box, vec};

#[global_allocator]
static A: lol_alloc::LeakingPageAllocator = lol_alloc::LeakingPageAllocator;
#[panic_handler] fn panic(_: &core::panic::PanicInfo) -> ! { core::arch::wasm32::unreachable() }

#[link(wasm_import_module = "env")]
unsafe extern "C" {
  fn edge_op(op: u32, recv: u32, name_ptr: *const u8, name_len: u32, argv_ptr: *const u32, argc: u32, out: *mut u32) -> i32;
  fn edge_encode(tag: u32, ptr: *const u8, len: u32) -> u32;
  fn edge_release(h: u32);
}

const OP_CALL: u32 = 0;
const TAG_BYTES: u32 = 4;

/// Required by the host shim for staging argv arrays.
#[unsafe(no_mangle)]
pub extern "C" fn __edge_alloc(size: u32) -> *mut u8 {
  Box::into_raw(vec![0u8; size as usize].into_boxed_slice()) as *mut u8
}

/// Releases an `__edge_alloc` buffer after the call; sizes must match.
#[unsafe(no_mangle)]
pub extern "C" fn __edge_free(ptr: *mut u8, size: u32) {
  if ptr.is_null() || size == 0 { return; }
  drop(unsafe { Box::from_raw(std::slice::from_raw_parts_mut(ptr, size as usize) as *mut [u8]) });
}

#[unsafe(no_mangle)]
pub extern "C" fn slugify(argv: *const u32, argc: u32, out: *mut u32) -> i32 {
  if argc != 1 { return 1; }
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

  // 3) Cleanup intermediate handles. The result handle in *out transfers to the host.
  unsafe { edge_release(space); edge_release(dash); edge_release(lower); }
  r
}
```

Same `Cargo.toml` as the `wasm-pdk` example (drop `wasm-pdk`, add `lol_alloc = "0.4"`). Imported from scripts the same way.

## How the host loads it

For `from "<url>" import <names>` with a `.wasm` URL, the host:

1. Fetches bytes, verifying any `#sha256-...` fragment.
2. Instantiates with the 6 host imports.
3. Walks the export table.
4. Marshals args as handles.
5. Propagates results.

Reference browser shim: [`runtime/worker/worker.js`](https://github.com/dylan-sutton-chavez/edge-python/blob/main/runtime/worker/worker.js). WASI hosts and Rust embedders mirror the shape.

## Constraints and caveats

- **Refcounted handles.** Guest releases every handle it creates via `edge_encode` / `edge_op` except the one returned through `*out`. Host releases argv.
- **`edge_decode` is primitives-only.** For `list`, `dict`, `set`, instances, use `edge_op` (e.g. `Call recv "items"`, `GetItem recv idx`).
- **Trailing kwargs slot.** Every plugin call carries one extra `u32` after the user's positional argv: handle `0` when the caller passed no `name=value` arguments, otherwise a `dict` handle holding the pairs. The `#[plugin_fn]` macro folds it into a `Kwargs` parameter if declared (`fn foo(a: Handle, kw: Kwargs)`). Otherwise it is silently absorbed and the function sees only its positional args.
- **Invoking a caller-supplied callable.** From the guest, `edge_op Call recv "__call__" argv` invokes `recv` directly. Lambdas, builtins, classes, and bound methods all route through the same dispatch the language uses. Use this to wire Python hooks like `default`, `object_hook`, `parse_int`.
- **Reentrance supported.** A guest's `edge_op` runs while the VM is paused on the script's `CallExtern`. Method dispatch routes through the same `vm/handlers/builtin_methods/` descriptor table the language uses internally. Adding a method there makes it visible to existing modules with no recompile.
- **Error-as-status, not panic.** Returning `1` does NOT abort the host. The host pulls the error and raises it as a typed Python exception.
- **Memory ownership.** Host only reads guest linear memory at well-defined copy points. Guest-internal allocations stay private.

## Author conveniences

The `wasm-pdk` crate (Plugin Development Kit), bundled in this repo, publishable independently of `compiler.wasm`, provides:

- `#[plugin_fn]`, typed Rust function -> wire-conformant export.
- `#[plugin_const]`, zero-arg fn -> module constant via the `__const_<name>` export convention; the host calls it once at import and binds the value as a module attribute.
- `#[plugin_class]` / `#[plugin_methods]` / `#[plugin_ctor]`, expose a Rust struct as a Python class via the `__class_<Name>_<method>` export convention.
- `module!()`, expands to `#[global_allocator]` + `#[panic_handler]`.
- `module_fixed_pool!()` (or `module_fixed_pool!(bytes)`), same but allocating from a fixed-size static pool that never calls `memory.grow`; used by the bundled `std` packages.
- `FromValue` / `IntoValue` with primitive impls (`i64`, `i128`, `f64`, `bool`, `String`, `&str`, `Bytes`, `Option<T>`, `Handle`). `i64` rejects out-of-range values with `ValueError`; use `i128` for the full range. `Bytes` maps to Python `bytes` over `tag::RAW`.
- `Handle` with `Drop`-driven release plus `call`, `get_attr` / `set_attr`, `get_item` / `set_item`, `len`, `iter` / `iter_next`, `new_dict` / `new_list`, `new_tuple` / `new_set` / `new_frozenset`, `type_of`.
- `Args`, trailing variadic positional params as borrowed handles; declare it as the last param before any `Kwargs`.
- `Kwargs`, thin wrapper around the trailing kwargs handle with `get::<T>(name)` for primitive kwargs and `get_handle(name)` for callables, tuples, dicts.
- `PluginCell<T>`, single-threaded interior mutability cell for static plugin state.
- `__edge_alloc` / `__edge_free` + `__edge_abi_version` emitted automatically.

The macro emits the worked-example boilerplate. Manual is around 25 lines for the first function, around 5 per additional.

Community PDKs (uncoordinated releases, each tracking the sealed wire spec): Zig (`wasm-pdk-zig`), AssemblyScript (`wasm-pdk-as`), C (`wasm-pdk.h`).

## See also

- [Imports](/reference/imports): how `from "..." import` resolves on the script side, including walk-up packages.json and integrity verification.
- [Writing modules](/reference/writing-modules): overview of the three module delivery paths.
