#!/usr/bin/env bash
# Regenerates the fuzz inputs from the single source of truth (tests/cases/vm.json). `in/` (seed corpus) and `edge.dict` are gitignored artifacts; run this once before fuzzing. Pure bash, no extra runtime.
set -euo pipefail
cd "$(dirname "$0")"

# Seed corpus: one file per unique `src` in the VM test fixtures. grep -oP pulls each JSON string body (handling \" and \\); sed unescapes the common escapes.
rm -rf in && mkdir -p in
while IFS= read -r raw; do
    src=$(printf '%s' "$raw" | sed -e 's/\\\\/\x01/g' \
        -e 's/\\n/\n/g' -e 's/\\t/\t/g' -e 's/\\r/\r/g' -e 's/\\"/"/g' \
        -e 's/\x01/\\/g')
    [ -z "$src" ] && continue
    name=$(printf '%s' "$src" | sha1sum | cut -c1-16)
    printf '%s' "$src" > "in/$name"
done < <(grep -oP '"src":\s*"\K(?:[^"\\]|\\.)*' ../tests/cases/vm.json)
echo "seeds: $(ls in | wc -l)"

# Token dictionary lives in dict.txt (committed source); copy it to the gitignored artifact AFL consumes.
cp dict.txt edge.dict
echo "dict: $(grep -c '^"' edge.dict) entries"
