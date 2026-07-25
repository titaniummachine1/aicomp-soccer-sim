#!/usr/bin/env python3
"""Shared helpers for gating Titanium candidate builds at REAL match length
(180s), not the first-to-10 marathon the pre-2026-07-25 gate used (see
docs/HANDOFF_2026-07-25.md section 0 in the private engine repo — that gate
measured the wrong objective).

Used by tournament_5candidates_180s.py and gate_round_robin.py.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Invoke via `cargo run` rather than the raw exe: the release binary
# dynamically links bevy_dylib and its DLL search fails when launched
# directly from this shell, even with the DLL copied next to the exe.
# `cargo run` resolves it correctly, at the cost of ~1-2s launch overhead
# per match (cargo's own up-to-date check).
CARGO_RUN_PREFIX = ["cargo", "run", "--release", "--bin", "soccer_headless", "--"]

SECS = "180"


def run_match(home_brain, away_brain, opening, secs=SECS):
    cmd = CARGO_RUN_PREFIX + [
        "--home", home_brain,
        "--away", away_brain,
        "--secs", secs,
        "--opening", opening,
        "--quiet",
    ]
    out = subprocess.run(cmd, capture_output=True, text=True, cwd=str(ROOT))
    if out.returncode != 0:
        print(f"MATCH FAILED: {' '.join(cmd)}\n{out.stderr}", file=sys.stderr)
        sys.exit(1)
    # cargo prints its own "Compiling/Finished/Running" lines to stdout in
    # some configs; the match JSON is always the last non-empty line.
    lines = [l for l in out.stdout.strip().splitlines() if l.strip()]
    return json.loads(lines[-1])


def decide(result):
    """Returns 'home' | 'away' | 'draw' — score first, possession tiebreak."""
    sh, sa = result["score_home"], result["score_away"]
    if sh > sa:
        return "home"
    if sa > sh:
        return "away"
    ph, pa = result["possession_s_home"], result["possession_s_away"]
    if ph > pa:
        return "home"
    if pa > ph:
        return "away"
    return "draw"


def round_robin(candidates):
    """candidates: list of (name, path). Returns (matchups, standings)."""
    from itertools import combinations

    table = {name: {"pts": 0, "gf": 0, "ga": 0, "pos_for": 0.0, "pos_against": 0.0, "w": 0, "d": 0, "l": 0} for name, _ in candidates}
    matchups = []

    for (n1, p1), (n2, p2) in combinations(candidates, 2):
        b1, b2 = f"graph:{p1}", f"graph:{p2}"

        r_home1 = run_match(b1, b2, "home")   # n1 kicks off
        r_home2 = run_match(b2, b1, "home")   # n2 kicks off (colors swapped)

        for tag, r, home_name, away_name in [
            ("g1", r_home1, n1, n2),
            ("g2", r_home2, n2, n1),
        ]:
            winner = decide(r)
            sh, sa = r["score_home"], r["score_away"]
            ph, pa = r["possession_s_home"], r["possession_s_away"]

            table[home_name]["gf"] += sh
            table[home_name]["ga"] += sa
            table[home_name]["pos_for"] += ph
            table[home_name]["pos_against"] += pa
            table[away_name]["gf"] += sa
            table[away_name]["ga"] += sh
            table[away_name]["pos_for"] += pa
            table[away_name]["pos_against"] += ph

            if winner == "home":
                table[home_name]["pts"] += 3
                table[home_name]["w"] += 1
                table[away_name]["l"] += 1
                result_str = f"{home_name} beats {away_name}"
            elif winner == "away":
                table[away_name]["pts"] += 3
                table[away_name]["w"] += 1
                table[home_name]["l"] += 1
                result_str = f"{away_name} beats {home_name}"
            else:
                table[home_name]["pts"] += 1
                table[away_name]["pts"] += 1
                table[home_name]["d"] += 1
                table[away_name]["d"] += 1
                result_str = f"{home_name} draws {away_name}"

            print(
                f"[{tag}] {home_name:26s} {sh:2d} - {sa:<2d} {away_name:26s} "
                f"(poss {ph:5.1f}s / {pa:5.1f}s)  -> {result_str}",
                file=sys.stderr,
            )
            matchups.append({
                "home": home_name, "away": away_name,
                "score_home": sh, "score_away": sa,
                "possession_home": ph, "possession_away": pa,
                "winner": winner,
            })

    print("\n== Standings (3 pts win / 1 pt draw, score first then possession) ==\n", file=sys.stderr)
    ranked = sorted(table.items(), key=lambda kv: (-kv[1]["pts"], -(kv[1]["gf"] - kv[1]["ga"]), -kv[1]["pos_for"]))
    header = f"{'rank':4s} {'candidate':26s} {'pts':4s} {'W':3s} {'D':3s} {'L':3s} {'GF':4s} {'GA':4s} {'GD':5s} {'poss_for_s':10s}"
    print(header, file=sys.stderr)
    for i, (name, s) in enumerate(ranked, 1):
        gd = s["gf"] - s["ga"]
        print(
            f"{i:<4d} {name:26s} {s['pts']:<4d} {s['w']:<3d} {s['d']:<3d} {s['l']:<3d} "
            f"{s['gf']:<4d} {s['ga']:<4d} {gd:<+5d} {s['pos_for']:<10.1f}",
            file=sys.stderr,
        )

    return matchups, ranked
