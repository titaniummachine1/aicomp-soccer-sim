"""Score a candidate on TOTAL GOALS SCORED across the whole roster, both sides.

Goals conceded are deliberately not part of the ranking — the promotion gate is
attacking output, so a build that scores more wins even if it also leaks more.
"""
import json
import subprocess
import sys
from pathlib import Path

SIM = Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim")
SAVES = Path.home() / "AppData" / "LocalLow" / "Unicorn One" / "AIComp" / "Saves" / "Soccer"
CAND = sys.argv[1]
SECS = sys.argv[2] if len(sys.argv) > 2 else "180"

roster = sorted(
    p for p in SAVES.glob("*.txt")
    if p.stem not in {"Titanium", "Titanium_test", "Titanium_challenger",
                      "MatchProbe", "Blank", "ClockProbe", "FunctionNestingProbe",
                      "OperationProbe", "StartupProbe", "Test1", "Test2",
                      "TackleEmptyStam", "TackleEmptyStamAway", "AIA_Debug",
                      "Controller", "Position", "PerfectController.bak"}
)


def run(home, away, opening):
    out = subprocess.run(
        ["cargo", "run", "--release", "--bin", "soccer_headless", "--",
         "--home", home, "--away", away, "--secs", SECS,
         "--opening", opening, "--seed", "1", "--quiet"],
        capture_output=True, text=True, cwd=str(SIM))
    lines = [l for l in out.stdout.strip().splitlines() if l.strip()]
    return json.loads(lines[-1]) if lines else None


total_for = total_against = 0
print(f"candidate: {Path(CAND).name}   {len(roster)} opponents x both sides, {SECS}s\n")
for opp in roster:
    row = []
    for side in ("home", "away"):
        h, a = (f"graph:{CAND}", f"graph:{opp}") if side == "home" else (f"graph:{opp}", f"graph:{CAND}")
        d = run(h, a, side)
        if not d:
            row.append("  ERR  ")
            continue
        gf = d["score_home"] if side == "home" else d["score_away"]
        ga = d["score_away"] if side == "home" else d["score_home"]
        total_for += gf
        total_against += ga
        row.append(f"{gf}-{ga}")
    print(f"  {opp.stem:24s} home {row[0]:>7s}   away {row[1]:>7s}")

print(f"\n  TOTAL GOALS SCORED   {total_for}")
print(f"  (conceded {total_against}, not part of the ranking)")
