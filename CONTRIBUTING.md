# Contributing to Edge

Thanks for taking the time to contribute. This guide covers issues, pull requests, and how to build and test each part of the repository.

## Issues

Include a minimal failing script, the command used to run it, and the expected and actual behavior. Security reports follow [SECURITY.md](SECURITY.md) instead.

## Pull Requests

For a large change, open an issue or email [c.sutton.dylan@gmail.com](mailto:c.sutton.dylan@gmail.com) first so it can be accepted once ready.

Pull requests are welcome in every part under the Apache 2.0 License, which is all but `lang/`, `site/`, `infra/` and `bot/`. Those four stay free of anyone else's copyright, so a patch to them is closed with thanks and written again by the maintainer, while an issue about them is always welcome. The licenses are in [README.md](README.md#license).

- New behavior comes with tests.
- Docs describe the code as it is after the change.
- Changes to the language, the CLI, or package behavior update `skill/SKILL.md`, and `cargo test -p skill` stays green.
- Significant changes run the [fuzzer](https://edgepython.com/docs/implementation/fuzzing) to check for new crashes or slowdowns.
- Changes to the engine keep `cargo run -p bench --profile cli` passing.

Run these from the repo root before sending. The maintainer runs CI once the pull request is open.

```bash
cargo wasm
cargo test --release
cargo clippy --all-targets -- -D warnings
cargo clippy --lib --no-default-features --features edge-python/runtime --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings
cargo clippy --lib --no-default-features --target wasm32-unknown-unknown -p edge-python -- -D warnings
cargo shear
```

## Style

Comments are one line, at most one per block, and deleted when redundant. No file-header comment or docstring. Doc edits match the length of the page they touch. Comments and docs use no colons, semicolons, or em-dashes.

## Building

The root Cargo workspace holds the engine, `abi`, `pdk`, `skill`, `lang` and `bench`, and `cli/` and `fuzz/` are workspaces of their own. `rust-toolchain.toml` pins every build to one Rust release, so the bench counts the same `compiler.wasm` everywhere.

```bash
cargo wasm # compiler.wasm, CI ships a smaller build
cargo wasm-cli # the speed build releases embed
cargo build --release # the .rlib Rust embedders link
(cd js && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.json && deno run -A npm:typescript@5.9.3/tsc -p tsconfig.worker.json && deno bundle --platform browser --format iife src/worker/worker.ts -o dist/worker/bundle.js)
(cd cli && cargo build --release)
deno lint js/
```

- `cargo wasm` turns on the `runtime` feature, the wrapper that makes `compiler.wasm` with its exports, host imports, allocator and panic handler. Without it the crate is the engine alone, for a wasm that links it as a library.
- `cli/` embeds `compiler.wasm` and `js/dist` at build time, so build them first or point `EDGE_COMPILER_WASM` and `EDGE_JS_DIST` at copies.
- The CLI runs the system calls of `js/src/system` in SpiderMonkey through `mozjs`, which compiles from source on the first build and wants clang, python3 and `llvm-objdump` on the path. On macOS a symlink named `llvm-objdump` to `/usr/bin/objdump` serves, and CI sets `MOZJS_FROM_SOURCE=1` on every target.
- The static Linux binary builds inside `rust:alpine` through [`musl.sh`](.github/actions/cli/musl.sh), since musl-gcc has no C++.

## Testing

### Engine

`cargo test --release` runs `tests/cases/vm.json` under `Limits::sandbox()`, so a budget, memory, or call-depth regression fails as a `MemoryError` or `RecursionError` instead of hanging. Every case must fit that budget, and two cases equal in every field fail the suite.

`--features memcheck` recounts every slot at each collection and fails a case whose running memory count drifted from it, and `tests/memory.rs` checks the memory model never counts less than the allocator hands out. A change that grows a container the heap holds keeps both green.

### Bench

`cargo run -p bench --profile cli` runs every case of `vm.json` on the `cargo wasm-cli` build, and stops when that build is older than `src/`. It counts the WebAssembly instructions each case executes and prices each at 0.82 ns, the 822756 gas that `wasm_regular_op_cost` sets in `core/parameters/res/runtime_configs/parameters.yaml` of nearcore, at the 1 ms per Tgas its gas estimator budgets, so `bench/.snapshot` keeps reference seconds that come out the same on every machine. Each case keeps them beside its memory peak in MB, by the model the memory limit counts, as `[seconds, MB]`. The peak includes garbage not yet collected, so a change to when the collector runs moves it too.

The Bench job fails a pull request when

- the snapshot was taken with another Rust,
- a case is missing from it, or an entry has no case,
- the geometric mean of time or memory moves past its threshold, either way,
- a single case moves past its own threshold in either, and in memory also by more than `memory_floor` MB.

Every run prints how the cases already in the snapshot moved, even when another rule fails. `--update` reports the change and takes the snapshot again, and a faster engine takes it too, so the next change is measured from where the code stands.

### Hosts

The JS, CLI and skill suites read the builds from a CDN, the way CI does. Stage what you built and serve it locally with no credentials, then point the suites at it. A suite without `EDGE_CDN_BASE` fails before it starts, so no test reaches the production CDN.

```bash
cd infra && npm ci && npm run stage -- ../_cdn && npm run cdn:local -- ../_cdn # keep it running
```

```bash
export EDGE_CDN_BASE=http://127.0.0.1:8788
deno run -A npm:playwright install --with-deps chromium # once
deno test --allow-all js/tests/
cargo build --release --target wasm32-unknown-unknown -p slugify-mod
(cd cli && cargo test)
cargo test -p skill # every executable cell of skill/SKILL.md
```

The lock cases of the CLI serve their own registry on loopback through `EDGE_SITE_BASE`. Only `edge add` of an unknown name and `edge publish` with a bad token still reach the production one.

### Fuzzing and Miri

- `fuzz/` runs coverage-guided fuzzing of the lexer, parser, and VM on [cargo-afl](https://github.com/rust-fuzz/afl.rs). Campaigns and crash triage are in [Fuzzing](https://edgepython.com/docs/implementation/fuzzing).
- [`miri.yml`](.github/workflows/miri.yml) interprets the same corpora under [Miri](https://github.com/rust-lang/miri) once a day, on a nightly of its own, for undefined behavior the native build runs past. Run one module with `cargo +nightly miri test -p edge-python --test tests vm::`.

## Site and Docs

`docs/` holds MDX pages ordered by numeric prefix, and `site/` renders them under `/docs`. The site is Astro on Cloudflare Workers with a D1 database, and runs locally on miniflare with no credentials, printing the sign-in code to the terminal.

```bash
cd site && npm ci
npm run dev # port 4322
EDGE_ENV=prod npm run dev # the same tree as production sees it
npm run check
npx playwright install --with-deps chromium firefox webkit # once
npm test # builds the Worker and drives it in three engines
```

- [`site/src/draft.ts`](site/src/draft.ts) names what is still being built, a path for a page and a fragment for a surface inside one. Under `EDGE_ENV=prod` a page answers 404 and the rest is rewritten out of the html.
- An `edge-python` code block followed by an `output` block becomes a playground on the real engine, so every example and its output stay a verifiable pair.
- A page nests one folder deep at most, carries a numeric prefix on every path segment, opens with a closed frontmatter block holding a `title` and a `description`, and has exactly one top-level heading. `npm run build` refuses a page that breaks any of it, and `edge build` holds the `docs` directory of a package to the same rules. Both read [`convention.ts`](site/src/lib/docs/convention.ts) and [`docs.rs`](cli/src/docs.rs), kept in step by [`docs.json`](tests/cases/docs.json), so a rule changed on one side fails on the other.

## The bot

`bot/` answers questions about Edge Python out of the published pages, over http at `ask.edgepython.com` and in the Discord server. It reads the published `SKILL.md` and searches the site under `/api` the way any other client does, runs what an answer needs computed on the published `compiler.wasm`, which each command below fetches first, and [`bot.yml`](.github/workflows/bot.yml) ships a fix to it on a push that touches it.

```bash
cd bot && npm ci
npm run check # types and the wrangler config
npm test      # what it keeps, reads and runs, and how a reply looks
npm run dev   # the http side on a local database, with the model through your wrangler login
```

A test reaches no runtime and holds no credential, since D1 is SQLite and `node:sqlite` runs the same schema the deploy creates. What the model answers is read by hand through that http side instead.

`DISCORD` in [`names.ts`](bot/names.ts) set to anything but `1` ships that http side alone, the way `npm run dev` always runs it. What it keeps between questions is [`db/schema.sql`](bot/db/schema.sql), and how it is bound and silenced is in [RUNBOOK.md](RUNBOOK.md#the-discord-bot).

## Infra and CI

- `infra/` declares every Cloudflare resource in code and is checked with `npm run check` and `npm test`. `stage` and `cdn:local` run locally, and the other scripts deploy with `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
- [`main.yml`](.github/workflows/main.yml) runs CI and CD, each part a composite action under [`.github/actions/`](.github/actions). Build jobs stage their outputs on `cdn.tmp.edgepython.com`, test jobs run every suite against them, and a push to `main` promotes the tested tree to `dev.edgepython.com`.
- [`site/db/schema.sql`](site/db/schema.sql) is the whole database as it stands, and every local, test and dev database is built from it. How production takes a schema change is in [RUNBOOK.md](RUNBOOK.md#migrations).
