#!/usr/bin/env bash
# Build the donutbrowser/Tauri client for macOS, drop artifacts in dist/mac/.
# Run on a Mac (we don't cross-compile to macOS from other OSes).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLIENT="$ROOT/client"
DIST="$ROOT/dist/mac"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-mac.sh must run on macOS (got $(uname -s)). Use build-win.sh on Windows." >&2
  exit 1
fi

# Source nvm so `pnpm` is on PATH in non-interactive shells.
export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
[[ -s "$NVM_DIR/nvm.sh" ]] && \. "$NVM_DIR/nvm.sh"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm not on PATH (try: corepack enable; nvm use)" >&2
  exit 1
fi

# Default to host arch; override with: TARGET=universal-apple-darwin ./build-mac.sh
ARCH="$(uname -m)"
case "$ARCH" in
  arm64)  DEFAULT_TARGET="aarch64-apple-darwin" ;;
  x86_64) DEFAULT_TARGET="x86_64-apple-darwin" ;;
  *) echo "unknown mac arch: $ARCH" >&2; exit 1 ;;
esac
TARGET="${TARGET:-$DEFAULT_TARGET}"

# `universal-apple-darwin` requires both rust targets installed.
if [[ "$TARGET" == "universal-apple-darwin" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
fi

echo "==> building macOS client (target=$TARGET)"
cd "$CLIENT"
pnpm tauri build --target "$TARGET" --bundles app,dmg

# Tauri puts artifacts under target/<TARGET>/release/bundle/{macos,dmg}/.
SRC_DMG="$CLIENT/src-tauri/target/$TARGET/release/bundle/dmg"
SRC_APP="$CLIENT/src-tauri/target/$TARGET/release/bundle/macos"

rm -rf "$DIST"
mkdir -p "$DIST"

if [[ -d "$SRC_DMG" ]]; then
  cp -a "$SRC_DMG"/*.dmg "$DIST/" 2>/dev/null || true
fi
if [[ -d "$SRC_APP" ]]; then
  # .app is a directory bundle; preserve perms and symlinks.
  for app in "$SRC_APP"/*.app; do
    [[ -e "$app" ]] && cp -a "$app" "$DIST/"
  done
fi

echo
echo "==> done. Artifacts in $DIST:"
ls -lh "$DIST"
