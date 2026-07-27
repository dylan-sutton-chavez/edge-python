<div align="center">
  <a href="https://edgepython.com/" target="_blank">
    <picture>
      <img width="300" src="docs/public/static/banner.svg" alt="Edge Python Logo">
    </picture>
  </a>
</div>

<br/>

Single-pass SSA bytecode compiler and threaded-code stack VM for a sandboxed Python subset: NaN-boxed values, inline caching, super-instruction fusion, pure-function memoization, mark-sweep GC. Coverage-guided fuzzing; runs in the browser as a WebAssembly module.

- Secure by default. No file, network, or environment access, unless explicitly enabled by the [host](https://edgepython.com/reference/packages#host-libraries).
- Around 200 KB footprint. The full compiler and runtime ship as a single WASM binary.
- Compile-time imports. Every module resolves at parse time, no dynamic loading, no runtime surprises.
- No AST. Source compiles directly to bytecode in a single pass: O(n).
- Snapshots. Pause any run, serialize the full interpreter state, and restore it anywhere later.

## More about it

- Docs (try Edge Python directly in your browser): [edgepython.com](https://edgepython.com/)

## Repository layout

Cargo workspace rooted at the engine crate (`edge-python`); commands work from any directory.

```text
├── abi
├── cli
├── docs
├── fuzz
├── host
├── js
├── pdk
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
├── std
└── tests
    └── cases
```

```bash
cargo wasm # release .wasm (the distributed artifact)
cargo build --release # host .rlib + cdylib for Rust embedders
cargo test --release --no-default-features # run the compiler test suite
```

*`--no-default-features` disables the `prebuilt` feature, which otherwise triggers a `build.rs` download of `compiler.wasm` from the GitHub release: that download is only needed by external Rust crates that consume this library, not when developing locally.*

The test suite (`tests/`, fixtures in `tests/cases/vm.json`) runs every case under `Limits::sandbox()`, not the default `none()`. The budget, heap, and call-depth guards short-circuit under `none` (`sandbox_off`), so only the bounded profile exercises them — that way a regression that lets a loop run unbounded, recurse without limit, or materialise an oversized collection becomes a failing `MemoryError` / `RecursionError` assertion instead of a hang. Every fixture must stay within the sandbox budget.

## Architecture

Single-pass pipeline: source -> SSA bytecode chunk; stack interpreter with adaptive inline caching and pure-function memoization.

* **Lexer** (`src/modules/lexer/`) LUT-driven, offset-based tokens.
* **Parser** (`src/modules/parser/`) Pratt precedence, SSA-versioned bytecode with `Phi` at joins, no AST.
* **Optimizer** (`src/modules/vm/optimizer.rs`) constant folding, Phi-noop elimination, dead-code compaction.
* **VM** (`src/modules/vm/`) flat-match dispatch, scalar + instance-dunder inline caches, pure-function template memoization, NaN-boxed 64-bit `Val` with a mark-and-sweep arena.
* **Resolver** (`src/modules/packages/`) host-injected; native imports register for `CallExtern` dispatch.

Full rationale, NaN-box patterns, IC thresholds, GC roots, and intentional omissions: [Design](https://edgepython.com/implementation/design). Lexer and parser internals: [Lexical](https://edgepython.com/implementation/lexical), [Syntax](https://edgepython.com/implementation/syntax).

Native modules ship via three delivery paths (CDN `.wasm`, host capability, JS host module), see [Writing modules](https://edgepython.com/reference/writing-modules).

## Quick start

### CLI

download it to your machine ([reference docs](https://edgepython.com/reference/cli)):

```bash
# Compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

edge -h # List all commands
```

`edge` hosts the runtime in a headless Chromium for `run`, `repl` and `test`; `install.sh` downloads a pinned `chrome-headless-shell` into `~/.cache/edge` unless a system Chrome/Chromium is already installed.

### Browser

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <script type="module" src="https://cdn.edgepython.com/runtime/src/element.js"></script>
</head>
<body>
  <edge-python entry="./app/main.py" packages="./app/packages.json"></edge-python>
</body>
</html>
```

The runtime spawns a Web Worker that pre-fetches imports, dispatches native calls, and streams `print()` output back.

### Rust host

Edge Python is a `cdylib`: a Rust host can instantiate `compiler.wasm` and call its exports directly, the same `.wasm` that ships to browsers; the host owns I/O. To add native modules from your own crate, implement the `Resolver` trait, see [Writing modules](https://edgepython.com/reference/writing-modules).

The crate declares `links = "compiler"` and its `build.rs` downloads the matching `compiler.wasm` from the GitHub Release for `CARGO_PKG_VERSION` into `OUT_DIR`. Downstream crates read the absolute path through `DEP_COMPILER_WASM`.

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

## What it is

Edge Python targets sandboxed in-browser execution: a dynamic, multi-paradigm Python subset with classes, async/await, structural pattern matching, and compile-time module resolution. There is no bundled stdlib, modules are external artifacts.

Full language reference, scope, and what intentionally isn't supported: [What Edge Python is](https://edgepython.com/getting-started/what-it-is). Architecture details: [Architecture](#architecture) above.

## Fuzzing

Coverage-guided fuzzing of the lex -> parse -> VM pipeline lives in [`fuzz/`](fuzz/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust.

```bash
cd fuzz
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

## CI/CD

One workflow [`.github/workflows/main.yml`](.github/workflows/main.yml) that runs the complete CI/CD, where each package is a steps in a composite action under [`.github/actions/`](.github/actions).

On pushes to `main` it deploys two Cloudflare Pages projects: `edge-python-cdn` (the bundled package artifacts) and `edge-python-docs` (served at `edgepython.com`).

## License

MIT OR Apache-2.0

## Sponsors 

- [PyneSys](https://pynesys.io/), since May 2026
