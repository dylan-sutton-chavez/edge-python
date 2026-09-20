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
- For significant changes, execute the [fuzzer](https://edgepython.com/docs/implementation/fuzzing) to check for new crashes or performance regressions.

The test suite (`tests/`, fixtures in `tests/cases/vm.json`) runs every case under `Limits::sandbox()`, so budget, heap, and call-depth regressions surface as a `MemoryError` or `RecursionError` assertion instead of a hang. Every fixture must stay within the sandbox budget.

A CI job will be run by the maintainer after the PR has been created.

PRs that introduce new behavior without test coverage, or that update documentation without reflecting the actual code change, will not be accepted.

## Building and Testing

A Cargo workspace at the repo root holds the engine, `abi`, `pdk`, `skill` and `lang`. `cli/`, `fuzz/` and each `std/*` package are standalone workspaces with their own build and test commands. The commands below run from the repo root.

```bash
cargo wasm # local release .wasm (CI ships a further size-optimised build)
cargo build --release # host .rlib + cdylib for Rust embedders
cargo test --release # run the compiler test suite
```

Each `std/*` package builds its own `.wasm` with `cargo build --release --target wasm32-unknown-unknown` run inside the package folder. The folder name is the package name, and Rust-keyword crates rename the artifact (`struct` builds `edge_struct.wasm`). `std/test` is pure Edge Python (`src/entry.py`) and needs no build. Each package's corpus is `<name>/<name>.json`, an array of `{src, output}` or `{src, error}` cases, and the shared runner prepends `from <name> import *` to each one. To add a std package, create `std/<name>/` with the crate (or `src/entry.py` for a script-only package) plus its corpus. No harness edits needed.

The JavaScript libraries in `js/builtins/*` are plain ESM, tested through headless Chromium. Corpora only the JS host serves sit beside the module, corpora shared with the CLI live in `tests/cases/builtins/`. Cases add optional `html`, `http_mocks`, and `ws_mocks` fixtures. The JS host (`js/src`) is TypeScript, linted with `deno lint js/` and tested under Deno with no browser and through Chromium.

Every suite reads the builds through a CDN, the way CI reads them from its tmp stage. Build what you changed, stage the tree with `infra`, serve it on miniflare, then point the suites at it with `EDGE_CDN_BASE`. The local CDN needs no credentials, and a suite without `EDGE_CDN_BASE` fails before it starts.

```bash
cargo wasm
for p in json re math struct
do (cd std/$p && cargo build --release --target wasm32-unknown-unknown)
done
(cd js && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.json && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.worker.json)
curl -fsSL https://github.com/bytecodealliance/StarlingMonkey/releases/download/starlingmonkey-v0.3.0/starling.wasm -o target/starling.wasm
echo "b5707b9d97164e0c29e471844a9ccdd81c445a5d379a9299ae2ee7a9dab3aabe  target/starling.wasm" | shasum -a 256 -c
(cd cli && cargo build) # before the stage, which ships its js-runtime artifact
cd infra && npm ci && npm run stage -- ../_cdn && npm run cdn:local -- ../_cdn # keep it running
```

```bash
export EDGE_CDN_BASE=http://127.0.0.1:8788
deno run -A npm:playwright install --with-deps chromium # once
deno test --allow-all js/tests/
deno test --allow-all std/harness/ # STDPKG=<name> narrows to one package
(cd js/builtins && SYSPKG=<dom|network|storage|time> deno test --allow-all --node-modules-dir=none tests/)
cargo build --release --target wasm32-unknown-unknown -p slugify-mod
(cd cli && cargo test)
cargo test -p skill # run every executable cell of skill/SKILL.md through the CLI
```

`cli/` embeds `compiler.wasm` and the std `.wasm` files at build time, so build them first or point `EDGE_COMPILER_WASM` and `EDGE_STD_DIR` at copies. It also precompiles the pinned StarlingMonkey from `target/starling.wasm`, or from the file `EDGE_STARLING_WASM` names, into `cli/target/<profile>/js-runtime/<sha256>.cwasm`. The binary keeps only that hash and downloads the artifact from `EDGE_CDN_BASE` on the first JavaScript import, which is why the CLI builds before the stage. Nothing is fetched at build time. Releases embed the speed build from `cargo wasm-cli` instead of the size build the browser gets. The `edge build --web` cases fetch the JS host and the packages from `EDGE_CDN_BASE`, so no test reaches the production CDN.

Coverage-guided fuzzing of the lex, parse and VM pipeline lives in [`fuzz/`](fuzz/), built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++) and running on stable Rust. Commands, the parallel and container campaigns, and crash triage are in [Fuzzing](https://edgepython.com/docs/implementation/fuzzing).

The docs in `docs/` are MDX pages ordered by their numeric prefixes, and the site in `site/` renders them under `/docs`. The site is Astro on Cloudflare Workers with a D1 database, and it runs locally on miniflare with no credentials. `npm ci` once, then `npm run dev` applies the local migrations and seed and serves on port 4322. Sign in with the email code, the local email binding prints each message's subject, code included, to the terminal. `npm run check`, `npm run build` and `npm test` are the gates CI runs, and `npm test` walks `tests/routes.json` against the built Worker on local bindings. Any `edge-python` code block immediately followed by an `output` block becomes an interactive playground that runs the snippet on the real engine through the JS host, so an example and its stated output are always a verifiable pair.

```bash
cd site && npm ci && npm run dev
```

`infra/` declares every Cloudflare resource in code. `dev` serves the site at `dev.edgepython.com` and its CDN at `cdn.dev.edgepython.com`. `tmp` is only a CDN, `cdn.tmp.edgepython.com`, where each CI run stages under its run id and objects expire after a day. `npm run app` ensures the D1 database, email sending, the Access gate and both R2 buckets. `npm run stage -- <dir> [part]` lays build outputs out as the CDN tree, `npm run upload -- <run> <dir>` puts one under a run's tmp prefix, and `npm run promote -- <run>` copies that tested tree to dev, resets and seeds the dev database, publishes the Worker with its secrets and clears the prefix. `npm run cdn:local -- <dir>` serves a staged tree on miniflare. Everything except `stage` and `cdn:local` reads `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`, and `npm run check` typechecks the package.

One workflow, [`.github/workflows/main.yml`](.github/workflows/main.yml), runs the complete CI/CD, and each package's logic lives in a composite action under [`.github/actions/`](.github/actions). Each build job stages its outputs on tmp, the test jobs run every suite against that prefix, and on pushes to `main` the promote job ships it to dev with the `DEV_` secrets.

## Comments and Docs

Keep comments minimal. One line, at most one per block, deleted when redundant. Match the length of the docs you edit rather than expanding them. No colons, semicolons, or em-dashes in comment or doc prose. No file-header comment or docstring at the top of a file.

Changes that alter language, CLI, or package behavior must update `skill/SKILL.md` to match. Its examples are executable cells, so `cargo test -p skill` must stay green.
