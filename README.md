<div align="center">
  <a href="https://edgepython.com/" target="_blank">
    <picture>
      <img width="300" src="docs/public/static/banner.svg" alt="Edge Python Logo">
    </picture>
  </a>
</div>

<br/>

Single-pass SSA bytecode compiler and threaded-code stack VM for a sandboxed Python subset. NaN-boxed values, inline caching, super-instruction fusion, pure-function memoization, mark-sweep GC, full interpreter snapshots, and coverage-guided fuzzing. Runs in the browser as a WebAssembly module and natively inside the CLI.

- Secure by default. No file, network, or environment access, unless explicitly enabled by the [host](https://edgepython.com/reference/packages#host-libraries).
- Around 200 KB footprint. The full compiler and runtime ship as a single WASM binary.
- Compile-time imports. Every module resolves at parse time, no dynamic loading, no runtime surprises.
- No AST. Source compiles directly to bytecode in a single pass: O(n).
- Snapshots. Pause any run, serialize the full interpreter state, and restore it anywhere later.

## More about it

- Docs (try Edge Python directly in your browser): [edgepython.com](https://edgepython.com/)

## Repository layout

A Cargo workspace at the repo root holds the engine, `abi` and `pdk`. `cli/`, `fuzz/` and each `std/*` package are standalone workspaces with their own build and test commands; the commands below run from the repo root.

```text
├── abi
├── cli
│   └── src
│       ├── cmd
│       └── engine
├── docs
├── fuzz
├── host
├── pdk
├── runtime
├── src
│   ├── lexer
│   ├── native
│   │   └── builtins
│   ├── packages
│   ├── parser
│   ├── util
│   ├── value
│   ├── vm
│   │   ├── globals
│   │   ├── methods
│   │   └── opcodes
│   └── wasm
├── std
└── tests
    └── cases
```

```bash
cargo wasm # local release .wasm (CI ships a further size-optimised build)
cargo build --release # host .rlib + cdylib for Rust embedders
cargo clippy --lib --features native # lint the native engine module
cargo test --release # run the compiler test suite
```

## Architecture

Single-pass pipeline: source -> SSA bytecode chunk; stack interpreter with adaptive inline caching and pure-function memoization.

* **Lexer** (`src/lexer/`) LUT-driven, offset-based tokens.
* **Parser** (`src/parser/`) Pratt precedence, SSA-versioned bytecode with `Phi` at joins, no AST.
* **Optimizer** (`src/optimizer.rs`) constant folding, Phi-noop elimination, dead-code compaction.
* **Values** (`src/value/`) NaN-boxed 64-bit `Val`, heap objects, and the mark-and-sweep arena the whole pipeline shares.
* **VM** (`src/vm/`) flat-match dispatch, scalar + instance-dunder inline caches, pure-function template memoization; `opcodes/` implements opcodes, `globals/` the global functions, `methods/` the builtin-type methods.
* **Resolver** (`src/packages/`) host-injected; native imports register for `CallExtern` dispatch.

Full rationale, NaN-box patterns, IC thresholds, GC roots, and intentional omissions: [Design](https://edgepython.com/implementation/design). Lexer and parser internals: [Lexical](https://edgepython.com/implementation/lexical), [Syntax](https://edgepython.com/implementation/syntax).

Native modules ship via four delivery paths (CDN `.wasm`, native `.so`/`.dylib` plugin, host capability, JS host module), see [Writing modules](https://edgepython.com/reference/writing-modules).

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

`run`, `repl` and `test` execute in the built-in native engine; `--web` hosts the runtime in a headless Chromium instead, and `install.sh` downloads a pinned `chrome-headless-shell` into `~/.cache/edge` unless a system Chrome/Chromium is already installed (`EDGE_NO_BROWSER=1` skips it).

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

Edge Python is a `cdylib`: a Rust host can instantiate `compiler.wasm` and call its exports directly, the same `.wasm` that ships to browsers; the host owns I/O. The crate fetches nothing at build time, so vendor the `.wasm` from a tagged release and pin it by checksum, see [Consuming the release](https://edgepython.com/reference/wasm-abi#consuming-the-release-from-a-rust-crate). To add native modules from your own crate, implement the `Resolver` trait, see [Writing modules](https://edgepython.com/reference/writing-modules).

### Native

`edge run`, `edge repl`, and `edge test` execute in the CLI's in-process native engine by default (`--web` restores Chromium); tagged releases and the CDN's `/native/` route carry each std package as a native plugin library ([native engine](https://edgepython.com/reference/native)).

```python
# hello.py
async def greet(name):
  await sleep(0.1)
  print(f"hello {name}")

await greet("edge")
```

```text
$ edge run hello.py
hello edge
```

Imports, std packages, and the built-in `time` / `network` modules resolve without a browser:

```python
# app.py
import json
from "./lib/helper.py" import double

data = json.loads('{"n": 21}')
await sleep(0.1)
print(json.dumps({"result": double(data["n"])}))
```

```text
$ edge run app.py
{"result":42}
```

## What it is

Edge Python targets sandboxed execution, in the browser and in the CLI's native engine: a dynamic, multi-paradigm Python subset with classes, async/await, structural pattern matching, and compile-time module resolution. There is no bundled stdlib, modules are external artifacts.

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
