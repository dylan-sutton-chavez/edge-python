#!/usr/bin/env bash
# Counts saved crashes by panic site, since one bug saves many inputs, and counts the saved hangs.
set -euo pipefail
cd "$(dirname "$0")"

bin=target/release/afl-pipeline
find out -type f -path '*crashes*' ! -name README.txt | while IFS= read -r f; do
    "$bin" < "$f" 2>&1 | grep -oE 'panicked at [^:]+:[0-9]+' || true
done | sort | uniq -c | sort -rn
echo "hangs: $(find out -type f -path '*hangs*' ! -name README.txt | wc -l | tr -d ' ')"
