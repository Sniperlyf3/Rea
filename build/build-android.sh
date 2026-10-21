#!/usr/bin/env bash
#
# Cross-build the native quicmedia library for Android via cargo-ndk and stage
# each ABI under the NuGet runtimes/android-<arch>/native layout that .NET for
# Android (MAUI) consumes from a NuGet package.
#
# Usage:
#   build/build-android.sh [--abis "arm64-v8a x86_64"] [--features <csv>] [--profile release|dev]
#
# minSdk/API level is left at cargo-ndk's default (21). Set the app's minSdk in
# its Gradle/MAUI manifest; cargo-ndk 4.x does not take a -p/--platform here.
#
# Requirements:
#   - Android NDK (ANDROID_NDK_HOME or ANDROID_NDK_ROOT set)
#   - cargo-ndk           (cargo install cargo-ndk)
#   - rust android targets (added automatically below via rustup)
#
# Default features are EMPTY here. Pass --features miniaudio-backend for real
# device audio on Android (miniaudio drives AAudio/OpenSL, compiled by build.rs
# with the NDK clang). Opus is opt-in and needs the Android cmake toolchain for
# libopus; see docs/ARCHITECTURE.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$ROOT/rust"
STAGE_ROOT="$ROOT/dotnet/QuicMedia/runtimes"

ABIS="arm64-v8a x86_64"
FEATURES="${FEATURES:-}"
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --abis)     ABIS="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    --profile)  PROFILE="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "error: cargo-ndk not found. Install with: cargo install cargo-ndk" >&2
  exit 1
fi
if [[ -z "${ANDROID_NDK_HOME:-}${ANDROID_NDK_ROOT:-}" ]]; then
  echo "error: set ANDROID_NDK_HOME (or ANDROID_NDK_ROOT) to your Android NDK path" >&2
  exit 1
fi

# ABI -> (rust target triple, .NET RID)
declare -A ABI_TRIPLE=(
  [arm64-v8a]=aarch64-linux-android
  [armeabi-v7a]=armv7-linux-androideabi
  [x86_64]=x86_64-linux-android
  [x86]=i686-linux-android
)
declare -A ABI_RID=(
  [arm64-v8a]=android-arm64
  [armeabi-v7a]=android-arm
  [x86_64]=android-x64
  [x86]=android-x86
)

# Ensure the rust targets are installed (best-effort).
for abi in $ABIS; do
  triple="${ABI_TRIPLE[$abi]:-}"
  [[ -z "$triple" ]] && { echo "unknown ABI: $abi" >&2; exit 2; }
  rustup target add "$triple" >/dev/null 2>&1 || true
done

# Build all requested ABIs in one cargo-ndk invocation into a jniLibs tree.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

NDK_T_ARGS=()
for abi in $ABIS; do
  NDK_T_ARGS+=(-t "$abi")
done

BUILD_ARGS=(build -p quicmedia-ffi)
[[ "$PROFILE" == "release" ]] && BUILD_ARGS+=(--release)
[[ -n "$FEATURES" ]] && BUILD_ARGS+=(--features "$FEATURES")

echo ">> cargo ndk ${NDK_T_ARGS[*]} -o $TMP/jniLibs ${BUILD_ARGS[*]}"
( cd "$RUST_DIR" && cargo ndk "${NDK_T_ARGS[@]}" -o "$TMP/jniLibs" "${BUILD_ARGS[@]}" )

# cargo-ndk lays files out as jniLibs/<abi>/libquicmedia.so; restage into runtimes/<rid>/native.
for abi in $ABIS; do
  rid="${ABI_RID[$abi]}"
  src="$TMP/jniLibs/$abi/libquicmedia.so"
  if [[ ! -f "$src" ]]; then
    echo "error: expected $src not produced" >&2
    exit 1
  fi
  dest="$STAGE_ROOT/$rid/native"
  mkdir -p "$dest"
  cp -f "$src" "$dest/"
  echo ">> staged $abi -> dotnet/QuicMedia/runtimes/$rid/native/libquicmedia.so"
done
