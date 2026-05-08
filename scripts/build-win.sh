#!/usr/bin/env bash
# Build the donutbrowser/Tauri client for Windows, drop artifacts in dist/win/.
#
# Two paths supported:
#   1. Native build on Windows (run this from Git Bash / WSL on a Win box).
#   2. Cross-compile from macOS/Linux via cargo-xwin (experimental, requires
#      `cargo install cargo-xwin` and `rustup target add x86_64-pc-windows-msvc`).
#      Tauri's signing/bundling on a non-Windows host is best-effort — the
#      .exe / .msi installer is produced but code-signing won't work.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLIENT="$ROOT/client"
DIST="$ROOT/dist/win"

# Source nvm so `pnpm` is on PATH (Mac/Linux).
export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
[[ -s "$NVM_DIR/nvm.sh" ]] && \. "$NVM_DIR/nvm.sh"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm not on PATH (try: corepack enable; nvm use)" >&2
  exit 1
fi

TARGET="${TARGET:-x86_64-pc-windows-msvc}"
HOST_OS="$(uname -s)"

# When not on Windows, route through cargo-xwin for cross-compile.
EXTRA_ARGS=()
if [[ "$HOST_OS" != *NT* && "$HOST_OS" != "MINGW"* && "$HOST_OS" != "MSYS"* ]]; then
  echo "==> cross-compiling Windows from $HOST_OS via cargo-xwin"
  if ! command -v cargo-xwin >/dev/null 2>&1; then
    echo "cargo-xwin not installed. Run: cargo install --locked cargo-xwin" >&2
    exit 1
  fi
  rustup target add "$TARGET" >/dev/null
  EXTRA_ARGS+=(--runner cargo-xwin)
fi

echo "==> building Windows client (target=$TARGET)"
cd "$CLIENT"
pnpm tauri build --target "$TARGET" --bundles nsis,msi "${EXTRA_ARGS[@]}"

SRC_NSIS="$CLIENT/src-tauri/target/$TARGET/release/bundle/nsis"
SRC_MSI="$CLIENT/src-tauri/target/$TARGET/release/bundle/msi"

rm -rf "$DIST"
mkdir -p "$DIST"

[[ -d "$SRC_NSIS" ]] && cp -a "$SRC_NSIS"/*.exe "$DIST/" 2>/dev/null || true
[[ -d "$SRC_MSI"  ]] && cp -a "$SRC_MSI"/*.msi  "$DIST/" 2>/dev/null || true

echo
echo "==> done. Artifacts in $DIST:"
ls -lh "$DIST"
