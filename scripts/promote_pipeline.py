#!/usr/bin/env python3
"""CANONICAL promotion test — simple total-goals gate.

Contenders (exactly two):
  * live champion (`Titanium.txt`)
  * current challenger (`Titanium_challenger.txt`)

Targets to beat (never contenders, never play each other):
  AIA, AIA3, Poponeta, Haialand-v2, StarCheese
  + every accepted Titanium_vN snapshot (deduped by content hash)

Each contender plays every target home AND away.

Plus champion ↔ challenger in all 4 side×kickoff configs:
  1. champ home / champ kicks
  2. champ home / challenger kicks
  3. challenger home / challenger kicks
  4. challenger home / champ kicks

Whoever scores more total goals across the full slate wins.
Challenger promotes only if it strictly outscores the champion.

Scoreboard: GP / GF / GA / GD
"""
from __future__ import annotations

import hashlib
import os
import re
import shutil
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_lib  # noqa: E402

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
_VERSION_RE = re.compile(r"^Titanium_v(\d+)\.txt$", re.IGNORECASE)


def _resolve_graph(*candidates: Path) -> Path | None:
    for p in candidates:
        if p.is_file():
            return p
    return None


def _file_digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def discover_accepted_versions() -> list[tuple[str, Path]]:
    found: dict[int, Path] = {}
    for root in (DATA_TI, SAVES, ENGINE_OUT):
        if not root.is_dir():
            continue
        for p in root.glob("Titanium_v*.txt"):
            m = _VERSION_RE.match(p.name)
            if not m:
                continue
            found.setdefault(int(m.group(1)), p)
    return [(f"Titanium_v{n}", found[n]) for n in sorted(found)]


STARCHEESE = _resolve_graph(SAVES / "StarCheese.txt", ENGINE_OUT / "StarCheese.txt")

EXTERNAL_TARGETS = [
    ("AIA", SAVES / "AIA.txt"),
    ("AIA3", SAVES / "AIA3.txt"),
    ("Poponeta", SAVES / "Poponeta.txt"),
    ("Haialand-v2", HAIALAND),
    ("StarCheese", STARCHEESE),
]


def build_targets() -> list[tuple[str, Path]]:
    """Externals + Titanium_vN, deduped by content (skip graphs identical to live)."""
    targets: list[tuple[str, Path]] = []
    seen: dict[str, str] = {}
    live_dig = _file_digest(LIVE) if LIVE.is_file() else None

    for name, path in EXTERNAL_TARGETS + discover_accepted_versions():
        if path is None or not path.is_file():
            print(f"  skip {name} — missing", file=sys.stderr)
            continue
        dig = _file_digest(path)
        if live_dig is not None and dig == live_dig:
            print(f"  note: {name} identical to live champion — skip as target", file=sys.stderr)
            continue
        if dig in seen:
            print(f"  note: {name} identical to {seen[dig]} — skip", file=sys.stderr)
            continue
        seen[dig] = name
        targets.append((name, path))
    return targets


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


def games_played(matches, bot):
    return sum(1 for m in matches if m["home"] == bot or m["away"] == bot)


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


def build_target_fixtures(contenders, targets):
    fixtures = []
    for ti_name, ti_path in contenders:
        for opp_name, opp_path in targets:
            fixtures.append((ti_name, ti_path, opp_name, opp_path, "home"))
            fixtures.append((opp_name, opp_path, ti_name, ti_path, "away"))
    return fixtures


def build_h2h_fixtures(contenders):
    by_name = {n: p for n, p in contenders}
    if CHAMPION_NAME not in by_name or CHALLENGER_NAME not in by_name:
        return []
    ch_p = by_name[CHAMPION_NAME]
    cl_p = by_name[CHALLENGER_NAME]
    return [
        (CHAMPION_NAME, ch_p, CHALLENGER_NAME, cl_p, "home"),
        (CHAMPION_NAME, ch_p, CHALLENGER_NAME, cl_p, "away"),
        (CHALLENGER_NAME, cl_p, CHAMPION_NAME, ch_p, "home"),
        (CHALLENGER_NAME, cl_p, CHAMPION_NAME, ch_p, "away"),
    ]


