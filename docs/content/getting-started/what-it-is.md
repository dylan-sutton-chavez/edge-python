---
title: "What Edge Python is"
description: "A subset of Python, compiled to bytecode and run on a sandboxed VM."
---

A sandboxed Python subset. Classes, async/await, structural pattern matching, and `packages.json` imports.

Compiled to bytecode. Run on a stack VM with adaptive inline caching and pure-function memoisation. See [Design](/implementation/design) for internals.

It reads like Python: it parses Python syntax. It runs differently. What it executes is curated.

## What it supports

- **First-class functions**: pass them, return them, store them in lists and dicts. Decorators apply to both `def` and `class`.
- **Lambdas with closures**: full lexical capture via shared cells, so sibling closures and `nonlocal` writes see each other; currying, partial application.
- **Generators and coroutines**: `yield`, `yield from`, `async def`, `await`. Generator expressions are eagerly materialised to lists; use a `def` with `yield` for true laziness.
- **Comprehensions**: list, dict, and set, with multiple `for` clauses and `if` guards.
- **Pattern matching**: `match` / `case` with literals, captures, OR-patterns, guards, and sequence patterns (one star permitted).
- **Exceptions**: `try` / `except` / `else` / `finally`, named handlers, `raise X from Y` (chain info discarded but `X` is what propagates), and subclass-aware matching (`except Exception` catches `RuntimeError`).
- **Context managers**: `with` and `async with` invoke `__enter__` / `__exit__` on the context-manager value; a truthy return from `__exit__` suppresses the raised exception.
- **Protocol dunders**: operator overloading, indexing, iteration, hashing, and `repr` / `str` / `format` dispatch through user-defined dunders; see [Dunders](/language/dunders) for the full matrix.
- **Numbers**: bounded integers (fast inline path, auto-promoting to wide; see [Integer width](/reference/limits-and-errors#integer-width)) and full IEEE-754 floats. No `complex`, `Decimal`, or `Fraction`.
- **Sequences**: lists, tuples, dicts (insertion-ordered), sets, frozensets, ranges, strings (UTF-8, codepoint-indexed), and bytes.
- **f-strings**: full grammar, embedded expressions, `{expr=}` self-doc, `!r` / `!s` / `!a` conversions, and format specs covering `s d b o x X f F e E g G n % c` plus fill / align / sign / `#` / `0` / width / `,` / precision.
- **Walrus operator**: `:=` in expressions (Name target only).
- **Type annotations**: parsed and discarded, no runtime `__annotations__`, no enforcement.
- **Module identity**: `__name__` is bound to `"__main__"` in the entry chunk and to the module's spec inside imported modules, so the canonical `if __name__ == "__main__":` guard works as expected.
- **Modules**: `import`, `from <spec> import names`, and `from <spec> import *` resolve at parse time through a host-injected resolver, with optional `#sha256-<hex>` integrity on URL specs. Two flavors: `.py` source modules and native modules. See [Imports](/reference/imports) for resolution semantics, [Writing modules](/reference/writing-modules) for the three delivery paths, and [Official packages](/reference/packages) for the ready-made modules (`json`, `dom`, `network`, `storage`, and more).
- **State snapshots**: a parked program can be serialized whole, heap, globals, and suspended coroutines included, then restored later, even on a fresh page load, to continue from the pause point. A program parks on its own (`receive()`, `sleep()`, a host call) or because the host called `pause()`, which stops even a bare `while True:` that never suspends. See [Snapshots](/language/snapshots).

## What it doesn't support

These parse for syntactic compatibility. They raise at runtime, or don't exist:

- **Standard library**: no bundled stdlib. Every module is external (see **Modules*- above).
- **I/O**: `input()` reads from a host-provided buffer. No file system, no network, no `os`, no `sys`. These surface only when the host runtime registers them as [host capabilities](/reference/writing-modules#path-b-host-capability), the same mechanism behind `print` and `input` themselves.
- **Async surface**: `async def` creates real coroutines, and the VM runs a cooperative scheduler. No `asyncio` module; the primitives are top-level builtins ([Async](/language/async)).
- **Metaclasses, descriptor protocol, `__slots__`**: not modeled.
- **Dynamic code**: no `exec`, no `eval`, no `compile`, no `__import__` (use the `import_module(name)` builtin to look up an already-imported module by alias).
- **Reflection**: nothing beyond `type`, `id`, `hash`, `repr`, `callable`, `getattr`, `setattr`, `hasattr`, `delattr`, `vars`, `globals`, `locals`, `isinstance`, and `issubclass`. `dir` is absent.
- **Relative imports**: `from . import x` is not supported; use quoted relative specs or `packages.json` aliases ([Imports](/reference/imports)).

## Design philosophy

Multi-paradigm sandboxed compiler:

- **Small binary**, compiler + VM in one WebAssembly release.
- **Faster interpreter**, no method-resolution overhead; hot opcodes promote to type-specialised fast paths via IC.
- **Aggressive memoisation**, pure functions auto-cached after two hits ([Functions](/language/functions#recursion)); most functional code is pure by construction.
- **Easier sandboxing**, no protocol dispatch, no stdlib; attack surface is the fixed built-in set.

## Sandbox guarantees

Inherits WASM-host guarantees: no syscalls, no FS, no network, isolated linear memory. On top, embedders enforce per-VM caps via `Limits::sandbox()`. Hits raise recoverable `RuntimeError` / `MemoryError` / `RecursionError` rather than crashing the host. See [Limits and errors](/reference/limits-and-errors#sandbox-limits).

## Where it runs

One `.wasm` artifact (`compiler.wasm`, a ~200 KB release) runs anywhere WebAssembly does:

- **Browser**: served alongside the [`js/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js) JS package, which bridges `print()` and module imports across the `WASM <-> JS` boundary. The host runtime owns I/O, fetching, and module loading.
- **Embedded Rust apps**: load `compiler.wasm` via your runtime of choice or use the `compiler` rlib when cargo-linked.

Two ABIs sit on top:

- **`Compiler <-> host` imports**, embedder-declared, covering output, module fetching, native dispatch, wall-clock time. Custom embedders that ship [host capabilities](/reference/writing-modules#path-b-host-capability) declare additional imports (DOM, FS) without touching the plugin ABI.
- **Plugin ABI (sealed v1)**, contract for CDN-distributed `.wasm` plugin modules. Exactly 6 `env.*` imports, never extended. See the [WASM module ABI](/reference/wasm-abi).
