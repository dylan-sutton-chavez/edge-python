---
title: "Embedding"
description: "Build your own scripting language by embedding the Edge Python engine through the lang crate."
---

The `lang` crate is a small, clean embedding API over the Edge Python engine. Use it to build your own lightweight scripting language, compiling source to a program, exposing Rust functions to scripts, rewriting syntax into your own dialect, and running everything under the same metered sandbox the CLI uses. It wraps the compiler and VM behind a handful of types and leaks none of the internals.

## Add the dependency

`lang` lives in the workspace beside the engine. Pull it from GitHub, pinned to a tag for a reproducible build.

```toml
[dependencies]
lang = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.1.0" }
```

`lang` is `no_std` and needs only `alloc`. The default `std` feature links the host standard library, and `default-features = false` drops it for wasm and bare-metal targets.

## Compile and run

The whole surface is `Engine`, `Program`, `Instance`, `Value`, `Output`, `Error`, and `Limits`. An engine is built once and reused across many compiles.

```rust
use lang::Engine;

let engine = Engine::builder().build();

let program = engine.compile("print('hello')")?;
let out = program.run()?;

assert_eq!(out.text(), "hello\n");
```

`compile` returns `Err(Error::Compile(_))` with a rustc-style caret when the source fails to parse. `run` returns `Err(Error::Run(_))` with a traceback when the script raises or trips a sandbox cap. Only a `Program` can run, so running uncompiled source cannot be expressed.

## Call a function

Compile once, then invoke a named top-level function many times with different arguments. `call` runs each invocation on a fresh instance, so nothing leaks between them.

```rust
use lang::{Engine, Value};

let engine = Engine::builder().build();

let program = engine.compile("def check(n):\n  return n > 10")?;

assert_eq!(*program.call("check", &[Value::Int(21)])?.value(), Value::Bool(true));
assert_eq!(*program.call("check", &[Value::Int(3)])?.value(), Value::Bool(false));
```

That rebuilds the VM and reruns the module body every time. `start` pays for it once and hands back an `Instance` the calls share, which is the shape a rule applied over many inputs wants.

```rust
let mut inst = program.start()?;

assert_eq!(*inst.call("check", &[Value::Int(21)])?.value(), Value::Bool(true));
assert_eq!(*inst.call("check", &[Value::Int(3)])?.value(), Value::Bool(false));
```

Both return the same `Output` as `run`, so `.value()` is the return and `.text()` is what that call printed, never the module body. An unknown name or a raise surfaces as `Error::Run`.

## Expose Rust functions

`define` publishes a Rust closure under `host`, the only module that resolves. Every binding shares that one flat namespace, there are no others to author. Arguments arrive as `Value`, and the return crosses back the same way.

```rust
use lang::{Engine, Value};

let engine = Engine::builder()
    .define("double", |args| Ok(Value::Int(args.first().and_then(Value::as_int).unwrap_or(0) * 2)))
    .build();

let out = engine
    .compile("from host import double\nprint(double(21))")?
    .run()?;

assert_eq!(out.text(), "42\n");
```

Natives are positional only and take scalars or `str`. A keyword argument, or a list, dict or instance, raises a `TypeError` in the script instead of reaching the closure. Read `args` defensively, since the call site decides how many arrive, and keep a native under 16 arguments.

A closure returning `Err(String)` raises in the script. The text before the first colon names the exception class, so `Err("ValueError: bad input".into())` is catchable with `except ValueError`, while a bare `Err("nope".into())` raises a class called `nope`.

A native closure is `Fn + Send + Sync + 'static`. State it mutates across calls rides behind an `Arc<Mutex<_>>` it captures, updated before each call.

## Values

`Value` is an owned, heap-free enum that round-trips scalars losslessly and renders composite objects as text.

| Variant | Script type |
|---|---|
| `None` | `None` |
| `Bool` | `bool` |
| `Int` | `int` |
| `Float` | `float` |
| `Str` | `str` |
| `Object` | any list, dict, instance, rendered |

`Object` only ever comes back out of a result, never into a native. Scalars convert in through `From`, and `as_int`, `as_float`, `as_str` and `as_bool` read them back, with the two numeric accessors taking the whole `bool` to `int` to `float` tower. `Display` prints a value the way the language would.

## Custom syntax

Your dialect is a rewrite of source into valid Edge Python, applied before lexing. Two builder hooks cover it, and both are no-ops when unused.

`keyword` renames a single construct wherever it stands as its own word. It walks the token stream, so a longer identifier never matches and the same word inside a string or a comment is left untouched.

```rust
use lang::Engine;

let engine = Engine::builder()
    .keyword("función", "def")
    .keyword("imprime", "print")
    .build();

let out = engine
    .compile("función saluda():\n  imprime('hola')\nsaluda()")?
    .run()?;

assert_eq!(out.text(), "hola\n");
```

`transform` is the general hook, any `Fn(&str) -> String`, for sugar, macros, or a mini-DSL you translate down to Edge Python. Transforms chain, each fed the previous output.

```rust
use lang::Engine;

let engine = Engine::builder()
    .transform(|src| src.replace("@when", "if"))
    .build();

let out = engine
    .compile("x = 5\n@when x > 0:\n  print('positive')")?
    .run()?;

assert_eq!(out.text(), "positive\n");
```

Whatever a transform emits must be valid Edge Python. Rewrites change the surface, never the semantics, so you can rename `and` but not change that it binds looser than `==`, and diagnostics point at the rewritten source that `Program::source` returns. New language semantics belong in the engine, not a transform.

## Limits

Every run is metered. `Limits` reexports the engine caps for recursion depth, op budget, and heap quota. The default is `Limits::sandbox`, and tighter is allowed.

```rust
use lang::{Engine, Limits};

let engine = Engine::builder()
    .limits(Limits { calls: 64, ops: 1_000_000, heap: 50_000 })
    .build();
```

A script that exceeds a cap fails with an `Error::Run` traceback (`RecursionError`, `MemoryError`, or a budget `RuntimeError`) rather than hanging. The sandbox is the security boundary, so the caps are part of the contract, not an add-on. A script reaches nothing the embedder did not define, and `input()` raises instead of reading the host stdin.

## Concurrency

Within a run, `async`/`await` and cooperative tasks run on one thread, like an event loop, and `sleep` advances a deterministic virtual clock rather than a wall clock, so a run reproduces exactly. Across threads `Program` is not `Send`, so you scale share-nothing, one `Engine` and `Program` per thread, never shared.

## Where to go next

- [Limits and errors](/reference/limits-and-errors) details each cap and the errors they raise.
- [Modules](/reference/modules) covers the resolver and how imports reach the engine.
