#!/usr/bin/env python3
"""Deploy normal / release / core compact variants into Unity Soccer saves.

One file per variant (no duplicate *_leaner_remap copies):
  {name}_normal.txt   — optimize=normal, leaner, keep Regions + layout
  {name}_release.txt  — optimize=release (debug strip), keep Regions + layout
  {name}_core.txt     — release + core (Regions gone, layout rects stripped)

Usage:
  python scripts/deploy_compact_variants.py
  python scripts/deploy_compact_variants.py AIA ZudanFC3.2
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SIM = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SIM.parent / "worldcupteams" / "AIGamePyLibrary"))
from AIGamePyLibrary import OptimizeFile  # noqa: E402

SAVES = Path.home() / "AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer"
OUT = SIM / "data" / "titanium" / "compacted"

# Default bots for side-by-side in-game compare.
DEFAULT_BOTS = {
    "AIA": SAVES / "AIA.txt",
    "ZudanFC3.2": SAVES / "ZudanFC3.2.txt",
}

VARIANTS = (
    # label, optimize, core
    ("normal", "normal", False),
    ("release", "release", False),
    ("core", "release", True),
)

# Old identical twin names — remove so Saves is not cluttered.
LEGACY_DUP_SUFFIXES = ("_compacted.txt", "_leaner_remap.txt")


def summarize(path: Path) -> str:
    d = json.loads(path.read_text(encoding="utf-8"))
    nodes = d["serializableNodes"]
    regions = sum(1 for n in nodes if n.get("id") == "Region")
    rects = sum(1 for n in nodes if "serializableRectTransform" in n)
    debug = sum(
        1
        for n in nodes
        if str(n.get("id", "")).startswith("Debug") or n.get("id") == "TimePlot"
    )
    return (
        f"size={path.stat().st_size:,} nodes={len(nodes)} "
        f"regions={regions} rects={rects} debug={debug}"
    )


def deploy_one(name: str, src: Path) -> None:
    if not src.is_file():
        print(f"SKIP missing {name}: {src}")
        return
    OUT.mkdir(parents=True, exist_ok=True)
    before = src.stat().st_size
    print(f"\n==== {name} ({before:,} bytes) ====")
    for label, optimize, core in VARIANTS:
        dest = OUT / f"{name}__{label}.txt"
        OptimizeFile(
            str(src),
            str(dest),
            optimize=optimize,
            pruneUnusedNodes=False,
            layout=None,
            leaner=True,
            remap_sids=True,
            core=core,
            verbose=False,
        )
        save_name = f"{name}_{label}.txt"
        target = SAVES / save_name
        target.write_bytes(dest.read_bytes())
        print(f"  {save_name}: {summarize(dest)}")
        print(f"    deployed -> {target}")

    for suffix in LEGACY_DUP_SUFFIXES:
        legacy = SAVES / f"{name}{suffix}"
        if legacy.is_file():
            legacy.unlink()
            print(f"  removed legacy duplicate {legacy.name}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "bots",
        nargs="*",
        help="Bot names (default: AIA ZudanFC3.2). Sources from Soccer saves.",
    )
    args = ap.parse_args()
    names = args.bots or list(DEFAULT_BOTS)
    for name in names:
        src = DEFAULT_BOTS.get(name) or (SAVES / f"{name}.txt")
        deploy_one(name, src)
    print("\ndone — load *_normal / *_release / *_core in AIComp to compare")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
