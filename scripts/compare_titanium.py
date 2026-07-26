#!/usr/bin/env python3
"""Run identical gate fixtures for every candidate, rank, and on ties ADD
every tied build to the Unity Saves pool so you can watch them in-game."""
import shutil
import sys
from itertools import combinations
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate_candidate  # noqa: E402
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

# (display name, graph path, save-as filename in Unity Soccer folder)
CANDIDATES = [
    ("Titanium (straight walk)", ROOT / "data" / "titanium" / "Titanium.txt", "Titanium.txt"),
    (
        "Titanium (anti-tackle)",
        ROOT.parent / "titanim-socker-engine" / "out" / "Titanium_challenger.txt",
        "Titanium_antitackle.txt",
    ),
    ("Haialand-v2", ROOT / "data" / "titanium" / "Haialand-v2.txt", "Haialand-v2.txt"),
]

WATCH_SCRIPT = ROOT / "scripts" / "watch_bots.ps1"


def score_build(name, path):
    cand = f"graph:{path}"
    rows = []
    total_gd = 0
    sides_won = 0
    sides_played = 0
    for opp_name, opp_path in gate_candidate.ROSTER:
        if not opp_path.is_file():
            continue
        opp = f"graph:{opp_path}"
        for label, home, away, opening, cand_home in (
            ("HOME", cand, opp, "home", True),
            ("AWAY", opp, cand, "away", False),
        ):
            r = gate_lib.run_match(home, away, opening)
            sh, sa = r["score_home"], r["score_away"]
            cs, os = (sh, sa) if cand_home else (sa, sh)
            w = gate_lib.decide(r)
            cand_won = (w == "home") == cand_home and w != "draw"
            sides_played += 1
            sides_won += int(cand_won)
            total_gd += cs - os
            rows.append((opp_name, label, cs, os, "WIN" if cand_won else "LOSS/DRAW"))
    return {"name": name, "path": path, "rows": rows, "sides_won": sides_won, "sides_played": sides_played, "gd": total_gd}


def h2h(a_path, b_path, a_name, b_name):
    rows = []
    a_pts = b_pts = 0
    for label, home, away, opening, a_home in (
        (f"{a_name} home", f"graph:{a_path}", f"graph:{b_path}", "home", True),
        (f"{a_name} away", f"graph:{b_path}", f"graph:{a_path}", "away", False),
    ):
        r = gate_lib.run_match(home, away, opening)
        sh, sa = r["score_home"], r["score_away"]
        w = gate_lib.decide(r)
        if w == "draw":
            a_pts += 1
            b_pts += 1
            tag = "DRAW"
        elif (w == "home") == a_home:
            a_pts += 3
            tag = f"{a_name} WIN"
        else:
            b_pts += 3
            tag = f"{b_name} WIN"
        rows.append((label, sh, sa, tag))
    return rows, a_pts, b_pts


def slug(name):
    if "anti-tackle" in name:
        return "Titanium_antitackle"
    if "straight" in name:
        return "Titanium"
    if "Haialand" in name:
        return "Haialand-v2"
    return name.split()[0]


def install_to_saves(name, src, save_as):
    SAVES.mkdir(parents=True, exist_ok=True)
    dst = SAVES / save_as
    shutil.copy2(src, dst)
    print(f"  installed {name} -> {dst}")
    return dst


def print_watch_commands(pool_names):
    print("\n== Watch in Bevy sim (viewer) ==\n")
    opponents = ["Poponeta", "AIA3", "Titanium", "Titanium_antitackle", "Haialand-v2"]
    for bot in pool_names:
        for opp in opponents:
            if opp == bot:
                continue
            print(f"  .\\scripts\\watch_bots.ps1 -Home {bot} -Away {opp} -Opening home")
            print(f"  .\\scripts\\watch_bots.ps1 -Home {opp} -Away {bot} -Opening away")
    print("\n== Watch in real AIComp Soccer (Unity) ==\n")
    print(f"  Bots copied to: {SAVES}")
    print("  Load any installed .txt from the in-game bot picker.")
    print("\n  Quick start (Bevy):")
    if pool_names:
        b = pool_names[0]
        print(f"    cd aicomp-soccer-sim")
        print(f"    .\\scripts\\watch_bots.ps1 -Home {b} -Away Poponeta")


