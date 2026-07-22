"""Report blank / soft graph nodes that stock AIA.txt actually uses."""
from __future__ import annotations

import json
from collections import Counter
from pathlib import Path

SAVES = (
    Path.home()
    / "AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/AIA.txt"
)

# Eval → Null today (or visual-only).
BLANK = {
    "Spherecast",
    "HitInfo",
    "SoccerPlayerSensors1",
    "SoccerPlayerSensors2",
    "SoccerPlayerSensors3",
    "SoccerPlayerSensors4",
    "ConstructSoccerProperties",
    "Country",
    "Color",
    "Stat",
    "Keypress",
    "Debug",
    "DebugDrawDisc",
    "DebugDrawLine",
    "TimePlot",
    "Region",
}

# SoccerGet labels that are wired but soft / approximate.
SOFT_GET = {
    "Direction of opponent goal from Teammate 1",
    "Direction of opponent goal from Teammate 2",
    "Direction of opponent goal from Teammate 3",
    "Direction of opponent goal from Teammate 4",
    "Clear direction from Teammate 1",
    "Clear direction from Teammate 2",
    "Clear direction from Teammate 3",
    "Clear direction from Teammate 4",
    "Clear direction from team carrier",
    "Clear direction from team carrier (avoid all walls)",
    "Clear direction from team carrier (avoid goal lines)",
    "Clear direction from team carrier (avoid sidelines)",
    "Backwards clear direction from team carrier",
    "Direction of clear teammate from Teammate 1",
    "Direction of clear teammate from Teammate 2",
    "Direction of clear teammate from Teammate 3",
    "Direction of clear teammate from Teammate 4",
    "Stamina of last defending opponent",
}


def main() -> None:
    if not SAVES.exists():
        print(f"missing {SAVES}")
        return
    raw = json.loads(SAVES.read_text(encoding="utf-8"))
    nodes = raw["serializableNodes"]
    ids = Counter(n["id"] for n in nodes)

    print(f"AIA.txt nodes={len(nodes)} types={len(ids)}")
    print("\n=== BLANK node types used by AIA ===")
    for nid in sorted(BLANK):
        c = ids.get(nid, 0)
        if c:
            print(f"  {c:3d}  {nid}")

    print("\n=== Soft SoccerGet* labels AIA reads (clear-dir / stamina) ===")
    used = Counter()
    for n in nodes:
        if n["id"].startswith("SoccerGet") and n.get("modifier") in SOFT_GET:
            used[n["modifier"]] += 1
    for mod, c in used.most_common():
        print(f"  {c:3d}  {mod}")

    print(
        """
Notes
-----
* Spherecast + SoccerPlayerSensors1..4 are in AIA but SoccerGet clear-dirs are
  what the bot consumes for play. Sim approximates those getters geometrically;
  raw HitInfo path is still Null.
* Survival public docs only share locomotion shape (move/sprint/stamina) — NOT
  Soccer spherecast radii/layers. Unity Physics.SphereCast shape is known;
  Soccer numbers need TimePlot / Frida.
* ConstructSoccerProperties + Country are faceoff-only (match setup).
* Color / Region / Debug* are visual — Null as values is fine for headless.
"""
    )


if __name__ == "__main__":
    main()
