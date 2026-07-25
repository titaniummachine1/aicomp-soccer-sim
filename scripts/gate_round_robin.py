#!/usr/bin/env python3
"""Generic round-robin gate for Titanium candidate builds, at REAL match
length (180s) — see gate_lib.py / docs/HANDOFF_2026-07-25.md section 0.

This is the "does the new one actually beat what's live" check that must
pass before `build_titanium.py --promote` is run. It never touches live
files itself — read-only comparison.

Usage:
    # candidate vs whatever's currently live (accepted champion)
    python scripts/gate_round_robin.py candidate=path/to/new.txt

    # explicit round robin among any number of named builds
    python scripts/gate_round_robin.py nameA=pathA.txt nameB=pathB.txt ...
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
LIVE_CHAMPION = ROOT / "data" / "titanium" / "Titanium.txt"


def parse_candidates(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        sys.exit(1)

    candidates = []
    for arg in argv:
        if "=" not in arg:
            print(f"Bad argument (want name=path): {arg}", file=sys.stderr)
            sys.exit(1)
        name, path_str = arg.split("=", 1)
        candidates.append((name, Path(path_str)))

    if len(candidates) == 1:
        if not LIVE_CHAMPION.is_file():
            print(f"No live champion at {LIVE_CHAMPION} to compare against; pass a second name=path.", file=sys.stderr)
            sys.exit(1)
        candidates.insert(0, ("live_champion", LIVE_CHAMPION))

    for name, path in candidates:
        if not path.is_file():
            print(f"Missing file for '{name}': {path}", file=sys.stderr)
            sys.exit(1)

    return candidates


def main():
    candidates = parse_candidates(sys.argv[1:])
    names = ", ".join(n for n, _ in candidates)
    print(f"== Gate: {names} — 180s matches, colors swapped ==\n", file=sys.stderr)
    matchups, ranked = gate_lib.round_robin(candidates)

    winner = ranked[0][0]
    print(f"\nStrongest: {winner}", file=sys.stderr)
    if len(candidates) == 2 and candidates[0][0] == "live_champion":
        candidate_name = candidates[1][0]
        if winner == candidate_name:
            print(f"-> '{candidate_name}' beat the live champion. Safe to promote.", file=sys.stderr)
        else:
            print(f"-> '{candidate_name}' did NOT beat the live champion. Do not promote.", file=sys.stderr)


if __name__ == "__main__":
    main()
