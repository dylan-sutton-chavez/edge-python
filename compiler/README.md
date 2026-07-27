# Edge Python

A compact single-pass SSA bytecode compiler and stack VM for a sandboxed Python subset. Hand-written lexer, Pratt parser that emits bytecode directly (no AST), and a threaded-code interpreter with dual inline caching (scalar + instance-dunder), super-instruction fusion, and pure-function template memoization. Deterministic execution; around 200 KB WASM release.

* **Docs:** [edgepython.com](https://edgepython.com/)

## Architecture

Single-pass pipeline: source -> SSA bytecode chunk; stack interpreter with adaptive inline caching and pure-function memoization.

* **Lexer** (`modules/lexer/`) LUT-driven, offset-based tokens.
* **Parser** (`modules/parser/`) Pratt precedence, SSA-versioned bytecode with `Phi` at joins, no AST.
* **Optimizer** (`modules/vm/optimizer.rs`) constant folding, Phi-noop elimination, dead-code compaction.
* **VM** (`modules/vm/`) flat-match dispatch, scalar + instance-dunder inline caches, pure-function template memoization, NaN-boxed 64-bit `Val` with a mark-and-sweep arena.
* **Resolver** (`modules/packages/`) host-injected; native imports register for `CallExtern` dispatch.

Full rationale, NaN-box patterns, IC thresholds, GC roots, and intentional omissions: [Design](https://edgepython.com/implementation/design). Lexer and parser internals: [Lexical](https://edgepython.com/implementation/lexical), [Syntax](https://edgepython.com/implementation/syntax).

## Layout

```text
├── fuzz-afl
├── src
│   ├── main
│   ├── modules
│   │   ├── lexer
│   │   ├── packages
│   │   ├── parser
│   │   └── vm
│   │       ├── builtins
│   │       ├── handlers
│   │       │   └── builtin_methods
│   │       └── types
│   └── util
└── tests
    └── cases
```

## Quick start

```bash
cargo wasm # release WASM artifact -> target/wasm32-unknown-unknown/release/compiler.wasm
cargo test --release --no-default-features # host-side test suite (skips the prebuilt wasm download, only needed by external consumers)
```

`cargo wasm` is a workspace alias (`.cargo/config.toml`) for `cargo build --release --target wasm32-unknown-unknown -p edge-python`. Plain `cargo build --release` produces host artifacts (`.rlib` + cdylib) for embedders linking `compiler`. To add native modules from your own crate, implement the `Resolver` trait, see [Writing modules](https://edgepython.com/reference/writing-modules).

The test suite (`tests/`, fixtures in `tests/cases/vm.json`) runs every case under `Limits::sandbox()`, not the default `none()`. The budget, heap, and call-depth guards short-circuit under `none` (`sandbox_off`), so only the bounded profile exercises them — that way a regression that lets a loop run unbounded, recurse without limit, or materialise an oversized collection becomes a failing `MemoryError` / `RecursionError` assertion instead of a hang. Every fixture must stay within the sandbox budget.

The host runtime owns I/O, network, and module fetching. Browser hosts use the [`runtime/`](../runtime/) JS package; the [`cli/`](../cli/) `edge` binary drives that same runtime through headless Chromium; Rust embedders instantiate `compiler.wasm` directly.

### Consuming the release from another Rust crate

This crate declares `links = "compiler"` and its `build.rs` downloads the matching `compiler.wasm` from the GitHub Release for `CARGO_PKG_VERSION` into `OUT_DIR`. Downstream crates read the absolute path through `DEP_COMPILER_WASM`.

```toml
# Downstream Cargo.toml
[dependencies]
edge-python = { git = "https://github.com/dylan-sutton-chavez/edge-python", tag = "v0.2.4" }
```

```rust
// Downstream build.rs
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = std::env::var("DEP_COMPILER_WASM").expect("`DEP_COMPILER_WASM` unset, upstream `edge-python` must declare `links = \"compiler\"`");
    std::fs::copy(&wasm, "runtime/compiler.wasm").expect("copy failed");
}
```

The download URL is derived from `CARGO_PKG_VERSION`, so a tag bump is the only retarget. Use `branch = "main"` for unreleased work. Requires `curl` on PATH; gated by the default-on `prebuilt` feature.

## Fuzzing

Coverage-guided fuzzing of the lex -> parse -> VM pipeline lives in [`fuzz-afl/`](fuzz-afl/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust.

```bash
cd compiler/fuzz-afl
./seeds.sh # generate corpus from vm.json, copy dict.txt -> edge.dict (once)
cargo afl build --release # instrument on stable, no nightly
cargo afl fuzz -i in -o out -x edge.dict target/release/afl-pipeline # runs until Ctrl-C; add -V 300 to stop after 300s

cargo afl whatsup out # status summary of the ./out campaign; run in another terminal while fuzzing
```

`./deploy.sh` runs a parallel campaign across the host cores (one instance per logical core by default; override with `JOBS`; one `-M` plus N-1 `-S` instances sharing `out/`), `compose.yml` runs the same in a container with findings persisted in a volume, and `.github/workflows/fuzzer.yml` runs the target daily in CI. `deploy.sh`/compose/CI write findings to `out/m0/`, `out/s1/`, etc. (a bare `cargo afl fuzz` uses `out/default/`).

Seeds are generated from `tests/cases/vm.json` and the dictionary artifact `edge.dict` is copied from the committed `dict.txt`, so both are gitignored. Reusing the same `out/` resumes the campaign: AFL recalibrates the saved queue (the dry-run pass) before fuzzing, so `execs` sits at 0 for a while; delete it with `rm -rf out` for a clean start. `deploy.sh` exports the WSL bypass vars itself; for a bare `cargo afl fuzz` under WSL, prefix it with `AFL_SKIP_CPUFREQ=1 AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1`. See [Fuzzing](https://edgepython.com/implementation/fuzzing) for details.

## References

1. **Aho, Sethi & Ullman**, *Compilers: Principles, Techniques and Tools* (1986). LUT-based lexer.
2. **Pratt**, *Top Down Operator Precedence* (POPL 1973). Precedence climbing parser.
3. **Cytron et al.**, *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991). SSA, φ-nodes.
4. **Gudeman**, *Representing Type Information in Dynamically Typed Languages* (1993). NaN-boxing.
5. **Deutsch & Schiffman**, *Efficient Implementation of the Smalltalk-80 System* (POPL 1984). Inline caching.
6. **Ertl & Gregg**, *The Structure and Performance of Efficient Interpreters* (JILP 2003). Threaded dispatch.
7. **Hölzle & Ungar**, *Optimizing Dynamically-Dispatched Calls with Run-Time Type Feedback* (PLDI 1994).
8. **Casey et al.**, *Towards Superinstructions for Java Interpreters* (SCOPES 2003). LoadAttr+Call fusion.
9. **Michie**, *Memo Functions and Machine Learning* (Nature 1968). Pure-function memoization.
10. **McCarthy**, *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep GC.
11. **Backus**, *Can Programming Be Liberated from the von Neumann Style?* (CACM 1978). Function-level paradigm.
