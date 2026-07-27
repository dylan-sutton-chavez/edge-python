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
│   ├── lexer
│   ├── packages
│   ├── parser
│   ├── util
│   ├── vm
│   │   ├── builtins
│   │   ├── handlers
│   │   │   └── builtin_methods
│   │   └── types
│   └── wasm
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

## Architecture

Single-pass pipeline: source -> SSA bytecode chunk; stack interpreter with adaptive inline caching and pure-function memoization.

* **Lexer** (`src/lexer/`) LUT-driven, offset-based tokens.
* **Parser** (`src/parser/`) Pratt precedence, SSA-versioned bytecode with `Phi` at joins, no AST.
* **Optimizer** (`src/vm/optimizer.rs`) constant folding, Phi-noop elimination, dead-code compaction.
* **VM** (`src/vm/`) flat-match dispatch, scalar + instance-dunder inline caches, pure-function template memoization, NaN-boxed 64-bit `Val` with a mark-and-sweep arena.
* **Resolver** (`src/packages/`) host-injected; native imports register for `CallExtern` dispatch.

Full rationale, NaN-box patterns, IC thresholds, GC roots, and intentional omissions: [Design](https://edgepython.com/implementation/design). Lexer and parser internals: [Lexical](https://edgepython.com/implementation/lexical), [Syntax](https://edgepython.com/implementation/syntax).

Native modules ship via three delivery paths (CDN `.wasm`, host capability, JS host module), see [Writing modules](https://edgepython.com/reference/writing-modules).

## Quick start

### CLI

Download it to your machine ([reference docs](https://edgepython.com/reference/cli)):

```bash
# Compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh
# Or from source (any platform with Rust + Cargo)
cargo install --path cli

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

Edge Python is a `cdylib`: a Rust host can instantiate `compiler.wasm` and call its exports directly, the same `.wasm` that ships to browsers; the host owns I/O. Declaring `edge-python` as a Cargo dependency fetches the matching release `.wasm` automatically (exposed as `DEP_COMPILER_WASM`), see [Consuming the release](https://edgepython.com/reference/wasm-abi#consuming-the-release-from-a-rust-crate). To add native modules from your own crate, implement the `Resolver` trait, see [Writing modules](https://edgepython.com/reference/writing-modules).

## What it is

Edge Python targets sandboxed in-browser execution: a dynamic, multi-paradigm Python subset with classes, async/await, structural pattern matching, and compile-time module resolution. There is no bundled stdlib, modules are external artifacts.

Full language reference, scope, and what intentionally isn't supported: [What Edge Python is](https://edgepython.com/getting-started/what-it-is).

## Fuzzing

Coverage-guided fuzzing of the lex -> parse -> VM pipeline lives in [`fuzz/`](fuzz/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust. Commands, the parallel/container campaigns, and crash triage: [Fuzzing](https://edgepython.com/implementation/fuzzing).

## CI/CD

One workflow [`.github/workflows/main.yml`](.github/workflows/main.yml) runs the complete CI/CD; each package's logic lives in a composite action under [`.github/actions/`](.github/actions).

On pushes to `main` it deploys two Cloudflare Pages projects: `edge-python-cdn` (the bundled package artifacts) and `edge-python-docs` (served at `edgepython.com`).

## License

Apache-2.0

## Sponsors 

- [PyneSys](https://pynesys.io/), since May 2026
