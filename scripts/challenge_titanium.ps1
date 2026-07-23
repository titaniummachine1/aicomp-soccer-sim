# Challenge Titanium candidate vs champion (run from aicomp-soccer-sim)
#
# Usage:
#   .\scripts\challenge_titanium.ps1 -Games 10 -WinGoals 5
#   .\scripts\challenge_titanium.ps1 -ChampionPath ..\titanim-socker-engine -CandidatePath ..\titanim-socker-engine-challenge
#
# Stockfish rule: candidate must beat champion; double -Games when noisy (cap 500).

param(
    [int]$Games = 10,
    [int]$WinGoals = 5,
    [float]$Secs = 900,
    [string]$ChampionPath = "..\titanim-socker-engine",
    [string]$CandidatePath = ""
)

$ErrorActionPreference = "Stop"
Write-Host "challenge: games=$Games win_goals=$WinGoals"
Write-Host "  champion=$ChampionPath"
if ($CandidatePath) { Write-Host "  candidate=$CandidatePath" } else {
    Write-Host "  candidate=(same titanium brain — use two checkouts for a real challenge)"
}

# Baseline: titanium vs aia (sanity that candidate still plays)
Write-Host "`n== sanity: titanium vs aia (first-to-$WinGoals) =="
cargo run --release --bin soccer_headless -- --home titanium --away aia --win-goals $WinGoals --secs $Secs --opening home --quiet
cargo run --release --bin soccer_headless -- --home aia --away titanium --win-goals $WinGoals --secs $Secs --opening away --quiet

# Batch self-play proxy (identical code until dual-path wiring lands)
Write-Host "`n== batch titanium mirrors (seeded openings) =="
cargo run --release --bin soccer_headless -- --batch $Games --jobs 4 --home titanium --away titanium --win-goals $WinGoals --secs $Secs --seed 0 --quiet

Write-Host "`nDone. Attach JSONL scores to the challenge PR. Double -Games if within noise."
