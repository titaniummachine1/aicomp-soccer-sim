"""Validate stock AIA for Left+Right (Home+Away) against sim node support.

Checks that every node type / SoccerGet label used by AIA.txt is classified:
  OK        — implemented like Unity for AIA gameplay paths
  SOFT      — wired but approximate (clear-dir / some %)
  VIZ       — Null OK (Debug / Region / TimePlot / Color)
  FACEOFF   — Null OK (ConstructSoccerProperties / Country)
  SENSOR    — Null today; AIA play uses SoccerGet clear-dirs instead
  GAP       — unknown / unimplemented on a hot path → fail

Then runs:
  cargo test --lib (AIA-related soft-skip if Saves missing)
  soccer_headless --home aia --away aia  (Left=AIA, Right=AIA)

Exit 0 only if inventory has no GAP and headless JSON ok=true.

Usage:
  python scripts/validate_aia_lr.py
  python scripts/validate_aia_lr.py --secs 20 --seed 7
  python scripts/validate_aia_lr.py --skip-headless
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SAVES = (
    Path.home() / "AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/AIA.txt"
)

# Graph / VM nodes that AIA may use — implemented for gameplay.
OK_NODES = {
    "Float",
    "Bool",
    "String",  # AIA: dangling / label-only (no live consumers); eval keeps modifier
    "GetVariable",
    "SetVariable",
    "Function",
    "CreateFunction",
    "SoccerGetBool",
    "SoccerGetFloat",
    "SoccerGetTransform",
    "SoccerGetVector3",
    "RelativePosition",
    "ConstructVector3",
    "Vector3Split",
    "AddFloats",
    "SubtractFloats",
    "MultiplyFloats",
    "DivideFloats",
    "Power",
    "Modulo",
    "Lerp",
    "Relay",
    "Operation",
    "AbsFloat",
    "Absolute",
    "AddVector3",
    "SubtractVector3",
    "ScaleVector3",
    "Normalize",
    "Magnitude",
    "Distance",
    "DotProduct",
    "Not",
    "CompareFloats",
    "CompareBool",
    "ConditionalSetBool",
    "ConditionalSetFloat",
    "ConditionalSetFloatV2",
    "ConditionalSetVector3",
    "IsNull",
    "Keypress",  # live in viewer; headless always false (Unity has no keys there either)
    "SoccerController1",
    "SoccerController2",
    "SoccerController3",
    "SoccerController4",
}

VIZ_NODES = {
    "Color",
    "Region",
    "Debug",
    "DebugDrawDisc",
    "DebugDrawLine",
    "TimePlot",
}

FACEOFF_NODES = {
    "ConstructSoccerProperties",
    "Country",
}

# Present in AIA; VM Null. Gameplay uses SoccerGet clear-dirs (SOFT below).
SENSOR_NODES = {
    "Spherecast",
    "HitInfo",
    "SoccerPlayerSensors1",
    "SoccerPlayerSensors2",
    "SoccerPlayerSensors3",
    "SoccerPlayerSensors4",
}

# SoccerGet modifiers that are approximate vs Unity (see docs/API_GAPS.md).
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


def classify_node(nid: str) -> str:
    if nid in OK_NODES:
        return "OK"
    if nid in VIZ_NODES:
        return "VIZ"
    if nid in FACEOFF_NODES:
        return "FACEOFF"
    if nid in SENSOR_NODES:
        return "SENSOR"
    if nid == "Stat":
        return "VIZ"  # not Soccer; Null OK
    return "GAP"


def inventory(path: Path) -> tuple[Counter[str], Counter[str], Counter[str]]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    nodes = raw["serializableNodes"]
    ids = Counter(n["id"] for n in nodes)
    gets: Counter[str] = Counter()
    soft: Counter[str] = Counter()
    for n in nodes:
        if n["id"].startswith("SoccerGet"):
            mod = (n.get("modifier") or "").strip()
            if not mod:
                continue
            gets[mod] += 1
            if mod in SOFT_GET:
                soft[mod] += 1
    return ids, gets, soft


def report_inventory(ids: Counter[str], soft: Counter[str]) -> list[str]:
    gaps: list[str] = []
    by_class: dict[str, list[tuple[str, int]]] = {
        "OK": [],
        "VIZ": [],
        "FACEOFF": [],
        "SENSOR": [],
        "GAP": [],
    }
    for nid, c in sorted(ids.items(), key=lambda kv: (-kv[1], kv[0])):
        cls = classify_node(nid)
        by_class[cls].append((nid, c))
        if cls == "GAP":
            gaps.append(f"{nid} ×{c}")

    print(f"AIA.txt  nodes={sum(ids.values())}  types={len(ids)}")
    for cls in ("GAP", "SENSOR", "SOFT", "FACEOFF", "VIZ", "OK"):
        if cls == "SOFT":
            print(f"\n=== SOFT SoccerGet labels used ({len(soft)}) ===")
            for mod, c in soft.most_common():
                print(f"  {c:3d}  {mod}")
            continue
        rows = by_class[cls]
        print(f"\n=== {cls} node types ({len(rows)}) ===")
        for nid, c in rows:
            print(f"  {c:3d}  {nid}")

    if gaps:
        print("\nFAIL: unclassified / unimplemented AIA nodes:")
        for g in gaps:
            print(f"  - {g}")
    else:
        print(
            "\nOK: every AIA node type is classified "
            "(OK / VIZ / FACEOFF / SENSOR). No unknown gaps."
        )
        print(
            "Note: SENSOR nodes are Null in the VM; AIA play reads SoccerGet "
            "clear-dirs (SOFT). Unity-exact spherecast->HitInfo is still open."
        )
    return gaps


def run(cmd: list[str], cwd: Path) -> int:
    print(f"\n$ {' '.join(cmd)}")
    p = subprocess.run(cmd, cwd=cwd)
    return p.returncode


def run_headless(secs: float, seed: int) -> int:
    """Left=AIA (--home), Right=AIA (--away). Parse last JSON line."""
    cmd = [
        "cargo",
        "run",
        "--release",
        "--bin",
        "soccer_headless",
        "--",
        "--secs",
        str(secs),
        "--home",
        "aia",
        "--away",
        "aia",
        "--seed",
        str(seed),
        "--quiet",
    ]
    print(f"\n$ {' '.join(cmd)}")
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        print(p.stderr[-4000:] if p.stderr else "(no stderr)")
        print(f"headless exit {p.returncode}")
        return p.returncode
    lines = [ln.strip() for ln in (p.stdout or "").splitlines() if ln.strip()]
    if not lines:
        print("FAIL: headless produced no JSON")
        return 2
    try:
        row = json.loads(lines[-1])
    except json.JSONDecodeError as e:
        print(f"FAIL: bad JSON: {e}\n{lines[-1][:500]}")
        return 2
    print(
        "headless Left=AIA Right=AIA -> "
        f"ok={row.get('ok')} score={row.get('score_home')}-{row.get('score_away')} "
        f"clock_s={row.get('clock_s')} ticks={row.get('ticks')}"
    )
    if not row.get("ok"):
        print(f"FAIL: headless ok!=true  full={row}")
        return 3
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--secs", type=float, default=20.0)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--skip-tests", action="store_true")
    ap.add_argument("--skip-headless", action="store_true")
    ap.add_argument("--graph", type=Path, default=SAVES, help="AIA graph path")
    args = ap.parse_args()

    if not args.graph.exists():
        print(f"missing AIA graph: {args.graph}")
        print("Install Unity AIComp Soccer saves or pass --graph <path>")
        return 1

    ids, _gets, soft = inventory(args.graph)
    gaps = report_inventory(ids, soft)
    rc = 1 if gaps else 0

    if not args.skip_tests:
        # AIA load + GraphBrain≡RuntimeBrain + SoccerGet coverage.
        code = run(["cargo", "test", "--lib", "--", "--test-threads=1"], ROOT)
        if code != 0:
            rc = code

    if not args.skip_headless:
        code = run_headless(args.secs, args.seed)
        if code != 0:
            rc = code

    if rc == 0:
        print(
            "\nPASS: AIA Left+Right inventory clean + tests/headless ok.\n"
            "Soft clear-dir SoccerGets remain approximate vs Unity "
            "(docs/API_GAPS.md / AIA_UPSTREAM_QUIRKS.md)."
        )
    else:
        print(f"\nFAIL: exit {rc}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
