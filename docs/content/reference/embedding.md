---
title: "Embedding"
description: "Build your own scripting language by embedding the Edge Python engine through the lang crate."
---

The `lang` crate is a small, clean embedding API over the Edge Python engine. Use it to build your own lightweight scripting language: compile source to a program, expose Rust functions to scripts, rewrite syntax into your own dialect, and run everything under the same metered sandbox the CLI uses. It wraps the compiler and VM behind six symbols and leaks none of the internals.

## Add the dependency

`lang` lives in the workspace beside the engine. Pull it from GitHub, pinned to a tag for a reproducible build.

```toml
[dependencies]
lang = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.1.0" }
```

The crate is `no_std` (it needs `alloc`), so it drops into embedded and wasm hosts as-is.

## Compile and run

The whole surface is `Engine`, `Program`, `Value`, `Output`, `Error`, and `Limits`. An engine is built once and reused across many compiles.

```rust
use lang::Engine;

let engine = Engine::builder().build();

let program = engine.compile("print('hello')")?;
let out = program.run()?;

assert_eq!(out.text(), "hello\n");
```

`compile` returns `Err(Error::Compile(_))` with a rustc-style caret when the source fails to parse. `run` returns `Err(Error::Run(_))` with a traceback when the script raises or trips a sandbox cap. Only a `Program` can run, so running uncompiled source cannot be expressed.

## Call a function

Compile once, then invoke a named top-level function many times with different arguments. Each call runs from a clean state, so nothing leaks between invocations, which suits a rule the host applies over many inputs.

```rust
use lang::{Engine, Value};

let engine = Engine::builder().build();

let program = engine.compile("def check(n):\n  return n > 10")?;

assert_eq!(*program.call("check", &[Value::Int(21)])?.value(), Value::Bool(true));
assert_eq!(*program.call("check", &[Value::Int(3)])?.value(), Value::Bool(false));
```

`call` runs the module body first to bind its definitions, then invokes the function. It returns the same `Output` as `run`, so `.value()` is the return and `.text()` is anything printed during the call. An unknown name or a raise surfaces as `Error::Run`. Arguments are `Value`, so scalars and strings pass directly.

## Expose Rust functions

`define` publishes a Rust closure to scripts under the `host` module. Arguments arrive as `Value`, and the return crosses back the same way.

```rust
use lang::{Engine, Value};

let engine = Engine::builder()
    .define("double", |args| Ok(Value::Int(args[0].as_int().unwrap_or(0) * 2)))
    .build();

let out = engine
    .compile("from host import double\nprint(double(21))")?
    .run()?;

assert_eq!(out.text(), "42\n");
```

A closure returning `Err(String)` raises in the script, caught by `try` or surfaced as an `Error::Run` traceback.

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

Scalars convert through the standard traits, so a native reads and writes plain Rust types.

```rust
use lang::Value;

let v = Value::from(21i64);
let n: i64 = v.try_into().unwrap();
assert_eq!(n, 21);
```

`as_int`, `as_float`, `as_str`, and `as_bool` are the borrowing accessors, and `Display` prints a value the way the language would.

## Custom syntax

Your dialect is a rewrite of source into valid Edge Python, applied before lexing. Two builder hooks cover it, and both are no-ops when unused.

`keyword` renames a single construct, whole-word so it never rewrites a substring of a longer name.

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

`transform` is the general hook: any `Fn(&str) -> String`, for sugar, macros, or a mini-DSL you translate down to Edge Python. Transforms chain, each fed the previous output.

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

Whatever a transform emits must be valid Edge Python. Rewrites change the surface, never the semantics: you can rename `and`, not change that it binds looser than `==`, and diagnostics point at the rewritten source. New language semantics belong in the engine, not a transform.

## Limits

Every run is metered. `Limits` reexports the engine caps for recursion depth, op budget, and heap quota. The default is `Limits::sandbox`, and tighter is allowed.

```rust
use lang::{Engine, Limits};

let engine = Engine::builder()
    .limits(Limits { calls: 64, ops: 1_000_000, heap: 50_000 })
    .build();
```

A script that exceeds a cap fails with an `Error::Run` traceback (`RecursionError`, `MemoryError`, or a budget `RuntimeError`) rather than hanging. The sandbox is the security boundary, so the caps are part of the contract, not an add-on.

## Concurrency

Within a run, `async`/`await` and cooperative tasks run on one thread, like an event loop. Across threads, `Program` is not `Send`: you scale share-nothing, one `Engine` and `Program` per thread, never shared.

```rust
use std::thread;
use lang::Engine;

let src = "def rule(n):\n  return n > 10";
thread::scope(|s| {
    for batch in inputs.chunks(chunk_size) {
        s.spawn(move || {
            let program = Engine::builder().build().compile(src).unwrap();
            for &v in batch {
                program.call("rule", &[v.into()]).unwrap();
            }
        });
    }
});
```

## Where to go next

- [ABI](/reference/abi) is the lower-level plugin contract, for shipping native modules over the CDN or `dlopen`.
- [Modules](/reference/modules) covers the resolver and how imports reach the engine.
- [Limits and errors](/reference/limits-and-errors) details each cap and the errors they raise.
