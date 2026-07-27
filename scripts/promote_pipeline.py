#!/usr/bin/env python3
"""CANONICAL promotion test — fixed fixtures, total goals, champion vs challenger.

Opponents (NOT promotable), in order:
AIA → AIA3 → Poponeta → Haialand-v2 → StarCheese.
Each titanium side plays every opponent home AND away, then champion vs
challenger both sides. Most total goals between the two titanium builds wins;
challenger replaces live Titanium only if it outscores the champion. Tie →
champion stays. Runs automatically after every challenger build.
"""
from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SAVES = (
    Path.home()
    / "AppData"
    / "LocalLow"
    / "Unicorn One"
    / "AIComp"
    / "Saves"
    / "Soccer"
)
LIVE = ROOT / "data" / "titanium" / "Titanium.txt"
BACKUPS = ROOT / "data" / "titanium" / "backups"
CHALLENGER = ROOT.parent / "titanim-socker-engine" / "out" / "Titanium_challenger.txt"
HAIALAND = ROOT / "data" / "titanium" / "Haialand-v2.txt"
STARCHEESE = SAVES / "StarCheese.txt"
if not STARCHEESE.is_file():
    STARCHEESE = ROOT.parent / "titanim-socker-engine" / "out" / "StarCheese.txt"

CHAMPION_NAME = "champion"
CHALLENGER_NAME = "challenger"

TITANIUM = [
    (CHAMPION_NAME, LIVE),
    (CHALLENGER_NAME, CHALLENGER),
]

# External roster — test targets only, never promoted
EXTERNAL = [
    ("AIA", SAVES / "AIA.txt"),
    ("AIA3", SAVES / "AIA3.txt"),
    ("Poponeta", SAVES / "Poponeta.txt"),
    ("Haialand-v2", HAIALAND),
    ("StarCheese", STARCHEESE),
]


def graph(path: Path) -> str:
    return f"graph:{path.resolve()}"


def run(home_name, away_name, home_path, away_path, opening):
    r = gate_lib.run_match(graph(home_path), graph(away_path), opening)
    return {
        "home": home_name,
        "away": away_name,
        "score_home": r["score_home"],
        "score_away": r["score_away"],
        "opening": opening,
    }


def goals_in_match(m, bot):
    if m["home"] == bot:
        return m["score_home"]
    if m["away"] == bot:
        return m["score_away"]
    return 0


def total_goals_for(matches, bot):
    return sum(goals_in_match(m, bot) for m in matches)


def build_fixtures():
    """Same order every run: externals (both sides) then champion vs challenger."""
    fixtures = []
    for ti_name, ti_path in TITANIUM:
        for opp_name, opp_path in EXTERNAL:
            if not opp_path.is_file():
                print(f"  skip {opp_name} — missing {opp_path}", file=sys.stderr)
                continue
            fixtures.append((ti_name, ti_path, opp_name, opp_path, "home"))
            fixtures.append((opp_name, opp_path, ti_name, ti_path, "away"))

    champ_path = LIVE
    chall_path = CHALLENGER
    fixtures.append((CHAMPION_NAME, champ_path, CHALLENGER_NAME, chall_path, "home"))
    fixtures.append((CHALLENGER_NAME, chall_path, CHAMPION_NAME, champ_path, "home"))
    return fixtures


def deploy_titanium_test(src: Path) -> None:
    """Always refresh Titanium_test so the user can watch what was just tested."""
    import time

    if not src.is_file():
        print(f"WARN: cannot auto-deploy Titanium_test — missing {src}", file=sys.stderr)
        return
    text = src.read_text(encoding="utf-8")
    dests = [
        ROOT / "data" / "titanium" / "Titanium_test.txt",
        SAVES / "Titanium_test.txt",
        SAVES / "Titanium_challenger.txt",
    ]
    now = time.time()
    for dest in dests:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(text, encoding="utf-8")
        try:
            import os

            os.utime(dest, (now, now))
        except OSError:
            pass
        print(f"  Auto-deployed TEST: {dest}")


