#!/usr/bin/env bash

set -e

INSTALL_DIR="${EDGE_INSTALL_DIR:-$HOME/.local/bin}"

# Downloaded modules and their pins, edge-owned and refetchable, so they go with the binary.
MODULE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/edge/modules"

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

# 3. Module cache. Unconditional, nothing else reads it and every entry refetches on demand.
if [ -d "$MODULE_DIR" ]; then
  rm -rf "$MODULE_DIR"
  echo "removed $MODULE_DIR"
fi

echo "edge removed."
