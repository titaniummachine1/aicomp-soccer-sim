#!/usr/bin/env python3
"""Regression gate for PerfectController prediction compaction.

Correct method (see docs/AIA_UPSTREAM_QUIRKS.md parity invariant):
  1. Inject identical ball state into sim
  2. Run graph prediction (DebugDraw) and sim rollout from that state
  3. Compare terminal rest AND max pointwise path error

Usage:
  py -3.12 scripts/perfect_prediction_regress.py
  py -3.12 scripts/perfect_prediction_regress.py --build
  py -3.12 scripts/perfect_prediction_regress.py --build --viewer

Fails if compact graph drifts from baseline or either drifts from sim rollout.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

SIM = Path(__file__).resolve().parents[1]
WORLDCP = SIM.parent / "worldcupteams"
OUT = SIM / "data" / "perfect_parity"
SAVES = Path.home() / "AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer"

# Same cases as ball_physics_sim regression + wall cases.
CASES = [
    ("center_roll", "0,0,12,0"),
    ("diag_45", "0,0,21,21"),
    ("wall_approach", "35,0,-8,0"),
    ("low_speed", "10,5,2,-1"),
]

MAX_TERMINAL_M = 0.05
MAX_PATH_M = 0.10


@dataclass
class ProbeResult:
    lines: list[tuple[float, float, float, float]]
    terminal: tuple[float, float] | None
    sim_rest: tuple[float, float] | None


def build(variant: str) -> Path:
    env = os.environ.copy()
    env["PERFECT_COMPRESS"] = "1" if variant == "compact" else "0"
    subprocess.run(
        [sys.executable, str(WORLDCP / "probes" / "build_perfect_controller.py")],
        cwd=str(WORLDCP),
        env=env,
        check=True,
    )
    src = SAVES / "PerfectController.txt"
    OUT.mkdir(parents=True, exist_ok=True)
    dst = OUT / f"PerfectController_{variant}.txt"
    dst.write_text(src.read_text(encoding="utf-8"), encoding="utf-8")
    return dst


def probe(graph: Path, ball: str) -> ProbeResult:
    cmd = [
        "cargo",
        "run",
        "--release",
        "--bin",
        "probe_dump",
        "--",
        "--home",
        f"graph:{graph}",
        "--ticks",
        "1",
        "--ball",
        ball,
    ]
    out = subprocess.run(cmd, cwd=str(SIM), capture_output=True, text=True, check=True)
    lines: list[tuple[float, float, float, float]] = []
    terminal = None
    sim_rest = None
    for raw in out.stdout.splitlines():
        if raw.startswith("LINE\t"):
            parts = raw.split("\t")
            if len(parts) >= 3:
                a = _parse_xz(parts[1])
                b = _parse_xz(parts[2])
                if a and b:
                    lines.append((a[0], a[1], b[0], b[1]))
        elif raw.startswith("DISC\t"):
            terminal = _parse_xz(raw.split("\t", 2)[-1])
        elif raw.startswith("SIM\t"):
            m = re.search(r"rest=\(([+-]?[\d.]+),([+-]?[\d.]+)\)", raw)
            if m:
                sim_rest = (float(m.group(1)), float(m.group(2)))
    return ProbeResult(lines=lines, terminal=terminal, sim_rest=sim_rest)


def _parse_xz(tag: str) -> tuple[float, float] | None:
    xs = re.findall(r"[-+]?\d*\.?\d+", tag.replace("y=", " "))
    if len(xs) >= 2:
        return float(xs[0]), float(xs[1])
    return None


def dist(a: tuple[float, float] | None, b: tuple[float, float] | None) -> float | None:
    if not a or not b:
        return None
    return math.hypot(a[0] - b[0], a[1] - b[1])


def path_points(lines: list[tuple[float, float, float, float]]) -> list[tuple[float, float]]:
    if not lines:
        return []
    pts = [(lines[0][0], lines[0][1])]
    for _, _, bx, bz in lines:
        pts.append((bx, bz))
    return pts


def path_max_drift(a: ProbeResult, b: ProbeResult) -> float | None:
    pa, pb = path_points(a.lines), path_points(b.lines)
    if not pa or not pb:
        return None
    n = min(len(pa), len(pb))
    return max(math.hypot(pa[i][0] - pb[i][0], pa[i][1] - pb[i][1]) for i in range(n))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true", help="Rebuild both graphs first")
    parser.add_argument(
        "--viewer",
        action="store_true",
        help="After gate passes, launch visible Bevy viewer (implies --build)",
    )
    args = parser.parse_args()

    if args.viewer:
        args.build = True

    if args.build or not (OUT / "PerfectController_baseline.txt").exists():
        build("baseline")
        build("compact")

    baseline = OUT / "PerfectController_baseline.txt"
    compact = OUT / "PerfectController_compact.txt"
    base_nodes = len(json.loads(baseline.read_text())["serializableNodes"])
    compact_nodes = len(json.loads(compact.read_text())["serializableNodes"])
    print(f"baseline nodes={base_nodes}  compact nodes={compact_nodes}  saved={base_nodes - compact_nodes}")

    failed = False
    for name, ball in CASES:
        b = probe(baseline, ball)
        c = probe(compact, ball)
        sim = b.sim_rest
        d_bc = dist(b.terminal, c.terminal)
        d_bs = dist(b.terminal, sim)
        d_cs = dist(c.terminal, sim)
        path_bc = path_max_drift(b, c)
        print(f"\n{name}  ball=({ball})")
        print(f"  baseline terminal {b.terminal}  compact {c.terminal}  sim {sim}")
        print(f"  terminal drift  baseline<->compact {d_bc:.4f}m  baseline<->sim {d_bs:.4f}m  compact<->sim {d_cs:.4f}m")
        print(f"  path max drift   baseline<->compact {path_bc:.4f}m" if path_bc is not None else "  path max drift   n/a")
        if d_bc is not None and d_bc > MAX_TERMINAL_M:
            print(f"  FAIL compact regressed vs baseline (>{MAX_TERMINAL_M}m)")
            failed = True
        if d_bs is not None and d_bs > MAX_TERMINAL_M:
            print(f"  FAIL baseline vs sim (>{MAX_TERMINAL_M}m)")
            failed = True
        if d_cs is not None and d_cs > MAX_TERMINAL_M:
            print(f"  FAIL compact vs sim (>{MAX_TERMINAL_M}m)")
            failed = True
        if path_bc is not None and path_bc > MAX_PATH_M:
            print(f"  FAIL path shape drift (>{MAX_PATH_M}m)")
            failed = True

    if failed:
        print("\nREGRESSION DETECTED — do not ship compact graph.")
        return 1
    print("\nOK — baseline and compact match sim within tolerance.")
    if args.viewer:
        bat = SIM / "scripts" / "run_viewer_perfect_regress.bat"
        # Gate already ran above; skip rebuild on the bat's second pass.
        env = os.environ.copy()
        env["PERFECT_REGRESS_SKIP_BUILD"] = "1"
        print("\nLaunching visible viewer (scripts/run_viewer_perfect_regress.bat)...")
        subprocess.run(["cmd", "/c", str(bat)], cwd=str(SIM), env=env, check=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