def deploy_titanium_test(src: Path) -> None:
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
    opps = []
    seen = set()
    for m in matches:
        other = m["away"] if m["home"] == bot else m["home"] if m["away"] == bot else None
        if other is None or other in seen:
            continue
        seen.add(other)
        opps.append(other)

    print(f"\n== {bot}: by opponent ==\n")
    print(f"  {'opponent':14s}  {'GP':>3}  {'GF':>4}  {'GA':>4}  {'GD':>4}  results")
    tot_gp = tot_gf = tot_ga = 0
    for opp in opps:
        gf = ga = gp = 0
        rows = []
        for m in matches:
            if m["home"] == bot and m["away"] == opp:
                gp += 1
                gf += m["score_home"]
                ga += m["score_away"]
                kick = "ko-H" if m["opening"] == "home" else "ko-A"
                rows.append(f"{m['score_home']}-{m['score_away']} ({kick})")
            elif m["away"] == bot and m["home"] == opp:
                gp += 1
                gf += m["score_away"]
                ga += m["score_home"]
                kick = "ko-H" if m["opening"] == "home" else "ko-A"
                rows.append(f"{m['score_away']}-{m['score_home']} (away,{kick})")
        tot_gp += gp
        tot_gf += gf
        tot_ga += ga
        print(
            f"  {opp:14s}  {gp:3d}  {gf:4d}  {ga:4d}  {gf - ga:4d}  {', '.join(rows)}"
        )
    print(
        f"  {'TOTAL':14s}  {tot_gp:3d}  {tot_gf:4d}  {tot_ga:4d}  {tot_gf - tot_ga:4d}"
    )


def print_scoreboard(matches, names):
    print("\n== Scoreboard ==")
    print("  GP=games played  GF=goals for  GA=goals against  GD=goal difference\n")
    print(f"  {'bot':12s}  {'GP':>3}  {'GF':>4}  {'GA':>4}  {'GD':>5}")
    rows = []
    for n in names:
        gp = games_played(matches, n)
        gf = total_goals_for(matches, n)
        ga = total_conceded_for(matches, n)
        rows.append((n, gp, gf, ga, gf - ga))
    for n, gp, gf, ga, gd in sorted(rows, key=lambda r: (-r[2], -r[4])):
        print(f"  {n:12s}  {gp:3d}  {gf:4d}  {ga:4d}  {gd:+5d}")


def print_h2h(matches):
    h2h = [
        m
        for m in matches
        if {m["home"], m["away"]} == {CHAMPION_NAME, CHALLENGER_NAME}
    ]
    if not h2h:
        return
    print("\n== Champion ↔ challenger (4 configs: side × kickoff) ==\n")
    for m in h2h:
        side = f"{m['home']} home"
        kick = f"{m['home']} kicks" if m["opening"] == "home" else f"{m['away']} kicks"
        print(
            f"  {m['home']:12s} {m['score_home']}-{m['score_away']} {m['away']:12s}  "
            f"({side}, {kick})"
        )
    ch_gf = total_goals_for(h2h, CHAMPION_NAME)
    cl_gf = total_goals_for(h2h, CHALLENGER_NAME)
    print(f"\n  H2H goals: champion {ch_gf}  challenger {cl_gf}")


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

    targets = build_targets()
    fixtures = build_target_fixtures(contenders, targets) + build_h2h_fixtures(contenders)
    target_names = [n for n, _ in targets]
    print("== Promotion test (180s, total goals) ==\n")
    print(f"  Contenders: {', '.join(names)}")
    print(f"  Targets:    {' -> '.join(target_names)}")
    print("  + champ↔challenger × 4 (both sides × both kickoffs)")
    print("  Rule: highest GF across full slate wins. No self-play.\n")
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
    print_h2h(matches)
    print_scoreboard(matches, names)

    totals = {n: total_goals_for(matches, n) for n in names}
    tg = totals[CHALLENGER_NAME]
    cg = totals[CHAMPION_NAME]
    print("\n== Verdict ==")
    if tg > cg:
        print(f"  Challenger wins gate (GF {tg} vs champion {cg}). Promoting.")
        promote_challenger()
        deploy_titanium_test(CHALLENGER)
    else:
        print(f"  No promotion — challenger GF {tg}, champion GF {cg}.")
        deploy_titanium_test(CHALLENGER)
    return 0


def main():
    return run_promotion_pipeline()


if __name__ == "__main__":
    sys.exit(main())
