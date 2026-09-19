#!/usr/bin/env bash

set -e

INSTALL_DIR="${EDGE_INSTALL_DIR:-$HOME/.local/bin}"

# Everything the CLI caches, edge-owned and refetchable, so it goes with the binary.
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/edge"

# 1. Binary.
if [ -f "$INSTALL_DIR/edge" ]; then
  rm -f "$INSTALL_DIR/edge"
  echo "removed $INSTALL_DIR/edge"
else
  echo "no edge binary at $INSTALL_DIR/edge"
fi

# 2. PATH entries. Leave a .edgebak in case the user wants to roll it back.
for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
  [ -f "$rc" ] || continue
  if grep -qs "$INSTALL_DIR" "$rc"; then
    sed -i.edgebak "\|export PATH=\"$INSTALL_DIR:\$PATH\"|d" "$rc"
    echo "cleaned edge entries from $rc (backup at $rc.edgebak)"
  fi
done

# 3. Cache. Unconditional, nothing else reads it and every entry refetches on demand.
if [ -d "$CACHE_DIR" ]; then
  rm -rf "$CACHE_DIR"
  echo "removed $CACHE_DIR"
fi

echo "edge removed."
