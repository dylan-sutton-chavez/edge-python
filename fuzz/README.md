# Fuzzing

The fuzzer drives the full lex, parse and VM pipeline with mutated input. It looks for panics and memory faults. It is built on [cargo-afl](https://github.com/rust-fuzz/afl.rs), which runs AFL++.

- On stable Rust it instruments through the SanitizerCoverage of the LLVM inside rustc and links the AFL++ runtime. No nightly toolchain is needed.
- Only programs that parse reach the VM. A chunk after a parse error is not reliable.
- The VM preempts every 7 back-edges. At the first park the harness saves a snapshot and restores it into a fresh VM. The snapshot round trip is fuzzed too.
- A `receive()` park gets an event pushed, at most 16 times per input.
- `input()` raises, since the harness gives no host data. It never blocks on the stdin AFL feeds.

The [Runbook](https://edgepython.com/docs/internals/runbook) gives the short public overview.

## The op budget

The target runs the VM under the sandbox profile. Runaway loops and allocations become a `VmErr` instead of a hang. The harness tightens one field.

```rust
Limits { ops: 100_000, ..Limits::sandbox() }
```

The default budget of 100 million ops is bounded, but a loop that ends legitimately can take long enough for AFL to flag it as a hang. The smaller budget keeps each execution inside the hang timeout of AFL and still reaches deep into the language. See [Limits and errors](https://edgepython.com/docs/reference/limits-and-errors).

## Build profile

The build runs `--release`.

- `[profile.release]` sets `debug = "line-tables-only"`. Backtraces get `file:line` without the heavier debuginfo of the dev profile.
- cargo-afl forces `opt-level=3`, `debug-assertions` and `overflow-checks` whatever the profile says.
- The debug assertions are what surface real bugs.

Most crashes are genuine bugs, not resource exhaustion. The exception is an arithmetic-overflow panic. It comes from `overflow-checks`, and the release VM runs without them. Triage drops those by hand, nothing filters them automatically.

## Running it

From the root of the repo, `make fuzz` runs a parallel campaign across the cores of the host. It calls `deploy.sh`.

```bash
make seeds   # regenerate the corpus and the dictionary from vm.json
make fuzz    # build, then one -M and N-1 -S instances sharing one out/
```

`deploy.sh` regenerates the seeds when `in/` is missing or empty, then builds the instrumented target. It takes these environment variables.

| Variable | Default | Meaning |
|---|---|---|
| `JOBS` | `$(nproc)` | Number of AFL instances, one per logical core |
| `DURATION` | `0` | Campaign length in seconds, `0` runs until stopped |
| `FRESH` | `0` | `1` deletes `out/` and starts clean |
| `TIMEOUT_MS` | `5000` | Hang threshold per input in ms, above the longest bounded VM run |

`deploy.sh` also forces `FRESH=1` when the binary changed since the last run. Check a running campaign from another shell with `cargo afl whatsup fuzz/out`.

A single instance runs by hand from `fuzz/`.

```bash
cd fuzz
cargo afl build --release    # instrument on stable, no nightly
cargo afl fuzz -i in -o out -x edge.dict target/release/afl-pipeline  # until Ctrl-C, add -V 300 to stop after 300 s
cargo afl whatsup out        # status summary, from another terminal
```

> [!NOTE]
> On macOS, `seeds.sh` needs GNU grep, since BSD grep has no `-P`. `deploy.sh` calls `nproc`, and macOS lacks it. Set `JOBS` there.

The same target runs every day in CI through [`.github/workflows/fuzzer.yml`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/.github/workflows/fuzzer.yml).

- It calls `make fuzz` on the runner, with no container.
- The run lasts 1,200 seconds by default. A manual dispatch can set another duration.
- Any saved crash fails the run. Hangs only raise a warning.
- The crashes and hangs are uploaded as the `fuzz-findings` artifact and kept for 14 days.

## Container campaigns

For a long campaign, `compose.yml` builds the image from `Dockerfile` and runs the same `deploy.sh`.

- Findings persist in the `findings` volume, mounted at `/app/fuzz/out` in the container. CI keeps its artifact only 14 days.
- The service sets `restart: unless-stopped`. The campaign survives host reboots and stops only on `docker compose down`.
- It sets `AFL_NO_AFFINITY=1`. A container hides the topology of the host, and AFL must not pin instances to cores it cannot see.

```bash
cd fuzz
DURATION=3600 docker compose up --build -d   # detached, the same JOBS, FRESH and TIMEOUT_MS overrides apply

docker compose ps          # Up vs Restarting
docker compose logs -f     # raw deploy output, seed count, instance count, startup errors

# Live status, -s is the aggregated summary, drop it for metrics per instance.
docker compose exec -it fuzzer bash -c "cd fuzz && watch -n 10 cargo afl whatsup -s out"

docker compose down        # stop the campaign

# Every saved crash across all instances and archived dirs.
docker compose exec -T fuzzer bash -c 'cd fuzz && find out -type f -path "*crashes*" ! -name README.txt'
```

Removing the container leaves the `findings` volume and the built image behind. The next `up` resumes the old `out/` from that volume. That is the usual cause of a campaign that starts stuck. A full reset takes three commands.

```bash
docker compose down -v                        # remove container and findings volume
docker rmi edge-python-afl-fuzzer:latest      # drop the image
docker builder prune -f                       # reclaim the build cache
```

> [!WARNING]
> Plain `docker compose down` keeps named volumes. Only `down -v` deletes the `findings` volume that holds the campaign.

## Resuming and rebuilds

Reusing the same `out/` resumes the campaign. AFL recalibrates the saved queue before fuzzing, and `execs` sits at 0 for a while. Resume is safe only when the target binary is unchanged. After a rebuild the saved coverage map no longer matches it.

`deploy.sh` sets `AFL_AUTORESUME=1`. Over a changed binary the instances then do not abort cleanly.

- They stall recalibrating the inherited queue. After a long prior campaign it can hold tens of thousands of entries.
- Meanwhile `cargo afl whatsup` reports them as dead, with `execs` and run time at 0.
- It also shows the stale coverage percentage of the previous session.

That looks like a crash, but it is only a resume over a changed binary. `deploy.sh` guards against it. After the build it takes the sha1 of the instrumented binary and compares it to `out/.binary-hash`. On a mismatch it forces `FRESH=1` and wipes `out/` before launching.

A bare `cargo afl fuzz` has no such guard. After a rebuild, start fresh yourself with `rm -rf out`.

`deploy.sh` also sets the bypass variables itself. A bare `cargo afl fuzz` under WSL needs `AFL_SKIP_CPUFREQ=1 AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1` in front. They skip the core-pattern and CPU-governor checks.

## Reproducing a crash

Where findings land depends on how the campaign started.

- A bare `cargo afl fuzz` writes to `out/default/`.
- `deploy.sh`, compose and CI pass `-M m0` and `-S s1` onward. Crashes and hangs land in `out/m0/`, `out/s1/`, and so on.

Reproduce one by piping it back into the target from `fuzz/`.

```bash
./target/release/afl-pipeline < out/m0/crashes/<id>   # out/default/crashes/<id> for a bare run
```

In a container campaign, list the saved crashes and reproduce one with a backtrace.

```bash
docker compose exec -it fuzzer bash -c "cd fuzz && find out -type f -path '*crashes*' ! -name README.txt"
docker compose exec -it fuzzer bash -c "cd fuzz && RUST_BACKTRACE=1 ./target/release/afl-pipeline < 'out/m0/crashes/<id>' 2>&1 | head -20"
```

## Triaging crashes

A parallel campaign saves one file per crashing input, not one per bug. Many distinct inputs reach a single panic site. `out/*/crashes/` overstates the real bug count. Reproduce each saved crash and group by panic site. Each unique `file:line` is one bug to fix.

```bash
for f in $(find out -type f -path '*crashes*' ! -name README.txt); do ./target/release/afl-pipeline < "$f" 2>&1 | grep -oE 'panicked at [^:]+:[0-9]+'; done | sort | uniq -c
```

Each time an instance resumes an existing `out/`, AFL archives the prior `crashes/` and `hangs/`.

- They move to timestamped `crashes.<date>/` and `hangs.<date>/` directories, and empty ones start.
- A long campaign accumulates many archive dirs.
- Glob `*crashes*` and `*hangs*`, not only `crashes/`. Otherwise you see only the current session, which is often empty.
- The live `saved_crashes` counter in `fuzzer_stats` can read non-zero while the active `crashes/` holds only `README.txt`. The files are in the archived dirs.

Shrink one crash to its minimal reproducer with `cargo afl tmin`. It feeds the case over stdin.

```bash
cargo afl tmin -i out/m0/crashes/<id> -o crash.min -- ./target/release/afl-pipeline
```

Hangs have no backtrace to group by. The op bound turns a real runaway loop into a `VmErr`. A saved hang is usually an input that ended but ran past `TIMEOUT_MS`, not a real lock-up. Confirm by running it again under a wall-clock timeout, where exit 124 means it is truly stuck.

```bash
for f in $(find out -type f -path '*hangs*' ! -name README.txt); do timeout 10 ./target/release/afl-pipeline < "$f" >/dev/null 2>&1; echo "$? $f"; done
```

## Inputs are generated, not committed

The seed corpus in `in/` derives from `tests/cases/vm.json`, the single source of truth. The token dictionary is written by hand in `dict.txt`. `make seeds` runs `seeds.sh`. It regenerates the gitignored `in/` and copies `dict.txt` to the gitignored `edge.dict` that AFL reads.

- `in/` holds one file per unique program `src` in the VM fixtures. AFL starts from valid programs that already exercise most of the language.
- `edge.dict` holds keywords, operators, dunders, boundary literals and idioms of several tokens. The byte mutator splices real tokens instead of finding them blindly. Edit `dict.txt` to grow it.

Seven files are tracked, `Cargo.toml`, `src/main.rs`, `seeds.sh`, `dict.txt`, `deploy.sh`, `Dockerfile` and `compose.yml`. The corpus, `edge.dict`, the AFL output and the build artifacts are all reproducible.

## References

1. Fioraldi et al. *AFL++. Combining Incremental Steps of Fuzzing Research* (WOOT 2020). The fuzzer this target runs on.
2. LLVM. *SanitizerCoverage* ([clang docs](https://clang.llvm.org/docs/SanitizerCoverage.html)). The instrumentation path on stable Rust.
