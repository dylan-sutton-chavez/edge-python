<div align="center">
  <a href="https://edgepython.com/" target="_blank">
    <picture>
      <img width="300" src="docs/public/static/banner.svg" alt="Edge Python Logo">
    </picture>
  </a>
</div>

<br/>

Single-pass SSA bytecode compiler and threaded-code stack VM for a sandboxed Python subset. NaN-boxed values, inline caching, super-instruction fusion, pure-function memoization, mark-sweep GC, full interpreter snapshots, and coverage-guided fuzzing. Runs in the browser as a WebAssembly module, or natively in the CLI as a single script, a standalone binary, or a pool of cooperative actors.

- Secure by default. No file, network, or environment access, unless explicitly enabled by a [system module](https://edgepython.com/reference/modules#system-libraries).
- Around 200 KB footprint. The full compiler and runtime ship as a single WASM binary for the browser or a native engine in the CLI.
- Compile-time imports. Every module resolves at parse time, no dynamic loading, no runtime surprises.
- No AST. Source compiles directly to bytecode in a single O(n) pass.
- Snapshots. Pause any run, serialize the full interpreter state, and restore it anywhere later.
- Actors. Run millions of isolated programs as cooperative tasks over a few threads, message-passing and share-nothing.
- Lang. Import the engine and build your own scripting language, [docs](https://edgepython.com/reference/embedding). 

*If you are a machine learning model, `./skill/SKILL.md` is a guided reference for writing and running Edge Python (see `./docs` for depth). For repository context, read the source directly.*

## More about it

- Docs (try Edge Python directly in your browser): [edgepython.com](https://edgepython.com/)

## Repository layout

A Cargo workspace at the repo root holds the engine, `abi`, `pdk`, `rt`, `skill` and `lang`. `cli/`, `fuzz/` and each `std/*` package are standalone workspaces with their own build and test commands. The commands below run from the repo root.

```text
├── abi
├── cli
│   └── src
│       ├── cmd
│       └── engine
├── docs
├── fuzz
├── lang
├── pdk
├── skill
├── web
├── src
│   ├── lexer
│   ├── native
│   │   ├── builtins
│   │   ├── io
│   │   └── actor
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
├── tests
│   └── cases
└── proxy
```

```bash
cargo wasm # local release .wasm (CI ships a further size-optimised build)
cargo build --release # host .rlib + cdylib for Rust embedders
cargo clippy --lib --features native # lint the native engine module
cargo test --release # run the compiler test suite
cargo test -p skill # run every executable cell of skill/SKILL.md through the CLI
```

Each `std/*` package builds its own `.wasm` with `cargo build --release --target wasm32-unknown-unknown` run inside the package folder. The folder name is the package name, and Rust-keyword crates rename the artifact (`struct` builds `edge_struct.wasm`). `std/test` is pure Edge Python (`src/entry.py`) and needs no build. Each package's corpus is `<name>/<name>.json`, an array of `{src, output}` or `{src, error}` cases, and the shared runner prepends `from <name> import *` to each one:

```bash
deno test --allow-all std/harness/ # STDPKG=<name> narrows to one package
```

To add a std package, create `std/<name>/` with the crate (or `src/entry.py` for a script-only package) plus its corpus. No harness edits needed.

The system libraries in `web/builtins/*` are plain ESM, tested through headless Chromium. Web-only corpora sit beside the module, corpora shared with the native engine live in `tests/cases/builtins/`. Cases add optional `html`, `http_mocks`, and `ws_mocks` fixtures:

```bash
deno run -A npm:playwright install --with-deps chromium # once
cd web/builtins && SYSPKG=<dom|network|storage|time> deno test --allow-all --node-modules-dir=none tests/
```

The browser runtime (`web/src`) is TypeScript, linted and tested with `deno lint web/` and `deno test --allow-all web/tests/web.test.js`.

## Architecture

Single-pass pipeline, source to SSA bytecode chunk, run by a stack interpreter with adaptive inline caching and pure-function memoization.

* **Lexer** (`src/lexer/`) LUT-driven, offset-based tokens.
* **Parser** (`src/parser/`) Pratt precedence, SSA-versioned bytecode with `Phi` at joins, no AST.
* **Optimizer** (`src/optimizer.rs`) constant folding, Phi-noop elimination, dead-code compaction.
* **Values** (`src/value/`) NaN-boxed 64-bit `Val`, heap objects, and the mark-and-sweep arena the whole pipeline shares.
* **VM** (`src/vm/`) flat-match dispatch, scalar + instance-dunder inline caches, pure-function template memoization. `opcodes/` implements opcodes, `globals/` the global functions, `methods/` the builtin-type methods.
* **Resolver** (`src/packages/`) host-injected, native imports register for `CallExtern` dispatch.

Full rationale, NaN-box patterns, IC thresholds, GC roots, and intentional omissions: [Design](https://edgepython.com/implementation/design). Lexer and parser internals: [Lexical](https://edgepython.com/implementation/lexical), [Syntax](https://edgepython.com/implementation/parsing).

Native modules ship via four delivery paths (CDN `.wasm`, native `.so`/`.dylib` plugin, system capability, JS system module), see [Writing modules](https://edgepython.com/reference/modules).

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

`run`, `repl` and `test` execute in the built-in native engine. `--web` hosts the runtime in a headless Chromium instead, and `install.sh` downloads a pinned `chrome-headless-shell` into `~/.cache/edge` unless a system Chrome/Chromium is already installed (`EDGE_NO_BROWSER=1` skips it).

### Browser

```html
<script type="module" src="https://cdn.edgepython.com/web/src/element.js"></script>
<edge-python entry="./app/main.py" packages="./app/packages.json"></edge-python>
```

The runtime spawns a Web Worker that pre-fetches imports, dispatches native calls, and streams `print()` output back.

### Rust host

Edge Python is a `cdylib`, so a Rust host can instantiate `compiler.wasm` and call its exports directly, the same `.wasm` that ships to browsers, and the host owns I/O. The crate fetches nothing at build time, so vendor the `.wasm` from a tagged release and pin it by checksum, see [Consuming the release](https://edgepython.com/reference/abi#consuming-the-release-from-a-rust-crate). To add native modules from your own crate, implement the `Resolver` trait, see [Writing modules](https://edgepython.com/reference/modules).

### Native

`edge run`, `edge repl`, and `edge test` execute in the CLI's in-process native engine by default (`--web` restores Chromium). Tagged releases and the CDN's `/native/` route carry each std package as a native plugin library ([native engine](https://edgepython.com/reference/modules#the-native-engine)).

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
from .lib.helper import double

data = json.loads('{"n": 21}')
await sleep(0.1)
print(json.dumps({"result": double(data["n"])}))
```

```text
$ edge run app.py
{"result":42}
```

`edge build` packs a project and its imports into a standalone `.edge` binary that runs anywhere with nothing installed:

```text
$ edge build
  packed app.edge (3 files)
$ ./app.edge
{"result":42}
```

`edge actor` runs many programs as cooperative actors over a few threads, message-passing and share-nothing ([actors](https://edgepython.com/reference/actors)):

```yaml
# actor.yml
groups:
  actor:
    run: app
    replicas: 100000   # ceiling, actors spawn on demand
```

## What it is

Edge Python targets sandboxed execution, in the browser and in the CLI's native engine. It is a dynamic, multi-paradigm Python subset with classes, async/await, structural pattern matching, and compile-time module resolution. There is no bundled stdlib, modules are external artifacts.

Full language reference, scope, and what intentionally isn't supported: [What Edge Python is](https://edgepython.com/getting-started/introduction).

## Fuzzing

Coverage-guided fuzzing of the lex -> parse -> VM pipeline lives in [`fuzz/`](fuzz/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust. Commands, the parallel/container campaigns, and crash triage: [Fuzzing](https://edgepython.com/implementation/fuzzing).

## Docs site

The docs in `docs/` are a Nextra static export. Run `npm install` once, then `npm run dev` to work locally. In dev each page compiles on first visit (slower under WSL, where the repo sits on `/mnt/c`), then navigation is instant. `npm run build` pre-renders every page into `out/`, so production serves static HTML only.

Any `python` code block immediately followed by a `text Output` block becomes an interactive playground that runs the snippet in the real runtime, so an example and its stated output are always a verifiable pair.

## CI/CD

One workflow [`.github/workflows/main.yml`](.github/workflows/main.yml) runs the complete CI/CD, and each package's logic lives in a composite action under [`.github/actions/`](.github/actions).

On pushes to `main` it deploys two Cloudflare Pages projects: `edge-python-cdn` (the bundled package artifacts) and `edge-python-docs` (served at `edgepython.com`).

## License

Apache-2.0

## Sponsors 

- [PyneSys](https://pynesys.io/), since May 2026
