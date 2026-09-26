# Contributing to Edge

Thanks for taking the time to contribute. This guide covers issues, pull requests, and how to build and test each part of the repository.

## Issues

Include a minimal failing script, the command used to run it, and the expected and actual behavior. Security reports follow [SECURITY.md](SECURITY.md) instead.

## Pull Requests

For a large change, open an issue or email [c.sutton.dylan@gmail.com](mailto:c.sutton.dylan@gmail.com) first so it can be accepted once ready.

Pull requests are welcome everywhere the Apache 2.0 License applies, which is every part except `lang/`, `site/` and `infra/`. Those three are the products, they carry no license, and they stay free of anyone else's copyright, so a patch to them is closed with thanks and written again by the maintainer. Issues about them are welcome all the same, since a report or a suggestion costs you nothing and carries no copyright.

- New behavior comes with tests.
- Docs describe the code as it is after the change.
- Changes to the language, the CLI, or package behavior update `skill/SKILL.md`, and `cargo test -p skill` stays green.
- Significant changes run the [fuzzer](https://edgepython.com/docs/implementation/fuzzing) to check for new crashes or slowdowns.

Run these from the repo root before sending. The maintainer runs CI once the PR is open.

```bash
cargo wasm
cargo test --release
cargo clippy --all-targets -- -D warnings
cargo clippy --lib --no-default-features --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings
cargo shear
```

## Style

Comments are one line, at most one per block, and deleted when redundant. No file-header comment or docstring. Doc edits match the length of the page they touch. Comments and docs use no colons, semicolons, or em-dashes.

## Building

The root Cargo workspace holds the engine, `abi`, `pdk`, `skill` and `lang`. `cli/`, `fuzz/` and each `std/*` package are separate workspaces.

```bash
cargo wasm # compiler.wasm, CI ships a smaller build
cargo build --release # .rlib and cdylib for Rust embedders
```

Each `std/*` package builds with `cargo build --release --target wasm32-unknown-unknown` inside its folder. Rust keywords rename the artifact, so `struct` builds `edge_struct.wasm`. `std/test` is pure Edge Python and needs no build. A new package is a `std/<name>/` folder with the crate, or `src/entry.py` for a script-only one, plus a `<name>.json` corpus of `{src, output}` or `{src, error}` cases. The runner prepends `from <name> import *` to each case, so the harness needs no edit.

`cli/` embeds `compiler.wasm`, the std `.wasm` files and the JS host from `js/dist` at build time. Build them first, or point `EDGE_COMPILER_WASM`, `EDGE_STD_DIR` and `EDGE_JS_DIST` at copies. It precompiles StarlingMonkey from `target/starling.wasm` or `EDGE_STARLING_WASM`, keeps only its hash, and downloads the artifact from `EDGE_CDN_BASE` on the first JavaScript import. Releases embed the speed build from `cargo wasm-cli`.

The JS host in `js/src` is TypeScript, linted with `deno lint js/`. The builtins in `js/builtins` are plain ESM.

## Testing

`cargo test --release` runs `tests/cases/vm.json` under `Limits::sandbox()`, so a budget, heap, or call-depth regression fails as a `MemoryError` or `RecursionError` instead of hanging. Every fixture must fit that budget.

The other suites read the builds from a CDN, the way CI does. Build what you changed, stage it, and serve it locally with no credentials.

```bash
cargo wasm
for p in json re math struct
do (cd std/$p && cargo build --release --target wasm32-unknown-unknown)
done
(cd js && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.json && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.worker.json)
curl -fsSL https://github.com/bytecodealliance/StarlingMonkey/releases/download/starlingmonkey-v0.3.0/starling.wasm -o target/starling.wasm
echo "b5707b9d97164e0c29e471844a9ccdd81c445a5d379a9299ae2ee7a9dab3aabe  target/starling.wasm" | shasum -a 256 -c
(cd cli && cargo build) # the stage ships its js-runtime artifact
cd infra && npm ci && npm run stage -- ../_cdn && npm run cdn:local -- ../_cdn # keep it running
```

Then point the suites at it. A suite without `EDGE_CDN_BASE` fails before it starts, so no test reaches the production CDN.

```bash
export EDGE_CDN_BASE=http://127.0.0.1:8788
deno run -A npm:playwright install --with-deps chromium # once
deno test --allow-all js/tests/
deno test --allow-all std/harness/ # STDPKG=<name> narrows to one package
(cd js/builtins && SYSPKG=<dom|network|storage|time> deno test --allow-all --node-modules-dir=none tests/)
cargo build --release --target wasm32-unknown-unknown -p slugify-mod
(cd cli && cargo test)
cargo test -p skill # every executable cell of skill/SKILL.md
```

Builtin corpora sit beside their module, or in `tests/cases/builtins/` when the CLI shares them. Cases may add `html`, `http_mocks`, and `ws_mocks` fixtures.

`fuzz/` runs coverage-guided fuzzing of the lexer, parser, and VM on [cargo-afl](https://github.com/rust-fuzz/afl.rs). Campaigns and crash triage are in [Fuzzing](https://edgepython.com/docs/implementation/fuzzing).

[`.github/workflows/miri.yml`](.github/workflows/miri.yml) interprets the same corpora under [Miri](https://github.com/rust-lang/miri) once a day, on the nightly it keeps to itself, for undefined behavior the native build runs past. Run one module with `cargo +nightly miri test -p edge-python --test tests vm::`.

## Site and Docs

`docs/` holds MDX pages ordered by numeric prefix. `site/` is Astro on Cloudflare Workers with a D1 database, and renders them under `/docs`. It runs locally on miniflare with no credentials, and the sign-in code prints to the terminal.

```bash
cd site && npm ci
npm run dev # port 4322
EDGE_ENV=prod npm run dev # the same tree as production sees it
npm run check
npx playwright install --with-deps chromium firefox webkit # once
npm test # builds the Worker and drives it in three engines
```

[`site/src/draft.ts`](site/src/draft.ts) names what is still being built, a path for a page and a fragment for a surface inside one. Under `EDGE_ENV=prod` a page answers 404 and the rest is rewritten out of the html.

An `edge-python` code block followed by an `output` block becomes a playground on the real engine, so every example and its output stay a verifiable pair.

A page nests one folder deep at most, carries a numeric prefix on every path segment, opens with a closed frontmatter block holding a `title` and a `description`, and has exactly one top-level heading. `npm run build` refuses a page that breaks any of it, and `edge build` holds a package's own `docs` directory to the same rules. Both read [`site/src/lib/docs/convention.ts`](site/src/lib/docs/convention.ts) and [`cli/src/docs.rs`](cli/src/docs.rs), kept in step by [`tests/cases/docs.json`](tests/cases/docs.json), so a rule changed on one side fails on the other.

## Infra and CI

`infra/` declares every Cloudflare resource in code and is checked with `npm run check` and `npm test`. `stage` and `cdn:local` run locally. The other scripts deploy and read `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.

[`.github/workflows/main.yml`](.github/workflows/main.yml) runs CI and CD, with each part in a composite action under [`.github/actions/`](.github/actions). Build jobs stage their outputs on `cdn.tmp.edgepython.com`, test jobs run every suite against them, and pushes to `main` promote the tested tree to `dev.edgepython.com`.

[`site/db/schema.sql`](site/db/schema.sql) is the whole database as it stands, and every local, test and dev database is built from it. Production keeps its rows, so a schema change also adds a file to `site/db/migrations/` that a `v` tag applies before the Worker ships, and the file is deleted once production has run it. `npm run schema` in `infra/` reads production and checks that it plus the pending migrations matches `schema.sql`, which the Database job warns about on `main` and enforces on a tag.
