#!/usr/bin/env python3
"""CANONICAL promotion test — simple total-goals gate.

Contenders (exactly two):
  * live champion (`Titanium.txt`)
  * current challenger (`Titanium_challenger.txt`)

Targets to beat (fixed roster, never promoted):
  AIA, AIA3, Poponeta, Haialand-v2, StarCheese

Each contender plays every target home AND away. No titanium-vs-titanium,
no round-robin, no AI against itself.

Whoever scores more total goals wins. Challenger promotes only if it
strictly outscores the champion.
"""
from __future__ import annotations

import os
import shutil
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

# cargo/sim is mostly waiting on one core per match; run several at once.
MATCH_WORKERS = int(os.environ.get("PROMOTE_WORKERS", "4"))

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
ENGINE_OUT = ROOT.parent / "titanim-socker-engine" / "out"
CHALLENGER = ENGINE_OUT / "Titanium_challenger.txt"
HAIALAND = ROOT / "data" / "titanium" / "Haialand-v2.txt"
DATA_TI = ROOT / "data" / "titanium"

CHAMPION_NAME = "champion"
CHALLENGER_NAME = "challenger"


def _resolve_graph(*candidates: Path) -> Path | None:
    for p in candidates:
        if p.is_file():
            return p
    return None


STARCHEESE = _resolve_graph(SAVES / "StarCheese.txt", ENGINE_OUT / "StarCheese.txt")

# Fixed targets — never contenders, never play each other.
TARGETS = [
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


def goals_against_in_match(m, bot):
    if m["home"] == bot:
        return m["score_away"]
    if m["away"] == bot:
        return m["score_home"]
    return 0


def total_goals_for(matches, bot):
    return sum(goals_in_match(m, bot) for m in matches)


def total_conceded_for(matches, bot):
    return sum(goals_against_in_match(m, bot) for m in matches)


def build_contenders() -> list[tuple[str, Path]]:
    contenders = []
    if LIVE.is_file():
        contenders.append((CHAMPION_NAME, LIVE))
    if CHALLENGER.is_file():
        contenders.append((CHALLENGER_NAME, CHALLENGER))
    return contenders


def build_fixtures(contenders: list[tuple[str, Path]]):
    """Each contender vs each target, home and away. No self-play."""
    fixtures = []
    for ti_name, ti_path in contenders:
        for opp_name, opp_path in TARGETS:
            if opp_path is None or not opp_path.is_file():
                print(f"  skip {opp_name} — missing", file=sys.stderr)
                continue
            fixtures.append((ti_name, ti_path, opp_name, opp_path, "home"))
            fixtures.append((opp_name, opp_path, ti_name, ti_path, "away"))
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
            os.utime(dest, (now, now))
        except OSError:
            pass
        print(f"  Auto-deployed TEST: {dest}")


def next_version_path() -> Path:
    """Next Titanium_vN snapshot path under data/titanium."""
    n = 1
    while (DATA_TI / f"Titanium_v{n}.txt").is_file():
        n += 1
    return DATA_TI / f"Titanium_v{n}.txt"


def promote_challenger() -> None:
    BACKUPS.mkdir(parents=True, exist_ok=True)
    DATA_TI.mkdir(parents=True, exist_ok=True)
    if LIVE.is_file():
        n = 0
        backup = BACKUPS / "Titanium_pre_promote.txt"
        while backup.is_file() and backup.read_bytes() != LIVE.read_bytes():
            n += 1
            backup = BACKUPS / f"Titanium_pre_promote_{n}.txt"
        if not backup.is_file():
            shutil.copy2(LIVE, backup)
            print(f"  backed up incumbent -> {backup}")

    ver = next_version_path()
    shutil.copy2(CHALLENGER, ver)
    shutil.copy2(CHALLENGER, SAVES / ver.name)
    shutil.copy2(CHALLENGER, ENGINE_OUT / ver.name)
    print(f"  accepted snapshot -> {ver.name}")

    shutil.copy2(CHALLENGER, LIVE)
    unity = SAVES / "Titanium.txt"
    unity.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(CHALLENGER, unity)
    print(f"  PROMOTED challenger -> {LIVE}")
    print(f"  PROMOTED challenger -> {unity}")


def print_per_opponent_table(matches, bot):
    """GF/GA for `bot` against every target."""
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
    contenders = build_contenders()
    names = [n for n, _ in contenders]
    if CHALLENGER_NAME not in names:
        print(f"Missing challenger: {CHALLENGER}", file=sys.stderr)
        return 1
    if CHAMPION_NAME not in names:
        print(f"Missing live champion: {LIVE}", file=sys.stderr)
        return 1

    print("== Auto-deploy Titanium_test (challenger under test) ==")
    deploy_titanium_test(CHALLENGER)
    print()

    fixtures = build_fixtures(contenders)
    target_names = [n for n, p in TARGETS if p is not None and p.is_file()]
    print("== Promotion test (180s, both sides, total goals) ==\n")
    print(f"  Contenders: {', '.join(names)}")
    print(f"  Targets:    {' -> '.join(target_names)}")
    print("  Rule: most goals vs targets wins. No self-play / no round-robin.\n")
    print(f"  Parallel matches: {MATCH_WORKERS} workers\n")

    def _one(idx_fix):
        idx, (home_name, home_path, away_name, away_path, opening) = idx_fix
        m = run(home_name, away_name, home_path, away_path, opening)
        return idx, m

    matches = [None] * len(fixtures)
    with ThreadPoolExecutor(max_workers=MATCH_WORKERS) as pool:
        futs = [pool.submit(_one, item) for item in enumerate(fixtures)]
        for fut in as_completed(futs):
            idx, m = fut.result()
            matches[idx] = m
            print(
                f"  {m['home']:12s} {m['score_home']}-{m['score_away']} {m['away']:12s}  "
                f"(opening={m['opening']})"
            )

    for n in names:
        print_per_opponent_table(matches, n)

    totals = {n: total_goals_for(matches, n) for n in names}
    conceded = {n: total_conceded_for(matches, n) for n in names}
    print("\n== Contender totals (vs targets only) ==\n")
    for n in sorted(names, key=lambda x: -totals[x]):
        print(
            f"  {n:12s}  {totals[n]} scored  /  {conceded[n]} conceded  "
            f"(GD {totals[n] - conceded[n]:+d})"
        )

    tg = totals[CHALLENGER_NAME]
    cg = totals[CHAMPION_NAME]
    print("\n== Verdict ==")
    if tg > cg:
        print(f"  Challenger wins gate ({tg} vs champion {cg}). Promoting.")
        promote_challenger()
        deploy_titanium_test(CHALLENGER)
    else:
        print(f"  No promotion — challenger {tg}, champion {cg}.")
        deploy_titanium_test(CHALLENGER)
    return 0


def main():
    return run_promotion_pipeline()


if __name__ == "__main__":
    sys.exit(main())
