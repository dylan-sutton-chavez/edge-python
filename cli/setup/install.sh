#!/usr/bin/env bash

set -e

BASE="${EDGE_INSTALL_BASE:-https://cdn.edgepython.com/cli}"
INSTALL_DIR="${EDGE_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
esac

target="${arch}-${os}"

echo "downloading edge (${target})..."
mkdir -p "$INSTALL_DIR"
curl -fsSL "${BASE}/edge-${target}.tar.gz" | tar -xz -C "$INSTALL_DIR" edge
chmod +x "$INSTALL_DIR/edge"
echo "installed $INSTALL_DIR/edge"

case "$(basename "${SHELL:-bash}")" in
  bash) rc="$HOME/.bashrc" ;;
  zsh) rc="$HOME/.zshrc" ;;
  *) rc="" ;;
esac

# Check the rc file, not the live PATH, an old shell keeps the dir after an uninstall.
rc_has_path() {
  grep -qs "$INSTALL_DIR" "$rc" 2>/dev/null && return 0
  # The default dir is often already on PATH via a $HOME-spelled rc/profile line.
  [ "$INSTALL_DIR" = "$HOME/.local/bin" ] && grep -qs '\$HOME/.local/bin' "$rc" 2>/dev/null
}

rc_changed=""
if [ -n "$rc" ] && ! rc_has_path; then
  printf '\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$rc"
  echo "added $INSTALL_DIR to PATH in $rc"
  rc_changed=1
fi

"$INSTALL_DIR/edge" --version

if [ -n "$rc_changed" ]; then
  echo "open a new terminal to pick up the new environment (or run 'exec \$SHELL')"
fi
