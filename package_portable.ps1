# Package portable (static) soccer sim release.
# Run after: cargo build --release --bin soccer_sim --bin timeplot_until_goal
# NEVER use --features fast_link for shared builds.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$rel = Join-Path $root "target\release"
$pkg = Join-Path $root "dist\soccer_sim_portable"

function Test-HasDynRustStd([string]$exePath) {
  $bytes = [IO.File]::ReadAllBytes($exePath)
  $ascii = [Text.Encoding]::ASCII.GetString($bytes)
  return [bool]([regex]::Match($ascii, 'std-[a-f0-9]{8,}\.dll', 'IgnoreCase').Success -or
                [regex]::Match($ascii, 'bevy_dylib', 'IgnoreCase').Success)
}

New-Item -ItemType Directory -Force -Path $pkg | Out-Null
$soccer = Join-Path $rel "soccer_sim.exe"
$tp = Join-Path $rel "timeplot_until_goal.exe"
if (-not (Test-Path $soccer)) { throw "missing $soccer - build release first" }
if (Test-HasDynRustStd $soccer) {
  throw "$soccer still needs std-*.dll. Rebuild WITHOUT --features fast_link"
}
Copy-Item -Force $soccer (Join-Path $pkg "soccer_sim.exe")
if (Test-Path $tp) {
  if (Test-HasDynRustStd $tp) {
    throw "$tp still needs std-*.dll. Rebuild WITHOUT --features fast_link"
  }
  Copy-Item -Force $tp (Join-Path $pkg "timeplot_until_goal.exe")
}
Copy-Item -Force (Join-Path $root "bevy_sim_params_v05.json") $pkg
if (Test-Path (Join-Path $root "assets")) {
  Copy-Item -Recurse -Force (Join-Path $root "assets") (Join-Path $pkg "assets")
}

$readme = @"
AIComp Soccer Sim - portable pack
=================================
Self-contained. No Rust DLLs required.

Run: soccer_sim.exe
Keep bevy_sim_params_v05.json and assets/ next to the exe.

Built WITHOUT Bevy/Rust dynamic linking.
If Windows complains about std-*.dll you have a bad (fast_link) build.
Rebuild with: cargo build --release --bin soccer_sim
(do not pass --features fast_link)
"@
Set-Content (Join-Path $pkg "README.txt") -Value $readme -Encoding UTF8

Write-Host "Packaged -> $pkg"
Get-ChildItem $pkg | ForEach-Object { "{0}`t{1:N0}" -f $_.Name, $_.Length }
