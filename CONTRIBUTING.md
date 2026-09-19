# How to Contribute to Edge Python

Thanks for your interest in contributing to Edge. This document outlines some recommendations on how to contribute.

## Issues and Feature Requests

Please provide a failing example if possible to help with issue reproduction.

## Pull Requests

For large changes, please try reaching out to the Edge maintainers via GitHub Issues or Email to ensure that the change can be accepted once it is ready.

Run the following commands before sending a pull request to ensure code quality:

- `cargo wasm` Build the `compiler.wasm`.
- `cargo test --release` Run the compiler test suite.
- `cargo clippy --all-targets -- -D warnings` Lint the Rust code.
- `cargo clippy --lib --no-default-features --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings` Lint the wasm build.
- `cargo shear` Detect unused dependencies.
- For significant changes, execute the [fuzzer](https://edgepython.com/implementation/fuzzing/) to check for new crashes or performance regressions.

The test suite (`tests/`, fixtures in `tests/cases/vm.json`) runs every case under `Limits::sandbox()`, so budget, heap, and call-depth regressions surface as a `MemoryError` or `RecursionError` assertion instead of a hang. Every fixture must stay within the sandbox budget.

A CI job will be run by the maintainer after the PR has been created.

PRs that introduce new behavior without test coverage, or that update documentation without reflecting the actual code change, will not be accepted.

## Building and Testing

A Cargo workspace at the repo root holds the engine, `abi`, `pdk`, `skill` and `lang`. `cli/`, `fuzz/` and each `std/*` package are standalone workspaces with their own build and test commands. The commands below run from the repo root.

```bash
cargo wasm # local release .wasm (CI ships a further size-optimised build)
cargo build --release # host .rlib + cdylib for Rust embedders
cargo test --release # run the compiler test suite
cargo test -p skill # run every executable cell of skill/SKILL.md through the CLI
```

Each `std/*` package builds its own `.wasm` with `cargo build --release --target wasm32-unknown-unknown` run inside the package folder. The folder name is the package name, and Rust-keyword crates rename the artifact (`struct` builds `edge_struct.wasm`). `std/test` is pure Edge Python (`src/entry.py`) and needs no build. Each package's corpus is `<name>/<name>.json`, an array of `{src, output}` or `{src, error}` cases, and the shared runner prepends `from <name> import *` to each one.

```bash
deno test --allow-all std/harness/ # STDPKG=<name> narrows to one package
```

To add a std package, create `std/<name>/` with the crate (or `src/entry.py` for a script-only package) plus its corpus. No harness edits needed.

`cli/` embeds `compiler.wasm` and the std `.wasm` files at build time, so build them first or point `EDGE_COMPILER_WASM` and `EDGE_STD_DIR` at copies. Nothing is fetched at build time. Releases embed the speed build from `cargo wasm-cli` instead of the size build the browser gets. The `edge build --web` cases vendor the local JS host from `js/`, so no test reaches the CDN.

```bash
cargo wasm
for p in json re math struct
do (cd std/$p && cargo build --release --target wasm32-unknown-unknown)
done
(cd js && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.json && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.worker.json)
cargo build --release --target wasm32-unknown-unknown -p slugify-mod
cd cli && cargo test
```

The JavaScript libraries in `js/builtins/*` are plain ESM, tested through headless Chromium. Corpora only the JS host serves sit beside the module, corpora shared with the CLI live in `tests/cases/builtins/`. Cases add optional `html`, `http_mocks`, and `ws_mocks` fixtures.

```bash
deno run -A npm:playwright install --with-deps chromium # once
cd js/builtins && SYSPKG=<actor|dom|network|storage|time> deno test --allow-all --node-modules-dir=none tests/
```

The JS host (`js/src`) is TypeScript, linted and tested with `deno lint js/`, `deno test --allow-all js/tests/js.test.js` through Chromium, and `deno test --allow-all js/tests/deno.test.js` under Deno with no browser.

Coverage-guided fuzzing of the lex, parse and VM pipeline lives in [`fuzz/`](fuzz/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust. Commands, the parallel and container campaigns, and crash triage are in [Fuzzing](https://edgepython.com/implementation/fuzzing).

The docs in `docs/` are a Nextra static export. Run `npm install` once, then `npm run dev` to work locally. In dev each page compiles on first visit (slower under WSL, where the repo sits on `/mnt/c`), then navigation is instant. `npm run build` pre-renders every page into `out/`, so production serves static HTML only. Any `python` code block immediately followed by a `text Output` block becomes an interactive playground that runs the snippet on the real engine through the JS host, so an example and its stated output are always a verifiable pair.

One workflow, [`.github/workflows/main.yml`](.github/workflows/main.yml), runs the complete CI/CD, and each package's logic lives in a composite action under [`.github/actions/`](.github/actions). On pushes to `main` it deploys two Cloudflare Pages projects, `edge-python-cdn` (the bundled package artifacts) and `edge-python-docs` (served at `edgepython.com`).

## Comments and Docs

Keep comments minimal. One line, at most one per block, deleted when redundant. Match the length of the docs you edit rather than expanding them. No colons, semicolons, or em-dashes in comment or doc prose. No file-header comment or docstring at the top of a file.

Changes that alter language, CLI, or package behavior must update `skill/SKILL.md` to match. Its examples are executable cells, so `cargo test -p skill` must stay green.
