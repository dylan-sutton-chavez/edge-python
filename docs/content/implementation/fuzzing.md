---
title: "Fuzzing"
description: "Coverage-guided fuzzing of the lex, parse, and VM pipeline with cargo-afl on stable Rust."
---

## Overview

The fuzzer drives the full lex, parse, and VM pipeline against mutated input, looking for panics and memory faults. It lives in [`fuzz/`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/fuzz) and is built on [cargo-afl](https://github.com/rust-fuzz/afl.rs) (AFL++). On stable Rust it instruments through rustc's own LLVM SanitizerCoverage and links the AFL++ runtime, so no nightly toolchain is required.

The target runs the VM under the sandbox profile, so runaway loops and allocations become a `VmErr` instead of a hang. The harness tightens one field, `Limits { ops: 100_000, ..Limits::sandbox() }`. The default 100M-op budget is bounded but takes long enough that AFL would flag a legitimately terminating loop as a hang. The smaller budget keeps each execution inside AFL's hang timeout while still reaching deep into the language. The harness also sets `strict_input = true`, so `input()` raises instead of blocking on the real stdin that AFL feeds through shared memory. See [Limits and errors](/reference/limits-and-errors).

Most crashes are genuine bugs, not resource exhaustion. The exception is arithmetic-overflow panics, a debug-only artifact of `overflow-checks`. The release VM runs without them, so they are triaged out by hand rather than filtered automatically.

The build runs `--release`. `[profile.release]` sets `debug = "line-tables-only"` for `file:line` backtraces without the dev profile's heavier debuginfo. cargo-afl forces `opt-level=3`, `debug-assertions`, and `overflow-checks` regardless of profile. The debug assertions are what surface real bugs.

## Running it

```bash
cd fuzz
./seeds.sh                   # generate corpus + dictionary from vm.json (once)
cargo afl build --release    # instrument on stable, no nightly
cargo afl fuzz -i in -o out -x edge.dict target/release/afl-pipeline  # runs until Ctrl-C; add -V 300 to stop after 300s

cargo afl whatsup out        # status summary; run in another terminal while fuzzing
```

For a parallel run across the host cores, `./deploy.sh` builds, regenerates seeds, and launches one `-M` plus N-1 `-S` instances sharing one `out/`. It runs one instance per logical core by default. Override with `JOBS`. `DURATION`, `FRESH`, and `TIMEOUT_MS` are optional too. The same target runs on a daily schedule in CI via [`.github/workflows/fuzzer.yml`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/.github/workflows/fuzzer.yml), which calls `deploy.sh` directly on the runner (no container) and fails the run on any saved crash.

| Variable | Default | Description |
|----------|---------|-------------|
| `JOBS` | `$(nproc)` | number of AFL instances, one per logical core |
| `DURATION` | `0` | campaign length in seconds (`0` runs until stopped) |
| `FRESH` | `0` | set to `1` to delete `out/` and start clean |
| `TIMEOUT_MS` | `5000` | per-input hang threshold in ms (should exceed the max bounded VM run) |

`deploy.sh` also forces `FRESH=1` automatically when the binary changed since the last run.

## Container campaigns

For a long-running campaign, `compose.yml` builds the image from `Dockerfile` and runs the same `deploy.sh`. Findings persist in the `findings` volume, mounted at `/app/fuzz/out` in the container, instead of CI's 14-day artifact. The service sets `restart: unless-stopped`, so the campaign survives host reboots and stops only on `docker compose down`. It also sets `AFL_NO_AFFINITY=1`, because a container hides the host topology and AFL must not pin instances to cores it cannot see.

```bash
cd fuzz
DURATION=3600 docker compose up --build -d   # detached; same JOBS / FRESH / TIMEOUT_MS overrides apply

docker compose ps          # Up vs Restarting
docker compose logs -f     # raw deploy output: seed count, instance count, startup errors

# Live status (-s = aggregated summary; drop it for per-instance metrics).
docker compose exec -it fuzzer bash -c "cd fuzz && watch -n 10 cargo afl whatsup -s out"

docker compose down        # stop the campaign

# Every saved crash across all instances and archived dirs.
docker compose exec -T fuzzer bash -c 'cd fuzz && find out -type f -path "*crashes*" ! -name README.txt'
```

Removing the container leaves the `findings` volume and the built image behind. The next `up` resumes the old `out/` from that volume, which is the usual cause of a campaign that starts stuck. For a full reset:

```bash
docker compose down -v                        # remove container and findings volume
docker rmi edge-python-afl-fuzzer:latest      # drop the image
docker builder prune -f                       # reclaim the build cache
```

Note that plain `docker compose down` keeps named volumes. Only `down -v` deletes the `findings` volume holding the campaign.

## Resuming and rebuilds

Reusing the same `out/` resumes the campaign. AFL recalibrates the saved queue before fuzzing, so `execs` sits at 0 for a while. Resume is only safe when the target binary is unchanged. After a rebuild, the saved coverage map no longer matches the new binary. With `AFL_AUTORESUME=1` (which `deploy.sh` sets) the instances do not abort cleanly. They stall recalibrating the inherited queue, which can be tens of thousands of entries from a long prior campaign. While they grind through it, `cargo afl whatsup` reports them as dead with `execs` and run time at 0 and shows the stale coverage percentage from the previous session. That looks like a crash but is just resume over a changed binary.

`deploy.sh` guards against this. After the build it sha1-sums the instrumented binary and compares the hash to `out/.binary-hash`. On a mismatch it forces `FRESH=1` and wipes `out/` before launching. A bare `cargo afl fuzz` has no such guard, so after a rebuild always start fresh yourself with `rm -rf out`.

`deploy.sh` sets the bypass vars itself. A bare `cargo afl fuzz` under WSL needs `AFL_SKIP_CPUFREQ=1 AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1` prefixed, to skip the core-pattern and CPU-governor checks.

## Reproducing a crash

Where findings land depends on how you launched. A bare `cargo afl fuzz` writes to `out/default/`. `deploy.sh`, compose, and CI pass `-M m0` and `-S s1...`, so crashes and hangs land in `out/m0/`, `out/s1/`, and so on. Reproduce one by piping it back into the target:

```bash
./target/release/afl-pipeline < out/m0/crashes/<id>   # out/default/crashes/<id> for a bare run
```

In a container campaign, list the saved crashes and reproduce one with a backtrace:

```bash
docker compose exec -it fuzzer bash -c "cd fuzz && find out -type f -path '*crashes*' ! -name README.txt"
docker compose exec -it fuzzer bash -c "cd fuzz && RUST_BACKTRACE=1 ./target/release/afl-pipeline < 'out/m0/crashes/<id>' 2>&1 | head -20"
```

## Triaging crashes

A parallel campaign saves one file per crashing input, not one per bug. A single panic site is reached by many distinct inputs, so `out/*/crashes/` overstates the real bug count. Reproduce each saved crash and group by panic site. Each unique `file:line` is one bug to fix:

```bash
for f in $(find out -type f -path '*crashes*' ! -name README.txt); do ./target/release/afl-pipeline < "$f" 2>&1 | grep -oE 'panicked at [^:]+:[0-9]+'; done | sort | uniq -c
```

Each time an instance resumes an existing `out/`, AFL archives the prior `crashes/` and `hangs/` to timestamped `crashes.<date>/` and `hangs.<date>/` directories and starts empty ones. A long campaign accumulates many archive dirs. Glob `*crashes*` and `*hangs*`, not just `crashes/`, or you only see the current (often empty) session. The live `saved_crashes` counter in `fuzzer_stats` can read non-zero while the active `crashes/` holds nothing but `README.txt`. The files are in the archived dirs.

Shrink one crash to its minimal reproducer with `cargo afl tmin`, which feeds the case over stdin:

```bash
cargo afl tmin -i out/m0/crashes/<id> -o crash.min -- ./target/release/afl-pipeline
```

Hangs have no backtrace to group by. The op bound turns a genuine runaway loop into a `VmErr`, so a saved hang is usually an input that terminated but ran past `TIMEOUT_MS`, not a real lock-up. Confirm by re-running under a wall-clock timeout, where exit 124 means genuinely stuck:

```bash
for f in $(find out -type f -path '*hangs*' ! -name README.txt); do timeout 10 ./target/release/afl-pipeline < "$f" >/dev/null 2>&1; echo "$? $f"; done
```

## Inputs are generated, not committed

The seed corpus (`in/`) derives from the single source of truth `tests/cases/vm.json`. The token dictionary is authored in `dict.txt`. `seeds.sh` regenerates the gitignored `in/` and copies `dict.txt` to the gitignored `edge.dict` that AFL consumes:

- **`in/`**: one file per unique program `src` in the VM test fixtures, giving AFL valid starting points that already exercise most of the language.
- **`dict.txt` -> `edge.dict`**: keywords, operators, dunders, boundary literals, and multi-token idioms, so the byte mutator splices real tokens instead of discovering them blindly. Edit `dict.txt` to grow it.

Seven files are tracked: `Cargo.toml`, `src/main.rs`, `seeds.sh`, `dict.txt`, `deploy.sh`, `Dockerfile`, and `compose.yml`. The corpus, `edge.dict`, AFL output, and build artifacts are all reproducible.

## References

1. Fioraldi et al. *AFL++: Combining Incremental Steps of Fuzzing Research* (WOOT 2020). The fuzzer this target runs on.
2. LLVM. *SanitizerCoverage* ([clang docs](https://clang.llvm.org/docs/SanitizerCoverage.html)). The stable-Rust instrumentation path.
