---
title: "Design"
description: "Compiler architecture, dispatch model, and runtime layout."
---

## Overview

The release build is around 200 KB on `wasm32-unknown-unknown` (`panic=abort`, `opt-level=z`, `lto=true`, `codegen-units=1`). The pipeline: LUT-driven lexer -> single-pass Pratt parser emitting SSA-versioned bytecode directly -> peephole constant-folding optimiser -> token-threaded interpreter with two layers of adaptive specialisation.

No AST, no IR. Bytecode is the only intermediate representation. Around 17,000 lines of Rust. Production deps are `hashbrown`, `itoa`, and `libm` (SHA-256 in-tree). The WASM build adds `lol_alloc` for a single-threaded free-list allocator.

Classes support single and multiple inheritance (C3 MRO), `super()`, full dunder dispatch, `@property` / `@x.setter`. Multi-paradigm: composition preferred, monomorphic dispatch optimised via instance-dunder IC.

## Concepts

- **Offset-based tokens**: `(kind, line, start, end)` indices into the source buffer. No string copies during lexing; parser slices lazily.
- **Single-pass SSA codegen**: Variables versioned per assignment (`x_1`, `x_2`). Control-flow joins emit `Phi` opcodes resolved at runtime.
- **Token-threaded dispatch**: `Vec<Instruction>` where each is `(opcode: OpCode, operand: u16)`. Hot loop is a flat `match`; Rust lowers it to a jump table. Not direct threading (computed-goto isn't available in safe Rust).
- **Per-instruction inline caching**: Each binary op records operand type tags. After `QUICK_THRESH = 4` stable hits the IC stores a typed `FastOp` (`AddInt`, `AddFloat`, `AddStr`, `LtFloat`, `EqStr`, `ModInt`, ...) as a speculative fast path with type-guard deopt.
- **Template memoisation**: pure user functions cache `(args) -> result` after `TPL_THRESH = 2` hits, capped at 256 entries each; a function's table self-disables and frees after 256 consecutive lookup misses. Gated on no-kw calls, byte-stable args (mutable containers disqualify), and an impurity-free body (purity detection in [Syntax](/implementation/syntax#lambda-and-function-bodies)). Static purity is backed by a runtime impurity check that propagates effects through calls, so a statically-pure wrapper over an impure callee (e.g. `apply(print, x)`) is never cached. Hashing is an FNV-like fold over raw `Val.0` bits with a value-eq verify.
- **Call-time name propagation**: free names in a function body fill from the caller's frame at call time through per-`(caller chunk, callee)` cached maps (exact-name slot pairs plus bare-name version candidates), then a bare-name fallback (caller SSA -> callee module attrs -> globals -> entry-module state).
- **NaN-boxed values**: `Val` is a 64-bit union: 47-bit signed ints (inline), IEEE-754 floats (NaNs canonicalised), bools, None, an undef sentinel, and 28-bit heap indices.
- **Mark-and-sweep GC**: Triggered when `live >= gc_threshold` or `alloc_count >= max(live/4, 4096)`. After each sweep `gc_threshold = max(live * 2, 512)`. Roots: stack, with-stack, yields, event queue, the pending `yield from` return value, slots and live-slot snapshots, slot templates, globals, every iterator frame's `iter_stack`, opcode-cache constants, active const pools, function templates.

## Bytecode shape

Each `Instruction` is 4 bytes: 1-byte `OpCode` (`#[repr(u8)]`), 2-byte operand, 1 byte padding. Opcodes span 17 categories: load, store, arith, bitwise, compare, logic, identity, control flow, iter, build, container, comprehension, function, ssa (Phi), yield, side effects, unsupported (raises at runtime). Around 40 specialised `Call*` variants cover hot builtins. `LoadAttr + Call` sites fuse into `CallMethod + CallMethodArgs` after first dispatch when every argument is a single-push load (up to 8, never across a jump target).

```text
OpCode::LoadConst -> operand = constant index
OpCode::LoadName  -> operand = name slot
OpCode::StoreName -> operand = name slot
OpCode::Add / Sub -> operand = 0 (IC slot derived from ip)
OpCode::Call      -> operand = (kw << 8) | pos
OpCode::Phi       -> operand = target slot, sources in chunk.phi_sources
OpCode::ForIter   -> operand = jump target on iterator exhaustion
```

## Dispatch shape

Hot loop reads `cache.fused_ref()[ip]`, a snapshot of the instruction stream with `LoadAttr + Call` sites fused into `CallMethod + CallMethodArgs` (arg loads shift down one slot so the pair sits adjacent at the call). Fusion runs once per chunk, cached.

For arith/compare opcodes, the loop checks `cache.get_fast(ip)`. If a `FastOp` is present, it runs inline without a function call. A type-guard miss invalidates the slot and falls back to the generic handler. IC is per-instruction, so monomorphic sites stabilise independently.

`LoadConst` reads a pre-materialised `Vec<Val>` (`OpcodeCache::const_vals`) built on first dispatch. Inline-range ints (47-bit) stay inline. Ints from 2⁴⁷ to 2¹²⁷ allocate a `HeapObj::LongInt` slot. Literals beyond ±2¹²⁷ are rejected at parse time.

## Memory model

`Val` is 64 bits NaN-boxed (`QNAN = 0x7FFC_0000_0000_0000`, `SIGN = 0x8000...`):

| Tag | Pattern | Notes |
|-----------|-----------------------------------------|--------------------------------------|
| Float | any non-canonical IEEE-754 | Quiet NaNs remapped to `0x7FF8...` |
| Int | `QNAN \| SIGN \| i48` | 47-bit signed inline; auto-promotes to `HeapObj::LongInt` (i128) on overflow |
| Undef | `QNAN` | Unbound-local sentinel |
| None | `QNAN \| 1` | |
| True | `QNAN \| 2` | |
| False | `QNAN \| 3` | |
| Heap | `QNAN \| 4 \| (i28 << 4)` | 28-bit index into `HeapPool` (max `1 << 28` slots) |

`INT_MAX = 140_737_488_355_327`, `INT_MIN = -140_737_488_355_328`. Inline ints take one ALU op per arithmetic. Overflow promotes to `HeapObj::LongInt(i128)` until results fit inline again. LongInts are interned by value, so equal values share a heap index (consistent `hash`/`eq`). The hard cap is ±2¹²⁷; wider raises `OverflowError`. Arbitrary-precision bigints would need a `Vec<u32>`-limb variant (heap-allocs per op, Knuth D / Karatsuba code) or dropping NaN-boxing. Both regress WASM-size and inner-loop goals.

`PartialEq` / `Hash` for `Val` reconcile value-equal numerics, so `1 == 1.0` and `hash(1) == hash(1.0)`. Dicts and sets key by content via `hash_val_with_heap` (same rule, extended to `LongInt` and to tuples/frozensets), so value-equal numerics — including `10**16 == 1e16` — and content-equal compound keys collapse to one key. It writes an inline int (and any integral float in range) as its `i64` value, falling back to `f64` bits only for non-integral floats. Hashing the `f64` bits directly would funnel small integers (whose low mantissa bits are zero) into one `FxHasher` bucket and collapse int-keyed dict/set to O(n²). `FxBuildHasher` uses a fixed seed for reproducible iteration order across runs.

Heap is a `Vec<HeapSlot>` arena with a free list (capped 524,288, sorted to prefer low indices). Strings, bytes (<=128 B), and LongInts are interned in side hashes. So equal values collapse to one slot and short literals short-circuit through identity (`is`); dict/set lookups stay correct across allocations via content hashing, not interning. Live-object cap is `Limits.heap` (default 10M, sandbox 100K). Single-colour mark-and-sweep, no refcount, cycles reclaimed natively.

`HeapObj` variants: `Str`, `Bytes`, `List` (`Rc<RefCell<Vec<Val>>>`), `Dict` (insertion-ordered), `Set`, `FrozenSet`, `Tuple`, `Func(fn_idx, defaults, captures)`, `Range`, `Slice`, `Ellipsis` (singleton, distinct from `'...'`), `Type`, `ExcInstance`, `BoundMethod`, `NativeFn`, `Class(name, members)`, `Instance(class, attrs)`, `BoundUserMethod(recv, fn)`, `Coroutine(ip, slots, stack, body, iter_stack, sync_frames, exception_frames)` (shared by generators, `async def`, and the implicit module-body coro; `body` is `BodyRef::Fn(usize)` or `BodyRef::Module`; `sync_frames` stacks suspended sync sub-calls so a plain `def` hitting a yielding builtin can resume mid-body; `exception_frames` carries the coro's own `try`/`except` blocks across yields), `Module(spec, attrs)`, `Extern(Arc<dyn Fn>)`.

## What the compiler intentionally does not do

- No SSA-wide constant propagation through `LoadName` (preserved to keep IC, super-op, template paths fast).
- No CSE, GVN, LICM, inlining, branch DCE, loop folding. Optimiser is constant folding + phi-noop elimination + dead-instruction compaction with jump-operand remap.
- No dead-store elimination beyond what falls out of constant folding.
- No IR, one representation between source and dispatch.
- No JIT (single-tier, pure Rust). Method JITs need per-arch stencils; trace JITs duplicate the execution model and complicate GC.
- No runtime module system: imports resolve at parse time through a host-injected `Resolver`. See [Imports](/reference/imports).
- No bigints, complex numbers, `bytearray`, `memoryview`, `Decimal`, `Fraction`. No `gen.send` / `throw` / `close`. No `asyncio` module: concurrency primitives are top-level builtins ([Async](/language/async)).

## Coroutine and context-manager dispatch

`async def` and `yield`-bearing `def` both produce `HeapObj::Coroutine`. `run()` drives the scheduler; the other primitives are top-level builtins ([Async](/language/async)).

A plain `def` inside a coroutine that calls a yielding builtin gets its state (`ip`, slots, stack/iter deltas) snapshotted as a `SyncFrame`, pushed on the enclosing Coroutine's `sync_frames` (innermost-last). `resume_coroutine` walks this stack inside-out before re-entering the outer body, so each helper's return value lands at the original `Call` site. Otherwise the outer's `resume_ip` would skip past the unfinished helper.

`vm.run()` wraps the module body as an implicit coroutine with `BodyRef::Module`. Top-level statements suspend like `async def` bodies. Dispatch is single-driver: `top_loop` is the only place that picks coros. `run` / `gather` / `with_timeout`, `await`, and calling a coroutine value are non-driving. They push targets to the scheduler, park the outer in `CoroState::WaitingForChildren { tasks, kind: WaitKind }`, and yield (`await` / coroutine-call use `WaitKind::Run`, so they resolve to the target's value even across suspension). `WaitKind` picks finalize behavior: `Run(target)` returns its value, `Gather` returns the list of results, and `Timeout { deadline_ns, target }` enforces a deadline. `wake_waiting_outers` (gated by `waiting_for_children_count`) drops terminal children, splices the result into the outer's saved stack placeholder, and marks the outer `Ready`. Or `raise_into_outer` injects the exception.

Coroutines carry their own `exception_frames` (7th tuple field). On entry, `resume_coroutine` denormalises stored depths (relative to `saved_stack_len` / `saved_iter_len`), pushes them onto the live `exception_stack`, and renormalises on yield-save. `dispatch.rs::exec` honors `pending_exec_exc_base`, so handler search includes restored frames. Net: `try`/`except` survives yields. `try: run(coro) except E:` catches a child's raise across multiple `run_resume` cycles. `SyncFrame.exception_delta` does the same for sync-helper try blocks spanning a yield.

`with` invokes `__enter__` / `__exit__(exc_type, exc_val, traceback)`. A truthy `__exit__` return suppresses. `async with` reuses sync `__enter__` / `__exit__` (no async dunders).

## Snapshots

`save_state` serializes a parked VM, every coroutine suspended and the Rust stack unwound, into a self-contained versioned blob; `restore_state` replays it onto a VM freshly booted from the blob's own embedded source. `Val` bits are written verbatim and heap slots restored at identical indices, so references, cycles, and interning survive with no remapping. The blob leads with a magic tag, a format version, and an FNV-1a fingerprint of the bytecode structure (instruction stream, constant and name pools, nested function / class / import chunks); restore rejects any blob whose re-parsed source does not reproduce that fingerprint, pinning each blob to one program and one compiler build.

Chunk-derived tables (bytecode, name pools, the extern table) are not stored: they come from the re-parse, only dynamic state crosses the wire. Restore runs in two passes because hashing reads the heap: first every slot is materialised (sets and frozensets land empty), then a rehash pass rebuilds dict indexes and fills the sets, and `rebuild_mro` recomputes each class linearization. IC and format-template caches start empty and warm lazily; the MRO cache is repopulated during restore. `Extern` handles resolve by name against the re-parsed chunk's extern table; host-side resources (in-flight host calls, DOM handles) are not part of the snapshot. `state_globals` / `state_stack` introspect a parked run without resuming it. See [Snapshots](/language/snapshots) for the host-facing feature and the [WASM module ABI](/reference/wasm-abi#snapshot-exports) for the exports and blob layout.

## References

1. Aho, Sethi & Ullman. *Compilers: Principles, Techniques and Tools* (1986). LUT-based lexer.
2. Pratt. *Top Down Operator Precedence* (POPL 1973).
3. Cytron et al. *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991).
4. Gudeman. *Representing Type Information in Dynamically Typed Languages* (1993). NaN-boxing.
5. Deutsch & Schiffman. *Efficient Implementation of the Smalltalk-80 System* (POPL 1984). Inline caching.
6. Ertl & Gregg. *The Structure and Performance of Efficient Interpreters* (JILP 2003). Threaded dispatch.
7. Casey et al. *Towards Superinstructions for Java Interpreters* (SCOPES 2003). LoadAttr+Call fusion.
8. Michie. *Memo Functions and Machine Learning* (Nature 1968). Pure-function memoization.
9. McCarthy. *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep GC.
10. Backus. *Can Programming Be Liberated from the von Neumann Style?* (CACM 1978). Function-level paradigm.
