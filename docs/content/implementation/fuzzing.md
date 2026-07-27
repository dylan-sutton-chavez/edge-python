---
title: "Fuzzing"
description: "Coverage-guided fuzzing of the lex, parse, and VM pipeline with cargo-afl on stable Rust."
---

## Overview

The fuzzer drives the full `lex -> parse -> VM` pipeline against mutated input. It looks for panics and memory faults. It lives in [`fuzz/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/fuzz) and is built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++). On stable it instruments through rustc's own LLVM SanitizerCoverage (`-sanitizer-coverage-trace-pc-guard`) and links the AFL++ runtime. So it runs on **stable Rust**, no nightly toolchain required. (AFL++'s own LLVM passes, CMPLOG and IJON, are an optional, nightly-only path the project does not use.)

The target runs the VM under the sandbox profile. Runaway loops and allocations become a `VmErr` instead of a hang. Most crashes are genuine bugs, not resource exhaustion; the exception is arithmetic-overflow panics — a debug-only artifact of `overflow-checks` (off in the release VM) — which are triaged out by hand, not filtered automatically. The harness tightens one field: `Limits { ops: 100_000, ..Limits::sandbox() }`. The default 100M-op budget is bounded, but takes long enough that AFL would flag a legitimately-terminating loop (or wide recursion) as a hang. The smaller budget keeps each execution inside AFL's hang timeout while still reaching deep into the language. It also sets `strict_input = true`, so `input()` raises instead of blocking on real stdin, which AFL feeds through shared memory. See [Limits and errors](/reference/limits-and-errors).

The build runs `--release`. `[profile.release]` sets `debug = "line-tables-only"` for `file:line` backtraces without the `dev` profile's heavier debuginfo. (cargo-afl forces `opt-level=3`, `debug-assertions`, and `overflow-checks` into `RUSTFLAGS` regardless of profile. `debug-assertions` are what surface real bugs; `overflow-checks` only add arithmetic-overflow panics, triaged out by hand since the release VM runs without them.)

## Running it

```bash
cd fuzz
./seeds.sh # generate corpus + dictionary from vm.json (once)
cargo afl build --release # instrument on stable, no nightly
cargo afl fuzz -i in -o out -x edge.dict target/release/afl-pipeline # runs until Ctrl-C; add -V 300 to stop after 300s

cargo afl whatsup out # status summary of the ./out campaign; run in another terminal while fuzzing
```

For a parallel run across the host cores, `./deploy.sh` builds, regenerates seeds, and launches one `-M` plus N-1 `-S` instances sharing one `out/`. It runs one instance per logical core by default. Override with `JOBS`. `DURATION` / `FRESH` / `TIMEOUT_MS` are optional too. The same target runs on a daily schedule in CI via [`.github/workflows/fuzzer.yml`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/.github/workflows/fuzzer.yml), which calls `deploy.sh` directly on the runner (no container) and fails the run on any saved crash.

For a long-running campaign in a container, `compose.yml` builds the image from `Dockerfile` and runs the same `deploy.sh`. Findings persist in the `findings` volume mounted at `fuzz/out/`, instead of CI's 14-day artifact. It sets `restart: unless-stopped`, so the campaign survives host reboots and only stops when you explicitly run `docker compose down`. It also sets `AFL_NO_AFFINITY=1`: a container hides the host topology, and AFL must not pin instances to cores it cannot see.

| Variable | Default | Description |
|----------|---------|-------------|
| `JOBS` | `$(nproc)` | Number of AFL instances (one per logical core). |
| `DURATION` | `0` | Campaign length in seconds. `0` runs until stopped. |
| `FRESH` | `0` | Set to `1` to delete `out/` and start a clean campaign. `deploy.sh` also forces this automatically when the binary changed since the last run. |
| `TIMEOUT_MS` | `5000` | Per-input hang threshold in ms. Should exceed the max bounded VM run. |

```bash
cd fuzz
DURATION=3600 docker compose up --build -d # detached; same JOBS / FRESH / TIMEOUT_MS overrides apply

docker compose ps # Up vs Restarting
docker compose logs -f # raw deploy output: seed count, instance count, startup errors

# Live status (-s = aggregated summary; drop it for per-instance metrics).
docker compose exec -it fuzzer bash -c "cd fuzz && watch -n 10 cargo afl whatsup -s out"

docker compose down # stop the campaign

docker compose exec -T fuzzer bash -c 'cd fuzz && find out -type f -path "*crashes*" ! -name README.txt' # Every saved crash across all instances and archived dirs
docker compose exec -T fuzzer bash -c 'cd fuzz && base64 out/m0/crashes/<id>' # Take a look to the bug in base 64
docker compose exec -T fuzzer bash -c "cd fuzz && find out -type f -path '*crashes*' ! -name README.txt -print0 | tar --null -cf - -T -" > ~/crashes.tar # Bundle every crash into a tar on the host
```

If a container is stuck restarting and `docker compose down` won't clear it, force-remove it by id:

```bash
docker ps # find the container id
docker rm -f <id> # force-remove it, even mid-restart
```

Removing the container (or `docker compose down`) leaves the `findings` volume and the built image behind. The next `up` then resumes the old `out/` from that volume, which is the usual cause of a campaign that starts stuck. For a full reset that frees the host:

```bash
docker compose down -v # remove container and the findings volume
docker volume ls # confirm no edge-python-afl_findings remains
docker rmi edge-python-afl-fuzzer:latest # drop the image
docker builder prune -f # reclaim the build cache
```

For the lifecycle and recovery commands themselves, see Docker's own guides:

- [restart policies](https://docs.docker.com/engine/containers/start-containers-automatically/): what `restart: unless-stopped` does and why a crash-looping process keeps coming back.
- [`docker compose down`](https://docs.docker.com/reference/cli/docker/compose/down/): removes the container but **keeps named volumes**; only `down -v` deletes the `findings` volume holding the campaign.
- [`docker compose up`](https://docs.docker.com/reference/cli/docker/compose/up/): `--force-recreate` re-creates the container from the existing image, preserving the binary; add `--build` only to pick up code changes.

Reusing the same `out/` resumes the campaign. AFL recalibrates the saved queue (the dry-run pass) before fuzzing, so `execs` sits at 0 for a while. Delete it with `rm -rf out` for a clean start. Resume is only safe when the target binary is unchanged. After a rebuild (any code change) the saved coverage map and `fastresume.bin` no longer match the new binary. With `AFL_AUTORESUME=1` (which `deploy.sh` sets) the instances do not abort cleanly: they stall re-calibrating the entire inherited queue, which can be tens of thousands of entries from a long prior campaign. While they grind through it `cargo afl whatsup` reports them as dead with `execs` and run time at 0, and shows the stale coverage percentage from the previous session's `fuzzer_stats`. That looks like a crash but is just resume over a changed binary.

`deploy.sh` guards against this automatically. After the build it sha1-sums the instrumented binary and compares it to the hash saved in `out/.binary-hash`; on a mismatch it forces `FRESH=1` and wipes `out/` before launching, then records the new hash. So a rebuilt binary always starts a clean campaign without manual intervention. A bare `cargo afl fuzz` (without `deploy.sh`) has no such guard, so always start fresh (`rm -rf out`) yourself after a rebuild.

`deploy.sh` sets the bypass vars itself. A bare `cargo afl fuzz` under WSL needs `AFL_SKIP_CPUFREQ=1 AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1` prefixed, to skip the core-pattern and CPU-governor checks.

Where findings land depends on how you launched. A bare `cargo afl fuzz` (no `-M`/`-S`) writes to `out/default/`. `deploy.sh`, compose, and CI pass `-M m0` / `-S s1…`, so crashes and hangs land in `out/m0/`, `out/s1/`, etc. Reproduce one by piping it back into the target:

```bash
./target/release/afl-pipeline < out/m0/crashes/<id> # out/default/crashes/<id> for a bare single-instance run
```

In a container campaign, list the saved crashes (the `m0`/`s1` dir is the instance) and reproduce one with a backtrace:

```bash
docker compose exec -it fuzzer bash -c "cd fuzz && find out -type f -path '*crashes*' ! -name README.txt"
docker compose exec -it fuzzer bash -c "cd fuzz && RUST_BACKTRACE=1 ./target/release/afl-pipeline < 'out/m0/crashes/<id>' 2>&1 | head -20"
```

## Triaging crashes

A parallel campaign saves one file per crashing *input*, not one per bug. A single panic site is reached by many distinct inputs, so `out/*/crashes/` overstates the real bug count. Reproduce each saved crash and group by panic site. Each unique `file:line` is one bug to fix:

```bash
for f in $(find out -type f -path '*crashes*' ! -name README.txt); do ./target/release/afl-pipeline < "$f" 2>&1 | grep -oE 'panicked at [^:]+:[0-9]+'; done | sort | uniq -c
```

Each time an instance resumes an existing `out/`, AFL archives the prior `crashes/` and `hangs/` to timestamped `crashes.<date>/` / `hangs.<date>/` and starts empty ones. A long campaign accumulates many archive dirs (one per restart). Glob `*crashes*` / `*hangs*`, not just `crashes/`, or you only see the current (often empty) session. The live `fuzzer_stats` `saved_crashes` counter can read non-zero while the active `crashes/` holds nothing but `README.txt`. The files are in the archived dirs.

Shrink one crash to its minimal reproducer with `cargo afl tmin` (feeds the case over stdin; no `@@`):

```bash
cargo afl tmin -i out/m0/crashes/<id> -o crash.min -- ./target/release/afl-pipeline
```

Hangs have no backtrace to group by. The op-bound (`Limits { ops: 100_000 }`) turns a genuine runaway loop into a `VmErr`. So a saved hang is usually an input that terminated but ran past `TIMEOUT_MS`, not a real lock-up. Confirm by re-running under a wall-clock timeout, where exit 124 means genuinely stuck:

```bash
for f in $(find out -type f -path '*hangs*' ! -name README.txt); do timeout 10 ./target/release/afl-pipeline < "$f" >/dev/null 2>&1; echo "$? $f"; done
```

## Inputs are generated, not committed

The seed corpus (`in/`) derives from the single source of truth `tests/cases/vm.json`. The token dictionary is authored in `dict.txt`. `seeds.sh` regenerates the gitignored `in/` and copies `dict.txt` to the gitignored `edge.dict` that AFL consumes:

- **`in/`**: one file per unique program `src` in the VM test fixtures, giving AFL valid starting points that already exercise most of the language.
- **`dict.txt` -> `edge.dict`**: keywords, operators, dunders, boundary literals, and multi-token idioms, so the byte mutator splices real tokens instead of discovering them blindly. Edit `dict.txt` to grow it.

Seven files are tracked: `Cargo.toml`, `src/main.rs`, `seeds.sh`, `dict.txt`, `deploy.sh`, `Dockerfile`, and `compose.yml`. The corpus, `edge.dict`, AFL output, and build artifacts are reproducible.

## References

1. Fioraldi et al. *AFL++: Combining Incremental Steps of Fuzzing Research* (WOOT 2020). The fuzzer this target runs on.
2. LLVM. *SanitizerCoverage* ([clang docs](https://clang.llvm.org/docs/SanitizerCoverage.html)). The stable-Rust instrumentation path.
