#!/usr/bin/env bash
# Monitor a running campaign from another shell with: cargo afl whatsup out
set -euo pipefail
cd "$(dirname "$0")"

JOBS="${JOBS:-$(nproc)}" # One instance per logical core.
DURATION="${DURATION:-0}"
FRESH="${FRESH:-0}"
TIMEOUT_MS="${TIMEOUT_MS:-5000}" # > max bounded run, so slow-but-terminating inputs aren't false hangs

export AFL_SKIP_CPUFREQ=1
export AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1
export AFL_NO_UI=1
export AFL_AUTORESUME=1 # resume existing out/ on restart instead of aborting; FRESH=1 wipes it first

(( JOBS < 1 )) && JOBS=1
echo "logical cores: $(nproc), instances: $JOBS"

# Regenerate the seed corpus / dictionary if absent, then build the instrumented target.
[ -d in ] && [ -n "$(ls -A in 2>/dev/null)" ] || bash ./seeds.sh
cargo afl build --release

# Rebuild invalidates resume; force clean start.
bin=target/release/afl-pipeline
newhash=$(sha1sum "$bin" | cut -d' ' -f1)
if [ -s out/.binary-hash ] && [ "$(cat out/.binary-hash)" != "$newhash" ]; then
  echo "instrumented binary changed; forcing FRESH start"
  FRESH=1
fi

# `out` is a mounted volume in the container; clear its contents, not the mount point itself.
[ "$FRESH" = "1" ] && { find out -mindepth 1 -delete 2>/dev/null || true; }
mkdir -p out logs
printf '%s\n' "$newhash" > out/.binary-hash

# -V time-box only when DURATION > 0.
vflag=()
(( DURATION > 0 )) && vflag=(-V "$DURATION")

pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; }
trap cleanup INT TERM EXIT

# One main (deterministic) instance, the rest secondaries (havoc); all share -o out and sync.
cargo afl fuzz "${vflag[@]}" -t "$TIMEOUT_MS" -i in -o out -x edge.dict -M m0 target/release/afl-pipeline >logs/m0.log 2>&1 &
pids+=($!)
for i in $(seq 1 $(( JOBS - 1 ))); do
  cargo afl fuzz "${vflag[@]}" -t "$TIMEOUT_MS" -i in -o out -x edge.dict -S "s$i" target/release/afl-pipeline >"logs/s$i.log" 2>&1 &
  pids+=($!)
done

echo "running $JOBS instance(s); logs in ./logs, status: cargo afl whatsup out"
wait
