#!/usr/bin/env python3
"""Round-robin strength tournament for the 5 candidate Titanium.txt graphs
dropped in Downloads on 2026-07-25, at REAL match length (180s), not the
first-to-10 marathon the earlier gate used (see docs/HANDOFF_2026-07-25.md
section 0 — that gate measured the wrong objective).

Each pair plays twice (colors swapped) to cancel opening-side bias. The sim
is deterministic given (graph pair, opening side) — no extra seeds needed;
verified empirically (same opening -> byte-identical score/possession
across different --seed values).

Usage (from aicomp-soccer-sim/):
    python scripts/tournament_5candidates_180s.py
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CANDIDATES_DIR = ROOT / "data" / "titanium" / "candidates_20260725"

run_match = gate_lib.run_match

CANDIDATES = [
    ("DL0_pre_pace_delay", CANDIDATES_DIR / "DL0_pre_pace_delay.txt"),
    ("DL1_after_b4a6e74", CANDIDATES_DIR / "DL1_after_b4a6e74.txt"),
    ("DL2_with_timeplot", CANDIDATES_DIR / "DL2_with_timeplot.txt"),
    ("DL3_shot_origin_fix", CANDIDATES_DIR / "DL3_shot_origin_fix.txt"),
    ("DL4_closed_form_current", CANDIDATES_DIR / "DL4_closed_form_current.txt"),
]

REFERENCE_OPPONENT = "aia"  # external fixed reference (aia3 graph), not part of round robin


def main():
    for name, path in CANDIDATES:
        if not path.is_file():
            print(f"Missing candidate file: {path}", file=sys.stderr)
            sys.exit(1)

    print(f"== Round robin: {len(CANDIDATES)} candidates, 180s matches, colors swapped ==\n", file=sys.stderr)
    matchups, ranked = gate_lib.round_robin(CANDIDATES)

    print(f"\n== Absolute reference: each candidate vs {REFERENCE_OPPONENT} (external, fixed), 180s, colors swapped ==\n", file=sys.stderr)
    ref_results = []
    for name, path in CANDIDATES:
        b = f"graph:{path}"
        r1 = run_match(b, REFERENCE_OPPONENT, "home")
        r2 = run_match(REFERENCE_OPPONENT, b, "home")
        gf = r1["score_home"] + r2["score_away"]
        ga = r1["score_away"] + r2["score_home"]
        pf = r1["possession_s_home"] + r2["possession_s_away"]
        pa = r1["possession_s_away"] + r2["possession_s_home"]
        print(
            f"{name:26s} vs {REFERENCE_OPPONENT}: GF {gf:2d}  GA {ga:2d}  GD {gf - ga:+3d}  "
            f"poss_for {pf:5.1f}s / poss_against {pa:5.1f}s",
            file=sys.stderr,
        )
        ref_results.append({"candidate": name, "gf": gf, "ga": ga, "poss_for": pf, "poss_against": pa})

    out = {"matchups": matchups, "standings": ranked, "vs_reference": ref_results}
    print(json.dumps(out))


if __name__ == "__main__":
    main()
