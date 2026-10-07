#!/usr/bin/env bash
# Regenerates the gitignored in/ and edge.dict from tests/cases/vm.json and dict.txt.
set -euo pipefail
cd "$(dirname "$0")"

# One seed file per unique src in the VM fixtures, unescaped from its JSON string and named by its hash.
rm -rf in && mkdir -p in
perl -MDigest::SHA=sha1_hex -ne '
    while (/"src":\s*"((?:[^"\\]|\\.)*)"/g) {
        my $s = $1;
        $s =~ s/\\(.)/$1 eq "n" ? "\n" : $1 eq "t" ? "\t" : $1 eq "r" ? "\r" : $1/ge;
        next if $s eq "";
        open my $out, ">", "in/" . substr(sha1_hex($s), 0, 16) or die $!;
        print $out $s;
    }' ../tests/cases/vm.json
echo "seeds: $(ls in | wc -l | tr -d ' ')"

cp dict.txt edge.dict
echo "dict: $(grep -c '^"' edge.dict) entries"
