#!/usr/bin/env python3
"""Head-to-head: opening-dump challenger vs live Titanium_v3."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
V3 = ROOT / "data" / "titanium" / "Titanium_v3.txt"
CH = ROOT.parent / "titanim-socker-engine" / "out" / "Titanium_challenger.txt"


def main() -> int:
    if not V3.is_file():
        print(f"missing {V3}", file=sys.stderr)
        return 1
    if not CH.is_file():
        print(f"missing {CH}", file=sys.stderr)
        return 1

    b3 = f"graph:{V3.resolve()}"
    bc = f"graph:{CH.resolve()}"

    print("== Challenger (opening GD dump) vs Titanium_v3 (180s, both openings) ==\n")
    r1 = gate_lib.run_match(bc, b3, "home")
    print(
        f"  challenger {r1['score_home']}-{r1['score_away']} v3"
        f"   (challenger kickoff)"
    )
    r2 = gate_lib.run_match(b3, bc, "home")
    print(
        f"  v3          {r2['score_home']}-{r2['score_away']} challenger"
        f"   (v3 kickoff)"
    )

    ch_gf = r1["score_home"] + r2["score_away"]
    v3_gf = r1["score_away"] + r2["score_home"]
    print("\n== Totals ==")
    print(f"  challenger  {ch_gf} scored / {v3_gf} conceded  (GD {ch_gf - v3_gf:+d})")
    print(f"  Titanium_v3 {v3_gf} scored / {ch_gf} conceded  (GD {v3_gf - ch_gf:+d})")
    if ch_gf > v3_gf:
        print("  Verdict: challenger STRONGER than v3")
    elif v3_gf > ch_gf:
        print("  Verdict: v3 still stronger")
    else:
        print("  Verdict: TIE")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
