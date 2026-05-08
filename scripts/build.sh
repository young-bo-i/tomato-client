#!/usr/bin/env bash
# Top-level build dispatcher.
#
#   ./scripts/build.sh              # auto: builds for the current host OS
#   ./scripts/build.sh mac          # force mac build
#   ./scripts/build.sh win          # force win build (cross-compile if not on Windows)
#   ./scripts/build.sh all          # try both (win step needs cargo-xwin if not on Windows)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${1:-auto}"

run_mac() { bash "$SCRIPT_DIR/build-mac.sh"; }
run_win() { bash "$SCRIPT_DIR/build-win.sh"; }

case "$TARGET" in
  mac) run_mac ;;
  win) run_win ;;
  all) run_mac && run_win ;;
  auto)
    case "$(uname -s)" in
      Darwin)               run_mac ;;
      Linux)                run_win ;;       # cross-compile via xwin
      MINGW*|MSYS*|CYGWIN*) run_win ;;       # native on Windows
      *) echo "unknown OS: $(uname -s); pass 'mac' or 'win' explicitly" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "usage: $0 [mac|win|all|auto]" >&2
    exit 1
    ;;
esac