def main():
    builds = []
    by_name = {}
    save_as = {}
    for name, path, fname in CANDIDATES:
        if not path.is_file():
            print(f"Missing {path}", file=sys.stderr)
            return 1
        b = score_build(name, path)
        builds.append(b)
        by_name[name] = b
        save_as[name] = fname

    print("== Gate roster — same fixtures for every candidate (180s, both sides) ==\n")
    for b in builds:
        print(f"--- {b['name']} ---")
        for opp, side, gf, ga, verdict in b["rows"]:
            print(f"  vs {opp:15s} {side:4s}  {gf}-{ga}  {verdict}")
        print(f"  TOTAL: {b['sides_won']}/{b['sides_played']} sides won, GD {b['gd']:+d}\n")

    print("== Head-to-head (both sides, 180s) ==\n")
    h2h_pts = {b["name"]: 0 for b in builds}
    h2h_tie_pairs = []
    for (n1, p1, _), (n2, p2, _) in combinations(CANDIDATES, 2):
        rows, pts1, pts2 = h2h(p1, p2, slug(n1), slug(n2))
        h2h_pts[n1] += pts1
        h2h_pts[n2] += pts2
        print(f"  {slug(n1)} vs {slug(n2)}")
        for label, sh, sa, tag in rows:
            print(f"    {label:22s}  {sh}-{sa}  {tag}")
        print(f"    points: {slug(n1)} {pts1}  {slug(n2)} {pts2}\n")
        if pts1 == pts2:
            h2h_tie_pairs.append((n1, n2))

    ranked = sorted(
        builds,
        key=lambda b: (-b["sides_won"], -b["gd"], -h2h_pts[b["name"]]),
    )
    print("== Standings (gate sides won, then GD, then h2h points) ==\n")
    for i, b in enumerate(ranked, 1):
        gd = b["gd"]
        pts = h2h_pts[b["name"]]
        print(
            f"  {i}. {b['name']:28s}  "
            f"{b['sides_won']}/{b['sides_played']} sides  GD {gd:+3d}  "
            f"h2h pts {pts}"
        )

    leader = ranked[0]
    top_key = (leader["sides_won"], leader["gd"])
    standings_tied = [b for b in builds if (b["sides_won"], b["gd"]) == top_key]

    pool = {leader["name"]}
    for b in standings_tied:
        pool.add(b["name"])
    for n1, n2 in h2h_tie_pairs:
        pool.add(n1)
        pool.add(n2)

    print(f"\n== Active pool (leader + standings ties + h2h ties) ==\n")
    for name in sorted(pool, key=lambda n: -h2h_pts[n]):
        tag = []
        if name == leader["name"]:
            tag.append("leader")
        if any(name in pair for pair in h2h_tie_pairs):
            tag.append("h2h-tie")
        if name in {b["name"] for b in standings_tied} and len(standings_tied) > 1:
            tag.append("standings-tie")
        print(f"  + {name}  ({', '.join(tag) or 'included'})")

    print("\n== Installing to Unity Saves ==\n")
    for name, path, fname in CANDIDATES:
        if name in pool:
            install_to_saves(name, path, fname)

    pool_slugs = [slug(n) for n in pool]
    print_watch_commands(pool_slugs)

    print(f"\n== Verdict ==\n  Leader: {leader['name']}")
    if len(standings_tied) > 1:
        print(f"  Standings tie at top ({len(standings_tied)} bots) — all added to pool.")
    if h2h_tie_pairs:
        print(f"  H2H ties: {len(h2h_tie_pairs)} pair(s) — both sides added to pool.")
    if leader["name"].startswith("Titanium (straight") and len(pool) == 1:
        print("  Clear win — champion stays alone in pool.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
