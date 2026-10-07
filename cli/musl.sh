#!/bin/sh
# Builds the static Linux CLI inside Alpine, whose C++ is musl's own, so SpiderMonkey links in.
set -eu
apk add -q --no-cache bash clang clang-dev linux-headers llvm make python3 zlib-static
export CC=clang CXX=clang++
# A cached archive links as it is, and without one SpiderMonkey builds from source.
[ -n "${MOZJS_ARCHIVE:-}" ] || export MOZJS_FROM_SOURCE=1
# cc-rs names the target unknown, Alpine's libstdc++ keeps its headers under the alpine triple.
export CXXFLAGS="-I$(echo /usr/include/c++/*/*-alpine-linux-musl)"
# Build scripts stay dynamic so bindgen can load libclang, only the edge binary links static.
RUSTFLAGS="-C target-feature=-crt-static" cargo rustc --locked --release --bin edge -- -C target-feature=+crt-static
