<#
.SYNOPSIS
  Build the native quicmedia library and stage it under the NuGet
  runtimes/<rid>/native layout the QuicMedia package expects.

.EXAMPLE
  build/build-native.ps1 -Rid win-x64
  build/build-native.ps1 -Rid win-x64 -Features "opus-codec,cpal-backend" -Profile release
#>
param(
  [string]$Target = "",
  [string]$Rid = "win-x64",
  [string]$Features = "opus-codec,cpal-backend",
  [ValidateSet("release", "dev")] [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$RustDir = Join-Path $Root "rust"
$StageRoot = Join-Path $Root "dotnet/QuicMedia/runtimes"

$cargoArgs = @("build", "-p", "quicmedia-ffi")
if ($Profile -eq "release") { $cargoArgs += "--release" }
if ($Features) { $cargoArgs += @("--features", $Features) }
if ($Target) { $cargoArgs += @("--target", $Target) }

Write-Host ">> cargo $($cargoArgs -join ' ')  (rid=$Rid)"
Push-Location $RustDir
try { & cargo @cargoArgs } finally { Pop-Location }

$outDir = Join-Path $RustDir "target"
if ($Target) { $outDir = Join-Path $outDir $Target }
$outDir = Join-Path $outDir $Profile

switch -Wildcard ($Rid) {
  "win-*" { $src = Join-Path $outDir "quicmedia.dll" }
  "osx-*" { $src = Join-Path $outDir "libquicmedia.dylib" }
  default { $src = Join-Path $outDir "libquicmedia.so" }
}

if (-not (Test-Path $src)) {
  throw "expected native artifact not found: $src"
}

$dest = Join-Path $StageRoot "$Rid/native"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -Force $src $dest
Write-Host ">> staged $(Split-Path $src -Leaf) -> dotnet/QuicMedia/runtimes/$Rid/native/"
