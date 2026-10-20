#!/usr/bin/env bash
#
# Stage native libraries for all RIDs you can build on this machine, then pack
# the QuicMedia NuGet package.
#
# A complete multi-RID package is normally assembled in CI: each OS/arch build
# agent runs build-native.(sh|ps1) for its RID into dotnet/QuicMedia/runtimes/,
# the runtimes/ tree is collected, and a final `dotnet pack` produces one
# package containing every platform's binary. This script does the host RID.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "== building native for host RID =="
"$ROOT/build/build-native.sh" "$@"

echo "== dotnet pack =="
dotnet pack "$ROOT/dotnet/QuicMedia/QuicMedia.csproj" -c Release -o "$ROOT/artifacts"
echo "== done: see $ROOT/artifacts =="
