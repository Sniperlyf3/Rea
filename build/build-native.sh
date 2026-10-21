#!/usr/bin/env bash
#
# Build the native quicmedia library and stage it under the NuGet
# runtimes/<rid>/native layout the QuicMedia package expects.
#
# Usage:
#   build/build-native.sh [--target <rust-target>] [--rid <dotnet-rid>]
#                         [--features <csv>] [--profile release|dev]
#
# Defaults: build for the host, RID auto-detected, features "opus-codec,miniaudio-backend".
#
# Cross targets (Android/iOS) require the relevant Rust target + toolchain
# (e.g. cargo-ndk + Android NDK, or the ios targets on macOS). See docs/ARCHITECTURE.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$ROOT/rust"
STAGE_ROOT="$ROOT/dotnet/QuicMedia/runtimes"

TARGET=""
RID=""
FEATURES="opus-codec,miniaudio-backend"
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)   TARGET="$2"; shift 2 ;;
    --rid)      RID="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    --profile)  PROFILE="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# Auto-detect host RID / target if not provided.
host_os="$(uname -s)"
host_arch="$(uname -m)"
if [[ -z "$RID" ]]; then
  case "$host_os" in
    Linux)  os="linux" ;;
    Darwin) os="osx" ;;
    *)      os="unknown" ;;
  esac
  case "$host_arch" in
    x86_64)        arch="x64" ;;
    aarch64|arm64) arch="arm64" ;;
    *)             arch="$host_arch" ;;
  esac
  RID="${os}-${arch}"
fi

CARGO_ARGS=(build -p quicmedia-ffi)
[[ "$PROFILE" == "release" ]] && CARGO_ARGS+=(--release)
[[ -n "$FEATURES" ]] && CARGO_ARGS+=(--features "$FEATURES")
[[ -n "$TARGET" ]] && CARGO_ARGS+=(--target "$TARGET")

echo ">> cargo ${CARGO_ARGS[*]}  (rid=$RID)"
( cd "$RUST_DIR" && cargo "${CARGO_ARGS[@]}" )

# Locate the produced shared library.
OUT_DIR="$RUST_DIR/target"
[[ -n "$TARGET" ]] && OUT_DIR="$OUT_DIR/$TARGET"
OUT_DIR="$OUT_DIR/$PROFILE"

case "$RID" in
  win-*)   src="$OUT_DIR/quicmedia.dll" ;;
  osx-*)   src="$OUT_DIR/libquicmedia.dylib" ;;
  *)       src="$OUT_DIR/libquicmedia.so" ;;
esac

if [[ ! -f "$src" ]]; then
  echo "error: expected native artifact not found: $src" >&2
  echo "       (built files:)" >&2
  ls -1 "$OUT_DIR" | grep -i quicmedia >&2 || true
  exit 1
fi

DEST="$STAGE_ROOT/$RID/native"
mkdir -p "$DEST"
cp -f "$src" "$DEST/"
echo ">> staged $(basename "$src") -> dotnet/QuicMedia/runtimes/$RID/native/"
