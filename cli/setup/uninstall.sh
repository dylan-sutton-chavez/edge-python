#!/usr/bin/env bash
# Remove the edge binary, its PATH entry, and the bundled chrome-headless-shell cache.

set -e

INSTALL_DIR="${EDGE_INSTALL_DIR:-$HOME/.local/bin}"

# Browser model: install.sh downloads a pinned chrome-headless-shell to ~/.cache/edge, so uninstall just removes that directory: no package-manager dispatch, no sudo, and we never touch system Chromium that other apps may depend on.
CHROME_DIR="${EDGE_CHROME_DIR:-$HOME/.cache/edge}"

# Downloaded plugins and their pins, edge-owned and refetchable, so they go with the binary.
NATIVE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/edge-native"

# 1. Binary.
if [ -f "$INSTALL_DIR/edge" ]; then
  rm -f "$INSTALL_DIR/edge"
  echo "removed $INSTALL_DIR/edge"
else
  echo "no edge binary at $INSTALL_DIR/edge"
fi

# 2. PATH and EDGE_CHROME_PATH entries. Leave a .edgebak in case the user wants to roll it back.
for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
  [ -f "$rc" ] || continue
  changed=0
  if grep -qs "$INSTALL_DIR" "$rc"; then
    sed -i.edgebak "\|export PATH=\"$INSTALL_DIR:\$PATH\"|d" "$rc"
    changed=1
  fi
  if grep -qs 'EDGE_CHROME_PATH=' "$rc"; then
    [ -f "$rc.edgebak" ] || cp "$rc" "$rc.edgebak"
    sed -i.tmp '/EDGE_CHROME_PATH=/d' "$rc" && rm -f "$rc.tmp"
    changed=1
  fi
  [ "$changed" = 1 ] && echo "cleaned edge entries from $rc (backup at $rc.edgebak)"
done

# 3. Module cache. Unconditional, nothing else reads it and every entry refetches on demand.
if [ -d "$NATIVE_DIR" ]; then
  rm -rf "$NATIVE_DIR"
  echo "removed $NATIVE_DIR"
fi

# 4. Bundled chrome-headless-shell cache. Opt-in; `edge uninstall` sets EDGE_UNINSTALL_REMOVE_BROWSER after asking in Rust, so we skip the prompt then.
remove_chrome_cache() {
  if [ -d "$CHROME_DIR" ]; then
    rm -rf "$CHROME_DIR"
    echo "removed $CHROME_DIR"
  else
    echo "no chrome-headless-shell cache at $CHROME_DIR"
  fi
}

case "${EDGE_UNINSTALL_REMOVE_BROWSER:-}" in
  1) remove_chrome_cache ;;
  0) echo "leaving chrome-headless-shell cache at $CHROME_DIR" ;;
  *)
    if [ -t 0 ]; then
      printf "remove bundled chrome-headless-shell cache at %s? [y/N] " "$CHROME_DIR"
      read -r ans
      case "$ans" in
        [yY]|[yY][eE][sS]) remove_chrome_cache ;;
        *) echo "leaving chrome-headless-shell cache at $CHROME_DIR" ;;
      esac
    else
      echo "leaving chrome-headless-shell cache at $CHROME_DIR (non-interactive)"
    fi
    ;;
esac

echo "edge removed."
