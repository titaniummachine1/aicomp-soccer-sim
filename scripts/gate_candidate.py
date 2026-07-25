#!/usr/bin/env python3
"""Gate a candidate against the roster, hardest opponent first, stopping at
the first failure.

Why not `gate_round_robin.py`: that plays every pairing before it reports, so
a candidate that collapses against the hardest opponent still costs a full
round robin to find out, and the failure is then averaged into a standings
table that can still rank it first. It ranked one 15 pts — five wins and a
single 0-7 — while the champion it "beat" had won BOTH sides of that same
fixture. Points hide a side-specific collapse; this does not.

Two rules, both from how the real thing actually breaks:

  * Hardest first. Poponeta is the hard one, then the champion head-to-head,
    then AIA3. A candidate that cannot clear the top of the list is not worth
    the remaining matches.
  * Both sides, every opponent, and BOTH must be won. Faceoff spots are world
    absolute, so a sign error in a team multiplier produces exactly one broken
    side — a candidate can win 7-0 at home and lose 0-7 away with nothing in
    the aggregate to show for it. An opponent is only "beaten" if both colours
    are beaten.

Usage:
    python scripts/gate_candidate.py path/to/candidate.txt
    python scripts/gate_candidate.py path/to/candidate.txt --keep-going
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
LIVE_CHAMPION = ROOT / "data" / "titanium" / "Titanium.txt"
SAVES = (
    Path.home()
    / "AppData"
    / "LocalLow"
    / "Unicorn One"
    / "AIComp"
    / "Saves"
    / "Soccer"
)

# Hardest first. Poponeta above the champion because it is the fixture that
# has historically exposed regressions earliest; the head-to-head sits second
# because a candidate that loses it cannot be promoted regardless.
ROSTER = [
    ("Poponeta", SAVES / "Poponeta.txt"),
    ("live_champion", LIVE_CHAMPION),
    ("AIA3", SAVES / "AIA3.txt"),
]


def play_both_sides(cand_path, opp_name, opp_path):
    """Returns (won_both, [(label, score_line, verdict)])."""
    cand = f"graph:{cand_path}"
    opp = f"graph:{opp_path}"
    rows, wins = [], 0
    # Candidate as home, then as away. `opening` is swapped with the colours so
    # neither side gets the kickoff advantage twice.
    for label, home, away, opening, cand_is_home in (
        ("candidate HOME", cand, opp, "home", True),
        ("candidate AWAY", opp, cand, "away", False),
    ):
        r = gate_lib.run_match(home, away, opening)
        sh, sa = r["score_home"], r["score_away"]
        winner = gate_lib.decide(r)
        cand_won = (winner == "home") == cand_is_home and winner != "draw"
        cs, os_ = (sh, sa) if cand_is_home else (sa, sh)
        rows.append((label, f"{cs} - {os_}", "WIN" if cand_won else "LOSS/DRAW"))
        wins += 1 if cand_won else 0
    return wins == 2, rows


def main():
    argv = [a for a in sys.argv[1:] if not a.startswith("--")]
    keep_going = "--keep-going" in sys.argv
    if not argv:
        print(__doc__, file=sys.stderr)
        return 1
    cand_path = Path(argv[0])
    if not cand_path.is_file():
        print(f"No candidate at {cand_path}", file=sys.stderr)
        return 1

    print(f"== Gating {cand_path.name} — hardest first, both sides must win ==\n")
    failures = []
    for opp_name, opp_path in ROSTER:
        if not opp_path.is_file():
            print(f"  {opp_name:15s} SKIPPED (no file at {opp_path})")
            continue
        ok, rows = play_both_sides(cand_path, opp_name, opp_path)
        for label, score, verdict in rows:
            print(f"  vs {opp_name:15s} {label:15s} {score:>8s}   {verdict}")
        if ok:
            print(f"  -> beat {opp_name} on both sides\n")
        else:
            failures.append(opp_name)
            print(f"  -> FAILED against {opp_name}\n")
            if not keep_going:
                print(
                    f"STOP: lost to {opp_name}. Remaining opponents skipped "
                    f"(--keep-going to play them anyway).\n"
                )
                break

    if failures:
        print(f"RESULT: NOT promotable — failed {', '.join(failures)}.")
        return 1
    print("RESULT: beat every opponent on both sides. Promotable.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
