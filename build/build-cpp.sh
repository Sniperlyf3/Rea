#!/usr/bin/env bash
#
# Build the native library and the C++ example/package via CMake.
#
# Usage: build/build-cpp.sh [--features <csv>] [--profile release|dev]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FEATURES="opus-codec,miniaudio-backend"
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --features) FEATURES="$2"; shift 2 ;;
    --profile)  PROFILE="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

echo "== building native quicmedia-ffi =="
CARGO_ARGS=(build -p quicmedia-ffi)
[[ "$PROFILE" == "release" ]] && CARGO_ARGS+=(--release)
[[ -n "$FEATURES" ]] && CARGO_ARGS+=(--features "$FEATURES")
( cd "$ROOT/rust" && cargo "${CARGO_ARGS[@]}" )

LIB_DIR="$ROOT/rust/target/$PROFILE"

echo "== configuring + building C++ (CMake) =="
cmake -S "$ROOT/cpp" -B "$ROOT/cpp/build" -DQUICMEDIA_LIB_DIR="$LIB_DIR" >/dev/null
cmake --build "$ROOT/cpp/build"
echo "== done: $ROOT/cpp/build/quicmedia_demo =="