def promote_challenger() -> None:
    BACKUPS.mkdir(parents=True, exist_ok=True)
    if LIVE.is_file():
        n = 0
        backup = BACKUPS / "Titanium_pre_promote.txt"
        while backup.is_file() and backup.read_bytes() != LIVE.read_bytes():
            n += 1
            backup = BACKUPS / f"Titanium_pre_promote_{n}.txt"
        if not backup.is_file():
            shutil.copy2(LIVE, backup)
            print(f"  backed up incumbent -> {backup}")
    shutil.copy2(CHALLENGER, LIVE)
    unity = SAVES / "Titanium.txt"
    unity.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(CHALLENGER, unity)
    print(f"  PROMOTED challenger -> {LIVE}")
    print(f"  PROMOTED challenger -> {unity}")


def goals_against_in_match(m, bot):
    if m["home"] == bot:
        return m["score_away"]
    if m["away"] == bot:
        return m["score_home"]
    return 0


def total_conceded_for(matches, bot):
    return sum(goals_against_in_match(m, bot) for m in matches)


def print_per_opponent_table(matches, bot):
    """GF/GA for `bot` against every distinct opponent in the fixture list."""
    opps = []
    seen = set()
    for m in matches:
        other = m["away"] if m["home"] == bot else m["home"] if m["away"] == bot else None
        if other is None or other in seen:
            continue
        seen.add(other)
        opps.append(other)

    print(f"\n== {bot}: scored / conceded by opponent ==\n")
    print(f"  {'opponent':14s}  {'GF':>4}  {'GA':>4}  {'GD':>4}  matches")
    tot_gf = tot_ga = 0
    for opp in opps:
        gf = ga = 0
        rows = []
        for m in matches:
            if m["home"] == bot and m["away"] == opp:
                gf += m["score_home"]
                ga += m["score_away"]
                rows.append(f"{m['score_home']}-{m['score_away']} (H)")
            elif m["away"] == bot and m["home"] == opp:
                gf += m["score_away"]
                ga += m["score_home"]
                rows.append(f"{m['score_away']}-{m['score_home']} (A)")
        tot_gf += gf
        tot_ga += ga
        print(f"  {opp:14s}  {gf:4d}  {ga:4d}  {gf - ga:4d}  {', '.join(rows)}")
    print(f"  {'TOTAL':14s}  {tot_gf:4d}  {tot_ga:4d}  {tot_gf - tot_ga:4d}")


def run_promotion_pipeline() -> int:
    for name, path in TITANIUM:
        if not path.is_file():
            print(f"Missing {name}: {path}", file=sys.stderr)
            return 1

    print("== Auto-deploy Titanium_test (challenger under test) ==")
    deploy_titanium_test(CHALLENGER)
    print()

    fixtures = build_fixtures()
    names = [CHAMPION_NAME, CHALLENGER_NAME]

    print("== Promotion test (180s, both sides, total goals) ==\n")
    print("  Order: AIA -> AIA3 -> Poponeta -> Haialand-v2 -> StarCheese -> champion vs challenger\n")
    matches = []
    for home_name, home_path, away_name, away_path, opening in fixtures:
        m = run(home_name, away_name, home_path, away_path, opening)
        matches.append(m)
        print(
            f"  {home_name:12s} {m['score_home']}-{m['score_away']} {away_name:12s}  "
            f"(opening={opening})"
        )

    for n in names:
        print_per_opponent_table(matches, n)

    totals = {n: total_goals_for(matches, n) for n in names}
    conceded = {n: total_conceded_for(matches, n) for n in names}
    print("\n== Titanium totals (all parity fixtures) ==\n")
    for n in sorted(names, key=lambda x: -totals[x]):
        print(
            f"  {n:12s}  {totals[n]} scored  /  {conceded[n]} conceded  "
            f"(GD {totals[n] - conceded[n]:+d})"
        )

    cg = totals[CHAMPION_NAME]
    tg = totals[CHALLENGER_NAME]
    print("\n== Verdict ==")
    if tg > cg:
        print(f"  Challenger wins ({tg} vs {cg}). Promoting.")
        promote_challenger()
        deploy_titanium_test(CHALLENGER)
    elif cg > tg:
        print(f"  Champion stays ({cg} vs {tg}).")
        deploy_titanium_test(CHALLENGER)
    else:
        print(f"  Tie ({cg}-{cg}). Champion stays.")
        deploy_titanium_test(CHALLENGER)
    return 0


def main():
    return run_promotion_pipeline()


if __name__ == "__main__":
    sys.exit(main())
