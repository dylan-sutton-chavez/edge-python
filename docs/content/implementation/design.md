---
title: "Design"
description: "Compiler architecture, dispatch model, and runtime layout."
---

## Overview

The compiler is a single pass. Source goes through a LUT-driven lexer, a Pratt parser that emits SSA-versioned bytecode directly, a peephole constant-folding optimiser, and a token-threaded interpreter with two layers of adaptive specialisation. There is no AST and no IR. Bytecode is the only intermediate representation.

The release build is around 200 KB on `wasm32-unknown-unknown` (`panic=abort`, `opt-level=z`, `lto=true`, `codegen-units=1`). The core is about 20,000 lines of Rust. Production dependencies are `hashbrown`, `itoa`, and `libm`, with SHA-256 implemented in-tree. The WASM build adds `dlmalloc` as the global allocator, so allocation cost stays flat as live blocks grow.

Classes support single and multiple inheritance with C3 linearization, `super()`, full dunder dispatch, and `@property` / `@x.setter`.

Deep dives live in their own pages. Tokenization is in [Lexical](/implementation/lexical). Parsing, SSA, and bytecode emission are in [Parsing](/implementation/parsing). This page covers the parts that span the pipeline.

## Key mechanisms

- **Token-threaded dispatch**: the hot loop is a flat `match` over `Vec<Instruction>`, which Rust lowers to a jump table. It is not direct threading, since computed goto is not available in safe Rust.
- **Per-instruction inline caching**: each binary op records operand type tags. After `QUICK_THRESH = 4` stable hits the cache stores a typed `FastOp` (`AddInt`, `AddFloat`, `AddStr`, `LtFloat`, `EqStr`, `ModInt`, and friends) as a speculative fast path. A type-guard miss invalidates the slot and falls back to the generic handler. Caching is per instruction, so monomorphic sites stabilise independently.
- **Template memoisation**: pure user functions cache `(args) -> result` after `TPL_THRESH = 2` hits, capped at 256 entries per function. A table self-disables and frees after 256 consecutive lookup misses. Caching requires a call without keywords, immutable arguments (mutable containers disqualify), and a pure body. Static purity is computed at parse time (see [Parsing](/implementation/parsing#lambda-and-function-bodies)) and backed by a runtime check that propagates effects through calls, so a statically pure wrapper over an impure callee such as `apply(print, x)` is never cached. Hashing is an FNV fold over the raw `Val` bits with a value-equality verify.
- **Call-time name propagation**: free names in a function body fill from the caller's frame at call time. The map is cached per `(caller chunk, callee)` pair as exact-name slot pairs plus bare-name version candidates. The fallback chain is caller SSA, then callee module attrs, then globals, then entry-module state.
- **NaN-boxed values**: `Val` is a 64-bit union holding 48-bit signed ints inline, IEEE-754 floats, bools, None, an undef sentinel, and 28-bit heap indices. See the memory model below.
- **Mark-and-sweep GC**: single-colour, no reference counts, cycles reclaimed natively.

## Bytecode shape

Each `Instruction` is 4 bytes. That is a 1-byte `OpCode` (`#[repr(u8)]`), a 2-byte operand, and 1 byte of padding. About 40 specialised `Call*` variants cover hot builtins. Operand meanings per opcode are listed in [Parsing](/implementation/parsing#bytecode-model).

## Dispatch shape

The hot loop reads `cache.fused_ref()[ip]`, a snapshot of the instruction stream with `LoadAttr` + `Call` sites fused into `CallMethod` + `CallMethodArgs`. Fusion happens after first dispatch when every argument is a single-push load, up to 8 arguments, never across a jump target. The arg loads shift down one slot so the pair sits adjacent at the call. Fusion runs once per chunk and is cached.

For arith and compare opcodes the loop checks `cache.get_fast(ip)`. A present `FastOp` runs inline without a function call.

`LoadConst` reads a pre-materialised `Vec<Val>` built on first dispatch. Inline-range ints stay inline. Ints from 2⁴⁷ to 2¹²⁷ allocate a `HeapObj::LongInt` slot. Literals beyond ±2¹²⁷ are rejected at parse time.

## Memory model

`Val` is 64 bits, NaN-boxed (`QNAN = 0x7FFC_0000_0000_0000`, `SIGN = 0x8000...`):

| Tag | Pattern | Notes |
|-----------|-----------------------------------------|--------------------------------------|
| Float | any non-canonical IEEE-754 | quiet NaNs remapped to `0x7FF8...` |
| Int | `QNAN \| SIGN \| i48` | 48-bit signed inline, promotes to `HeapObj::LongInt` (i128) on overflow |
| Undef | `QNAN` | unbound-local sentinel |
| None | `QNAN \| 1` | |
| True | `QNAN \| 2` | |
| False | `QNAN \| 3` | |
| Heap | `QNAN \| 4 \| (i28 << 4)` | 28-bit index into the heap arena, max `1 << 28` slots |

`INT_MAX = 140_737_488_355_327`, `INT_MIN = -140_737_488_355_328`. Inline ints cost one ALU op per arithmetic. Overflow promotes to `HeapObj::LongInt(i128)` and results demote back inline when they fit. LongInts are interned by value, so equal values share a heap index and stay `hash`/`eq` consistent. The hard cap is ±2¹²⁷. Wider results raise `OverflowError`. Arbitrary-precision bigints would need a limb vector with per-op heap allocation, or dropping NaN-boxing. Both regress the WASM-size and inner-loop goals.

Dicts and sets key by content via `hash_val_with_heap`, so value-equal numerics collapse to one key. For example `1 == 1.0` and `10**16 == 1e16` hit the same slot. An inline int (and any integral float in range) hashes as its `i64` value. Only non-integral floats hash their `f64` bits. Hashing float bits directly would funnel small integers, whose low mantissa bits are zero, into one `FxHasher` bucket and degrade int-keyed lookups to O(n²). `FxBuildHasher` uses a fixed seed, so iteration order is reproducible across runs.

The heap is a `Vec<HeapSlot>` arena with a free list capped at 524,288 entries, sorted to prefer low indices. Strings and bytes up to 128 bytes and all LongInts are interned in side hashes. Equal values collapse to one slot, so short literals short-circuit through identity (`is`). The live-object cap is `Limits.heap` (see [Limits and errors](/reference/limits-and-errors#sandbox-limits)).

The main `HeapObj` variants are `Str`, `Bytes`, `LongInt`, `List`, `Dict` (insertion-ordered), `Set`, `FrozenSet`, `Tuple`, `Func`, `Range`, `Slice`, `Type`, `ExcInstance`, `BoundMethod`, `NativeFn`, `Class`, `Instance`, `BoundUserMethod`, `Super`, `Property`, `StaticMethod`, `ClassMethod`, `Coroutine`, `Module`, and `Extern`.

## Garbage collection

Collection triggers when `live >= gc_threshold` or `alloc_count >= max(live / 4, 4096)`. After each sweep, `gc_threshold = max(live * 2, 512)`. Roots include the value stack, the with-stack, yields, the event queue, the pending `yield from` value, slots and live-slot snapshots, closure cells on active call frames, slot templates, globals, module state, parked scheduler coroutines, iterator frames, opcode-cache constants, active const pools, and memoisation entries.

## Coroutine dispatch

`async def` and `yield`-bearing `def` both compile to `HeapObj::Coroutine`. The user-facing primitives are described in [Async](/language/async).

A plain `def` inside a coroutine that calls a yielding builtin gets its state (`ip`, slots, stack and iterator deltas) snapshotted as a `SyncFrame` and pushed on the enclosing coroutine's `sync_frames`, innermost last. Resume walks this stack inside-out before re-entering the outer body, so each helper's return value lands at its original `Call` site.

`vm.run()` wraps the module body as an implicit coroutine, so top-level statements suspend like `async def` bodies. Dispatch is single-driver. `top_loop` is the only place that picks coroutines. `run`, `gather`, `with_timeout`, `await`, and calling a coroutine value push targets to the scheduler, park the caller in `CoroState::WaitingForChildren`, and yield. The `WaitKind` picks the finalize behavior. `Run(target)` returns the target's value, `Gather` returns the list of results, `Timeout` enforces a deadline. When children finish, the outer's saved stack placeholder is spliced with the result and the outer is marked ready, or the exception is injected into it.

Coroutines carry their own `try`/`except` frames across yields. On entry the stored frames are denormalised onto the live exception stack and renormalised on yield-save, so `try: run(coro) except E:` catches a child's raise across multiple resume cycles.

`with` invokes `__enter__` and `__exit__(exc_type, exc_val, traceback)`. A truthy `__exit__` return suppresses the exception. `async with` reuses the sync dunders.

## Snapshots

`save_state` serializes a parked VM into a self-contained versioned blob. `restore_state` replays it onto a VM freshly booted from the blob's own embedded source. `Val` bits are written verbatim and heap slots are restored at identical indices, so references, cycles, and interning survive with no remapping. The blob leads with a magic tag, a format version, and a structural fingerprint (FxHash) of the bytecode. Restore re-parses the embedded source and rejects any blob whose fingerprint does not match.

Preemption supplies the parked state when a program never suspends on its own. `set_preempt_interval(n)` makes the dispatch loop sample a counter at loop back-edges and raise the ordinary yield path every `n` hits, leaving the coroutine ready while `top_loop` returns `Preempted`. Sampling is gated on a per-frame `frame_safe` flag that only the dispatch `Call` path and the scheduler step ever set, so native re-entry is unpreemptible by default and a new re-entrant path inherits that default instead of silently corrupting a blob.

Chunk-derived tables (bytecode, name pools, the extern table) are not stored. They come from the re-parse, so only dynamic state crosses the wire. Restore runs in two passes because hashing reads the heap. First every slot is materialised (sets and frozensets land empty), then a rehash pass rebuilds dict indexes and fills the sets, and `rebuild_mro` recomputes each class linearization. IC and memoisation caches start empty and warm lazily. `Extern` handles resolve by name against the re-parsed chunk's extern table. Host-side resources such as in-flight host calls and DOM handles are not part of the snapshot. `state_globals` and `state_stack` introspect a parked run without resuming it.

See [Snapshots](/language/snapshots) for the host-facing feature and the [ABI](/reference/abi#snapshot-exports) for the exports and blob layout.

## What the compiler intentionally does not do

- No SSA-wide constant propagation through `LoadName`. The names stay so the IC, fusion, and memoisation paths stay fast.
- No CSE, GVN, LICM, inlining, branch DCE, or loop folding. The optimiser is constant folding plus phi-noop elimination plus dead-instruction compaction with jump-operand remap.
- No JIT. A method JIT needs per-arch stencils, and a tracing JIT duplicates the execution model and complicates the GC.
- No runtime module system. Imports resolve at parse time through a host-injected resolver. See [Modules](/reference/modules).
- No bigints, complex numbers, `bytearray`, `memoryview`, `Decimal`, or `Fraction`. No `gen.send` / `throw` / `close`. No `asyncio` module. Concurrency primitives are top-level builtins (see [Async](/language/async)).

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
