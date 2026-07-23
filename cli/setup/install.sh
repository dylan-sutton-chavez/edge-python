#!/usr/bin/env bash
# Re-run this script any time to upgrade.

set -e

BASE="${EDGE_INSTALL_BASE:-https://cdn.edgepython.com/cli}"
INSTALL_DIR="${EDGE_INSTALL_DIR:-$HOME/.local/bin}"

# Bundle a pinned chrome-headless-shell in ~/.cache/edge (same as Puppeteer approach); Chromium isn't in AL2/AL2023/RHEL repos and ID_LIKE distro detection is unreliable.
CHROME_DIR="${EDGE_CHROME_DIR:-$HOME/.cache/edge}"
CHROME_BUILD="${EDGE_CHROME_BUILD:-131.0.6778.85}"

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

case "$target" in
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl|x86_64-apple-darwin|aarch64-apple-darwin) ;;
  *) echo "no prebuilt for $target yet; build from source with 'cargo install --path cli'" >&2; exit 1 ;;
esac

# Map our target to the chrome-for-testing platform folder name.
case "$target" in
  x86_64-unknown-linux-musl) chrome_platform="linux64" ;;
  aarch64-unknown-linux-musl) chrome_platform="" ;; # no headless-shell build for linux-arm64
  x86_64-apple-darwin) chrome_platform="mac-x64" ;;
  aarch64-apple-darwin) chrome_platform="mac-arm64" ;;
esac

CHROME_BIN="$CHROME_DIR/chrome-headless-shell-${chrome_platform}/chrome-headless-shell"

# True if a Chrome/Chromium-flavored binary the engine accepts is already reachable.
have_browser() {
  [ -n "${EDGE_CHROME_PATH:-}" ] && [ -x "${EDGE_CHROME_PATH}" ] && return 0
  [ -n "$chrome_platform" ] && [ -x "$CHROME_BIN" ] && return 0
  command -v chromium >/dev/null 2>&1 \
    || command -v chromium-browser >/dev/null 2>&1 \
    || command -v google-chrome >/dev/null 2>&1 \
    || command -v microsoft-edge >/dev/null 2>&1
}

# Download a pinned chrome-headless-shell zip from Google's chrome-for-testing CDN.
install_browser() {
  if [ -z "$chrome_platform" ]; then
    echo "no chrome-headless-shell build for $target; install Chrome/Chromium manually and set EDGE_CHROME_PATH" >&2
    exit 1
  fi

  if ! command -v unzip >/dev/null 2>&1; then
    echo "unzip is required to extract chrome-headless-shell; install it (apt/dnf/yum/pacman/apk/brew install unzip) and re-run" >&2
    exit 1
  fi

  echo "no Chromium-flavored browser found; downloading chrome-headless-shell ${CHROME_BUILD} (${chrome_platform})..."
  local url="https://storage.googleapis.com/chrome-for-testing-public/${CHROME_BUILD}/${chrome_platform}/chrome-headless-shell-${chrome_platform}.zip"
  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/edge-chs.XXXXXX.zip")"
  mkdir -p "$CHROME_DIR"
  curl -fsSL "$url" -o "$tmp"
  unzip -q -o "$tmp" -d "$CHROME_DIR"
  rm -f "$tmp"
  chmod +x "$CHROME_BIN"
  echo "installed: $CHROME_BIN"
}

echo "downloading edge (${target})..."
mkdir -p "$INSTALL_DIR"
curl -fsSL "${BASE}/edge-${target}.tar.gz" | tar -xz -C "$INSTALL_DIR" edge
chmod +x "$INSTALL_DIR/edge"
echo "installed $INSTALL_DIR/edge"

if have_browser; then
  echo "browser: found an existing Chrome/Chromium, skipping chrome-headless-shell download"
else
  install_browser
fi

case "$(basename "${SHELL:-bash}")" in
  bash) rc="$HOME/.bashrc" ;;
  zsh) rc="$HOME/.zshrc" ;;
  *) rc="" ;;
esac

# Persist EDGE_CHROME_PATH so the CLI finds the bundled headless shell across shells.
if [ -n "$rc" ] && [ -n "$chrome_platform" ] && [ -x "$CHROME_BIN" ] && ! grep -qs 'EDGE_CHROME_PATH=' "$rc" 2>/dev/null; then
  printf '\nexport EDGE_CHROME_PATH="%s"\n' "$CHROME_BIN" >> "$rc"
  echo "added EDGE_CHROME_PATH to $rc"
fi

# Check the rc file, not the live $PATH: after an uninstall the old shell still has the dir in $PATH, but new shells would not.
rc_has_path() {
  grep -qs "$INSTALL_DIR" "$rc" 2>/dev/null && return 0
  # The default dir is often already on PATH via a $HOME-spelled rc/profile line.
  [ "$INSTALL_DIR" = "$HOME/.local/bin" ] && grep -qs '\$HOME/.local/bin' "$rc" 2>/dev/null
}

if [ -n "$rc" ] && ! rc_has_path; then
  printf '\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$rc"
  echo "added $INSTALL_DIR to PATH in $rc; run 'source $rc' or open a new shell"
fi

"$INSTALL_DIR/edge" --version
