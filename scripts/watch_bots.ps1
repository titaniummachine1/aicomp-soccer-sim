# Watch bot matchups in the Bevy sim viewer.
#
# Examples:
#   .\scripts\watch_bots.ps1 -Home Haialand-v2 -Away Poponeta
#   .\scripts\watch_bots.ps1 -Home Titanium -Away Haialand-v2 -Opening away
#   .\scripts\watch_bots.ps1 -Home Titanium_antitackle -Away AIA3 -Fast

param(
    [Parameter(Mandatory = $true)]
    [string]$Home,
    [Parameter(Mandatory = $true)]
    [string]$Away,
    [ValidateSet("home", "away")]
    [string]$Opening = "home",
    [switch]$Fast
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Saves = Join-Path $env:USERPROFILE "AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer"

$Bots = @{
    "Titanium"            = Join-Path $Root "data\titanium\Titanium.txt"
    "Titanium_test"       = Join-Path $Root "data\titanium\Titanium_test.txt"
    "Titanium_challenger" = Join-Path $Root "..\titanim-socker-engine\out\Titanium_challenger.txt"
    "Titanium_antitackle" = Join-Path $Root "..\titanim-socker-engine\out\Titanium_challenger.txt"
    "Haialand-v2"         = Join-Path $Root "data\titanium\Haialand-v2.txt"
    "Poponeta"            = Join-Path $Saves "Poponeta.txt"
    "AIA3"                = Join-Path $Saves "AIA3.txt"
    "aia3"                = Join-Path $Saves "AIA3.txt"
    "aia"                 = "aia"
    "chase"               = "chase"
    "idle"                = "idle"
}

function Resolve-Brain([string]$Name) {
    if ($Bots.ContainsKey($Name)) {
        $p = $Bots[$Name]
        if ($p -match "\.txt$") {
            if (-not (Test-Path $p)) {
                throw "Bot file missing: $p`nRun compare_titanium.py first to install bots."
            }
            return "graph:$p"
        }
        return $p
    }
    if ($Name -like "graph:*") { return $Name }
    if (Test-Path $Name) { return "graph:$Name" }
    throw "Unknown bot '$Name'. Known: $($Bots.Keys -join ', ')"
}

$homeBrain = Resolve-Brain $Home
$awayBrain = Resolve-Brain $Away
$fastArg = if ($Fast) { @("--fast") } else { @() }

Write-Host "Viewer: HOME=$Home ($homeBrain)  AWAY=$Away ($awayBrain)  opening=$Opening"
Write-Host "  Space=pause  F=fast  R=restart same opening"
Write-Host ""

Set-Location $Root
cargo run --release -- --home $homeBrain --away $awayBrain --opening $Opening @fastArg
