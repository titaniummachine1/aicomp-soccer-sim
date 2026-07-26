"""Score a candidate on TOTAL GOALS SCORED across every real opponent.

Four fixtures per opponent — both sides of the pitch x both kickoffs:

    candidate LEFT,  candidate kicks off      candidate LEFT,  opponent kicks off
    candidate RIGHT, candidate kicks off      candidate RIGHT, opponent kicks off

Side and kickoff are independent advantages, so testing only the two "matched"
combinations leaves half the space unmeasured and correlates the two variables.

Goals conceded are reported but NOT ranked: the gate is attacking output, so a
build that scores more wins even if it also leaks more.

Mercy handling — the rule fights a goal-count metric, so:
  (default)  --no-mercy is passed: every match runs full time, and a runaway
             that would finish 12-0 records 12 rather than being cut off at 7.
  --mercy    keep the early stop but DOUBLE that match's goals, since a match is
             stopped precisely for being one-sided and scoring it at face value
             would punish the most dominant result for running out of clock.

    python scripts/gate_total_goals.py <candidate.txt> [secs] [--mercy]
"""
import json
import subprocess
import sys
from pathlib import Path

SIM = Path(__file__).resolve().parent.parent
SAVES = Path.home() / "AppData" / "LocalLow" / "Unicorn One" / "AIComp" / "Saves" / "Soccer"

# Opponents that ACTUALLY PLAY, established by playing every save against Blank
# (a graph that never moves and never presses Interact) and keeping only those
# that could take the ball off it. Measured 2026-07-26:
#
#   AIA 3 goals/34%   AIA3 3/23%   Haialand-v2 3/29%
#   Poponeta 3/23%    Controller 3/29%   cool_dudes 0 goals/45% possession
#
# Deliberately excluded, with reasons:
#   ZudanFC3, PerfectController, PerfectController.bak — 0 goals, 0% possession
#       against a graph that does nothing, i.e. inert. They were in an earlier
#       roster handing out free 7-0 wins that measured nothing at all. ZudanFC3
#       looks like a real entrant, so its being inert is worth investigating
#       rather than assumed.
#   Test1/Test2, TackleEmptyStam*, *Probe, Position, _trajectory_service_frag —
#       probes and fragments, not opponents.
#   AIA_Debug — identical behaviour to AIA (3 goals, 33.6% possession); keeping
#       both would double AIA's weight in the total.
#   Titanium* — our own builds; a self-match is 0-0 and tells us nothing.
OPPONENTS = ["AIA", "AIA3", "Haialand-v2", "Poponeta", "Controller", "cool_dudes"]


def run(home, away, opening, secs, mercy):
    cmd = ["cargo", "run", "--release", "--bin", "soccer_headless", "--",
           "--home", home, "--away", away, "--secs", secs,
           "--opening", opening, "--quiet"]
    if not mercy:
        cmd.append("--no-mercy")
    out = subprocess.run(cmd, capture_output=True, text=True, cwd=str(SIM))
    lines = [l for l in out.stdout.strip().splitlines() if l.strip()]
    return json.loads(lines[-1]) if lines else None


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2
    candidate = Path(args[0]).resolve()
    secs = args[1] if len(args) > 1 else "180"
    mercy = "--mercy" in sys.argv

    roster = [SAVES / f"{n}.txt" for n in OPPONENTS]
    missing = [p.stem for p in roster if not p.is_file()]
    roster = [p for p in roster if p.is_file()]
    if missing:
        print(f"  MISSING, not played: {', '.join(missing)}")

    mode = "mercy on (blowouts doubled)" if mercy else "no mercy (full time)"
    print(f"candidate: {candidate.name}")
    print(f"{len(roster)} opponents x 4 fixtures, {secs}s each, {mode}\n")
    print(f"  {'opponent':<16} {'L/own':>8} {'L/opp':>8} {'R/own':>8} {'R/opp':>8}   scored")

    grand_for = grand_against = 0
    for opp in roster:
        cells, opp_for, opp_against = [], 0, 0
        # side = which end the candidate plays; kicker = who takes the kickoff.
        for side in ("home", "away"):
            for kicker in ("own", "opp"):
                if side == "home":
                    home, away = f"graph:{candidate}", f"graph:{opp}"
                    opening = "home" if kicker == "own" else "away"
                else:
                    home, away = f"graph:{opp}", f"graph:{candidate}"
                    opening = "away" if kicker == "own" else "home"
                d = run(home, away, opening, secs, mercy)
                if not d:
                    cells.append("ERR")
                    continue
                gf = d["score_home"] if side == "home" else d["score_away"]
                ga = d["score_away"] if side == "home" else d["score_home"]
                scored = gf * 2 if d.get("mercy_stopped") else gf
                opp_for += scored
                opp_against += ga
                cells.append(f"{gf}-{ga}" + ("x2" if d.get("mercy_stopped") else ""))
        grand_for += opp_for
        grand_against += opp_against
        print(f"  {opp.stem:<16} " + " ".join(f"{c:>8}" for c in cells) + f"   {opp_for:>6}")

    print(f"\n  TOTAL GOALS SCORED   {grand_for}")
    print(f"  (conceded {grand_against}, reported only — not part of the ranking)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
