"""Compare Unity TimePlot (EU decimals) vs sim TimePlot JSON."""
from __future__ import annotations

import json
import math
import re
import sys
from pathlib import Path


def load_eu_json(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    # Unity export uses comma as decimal separator in numbers.
    text = re.sub(r"(?<=\d),(?=\d)", ".", text)
    return json.loads(text)


def series_map(doc: dict) -> dict[str, dict]:
    out = {}
    for s in doc.get("series") or []:
        name = s.get("name")
        if not name:
            continue
        out[name] = s
    return out


def rmse(a: list[float], b: list[float]) -> float | None:
    n = min(len(a), len(b))
    if n < 2:
        return None
    # Align by index (both fixed-dt ~0.019); trim to shared length.
    err = 0.0
    for i in range(n):
        d = a[i] - b[i]
        err += d * d
    return math.sqrt(err / n)


def span(ys: list[float]) -> float:
    if not ys:
        return 0.0
    return max(ys) - min(ys)


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: compare_timeplots.py <unity.json> <sim.json>")
        return 2
    unity = load_eu_json(Path(sys.argv[1]))
    sim = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
    um, sm = series_map(unity), series_map(sim)
    u_names, s_names = set(um), set(sm)
    shared = sorted(u_names & s_names)
    only_u = sorted(u_names - s_names)
    only_s = sorted(s_names - u_names)

    print(f"Unity simTime={unity.get('simTime')} series={len(um)}")
    print(f"Sim   simTime={sim.get('sim_time') or sim.get('simTime')} series={len(sm)}")
    print(f"shared={len(shared)} only_unity={len(only_u)} only_sim={len(only_s)}")

    # Priority channels for gameplay parity.
    priority = [
        "Meta.DebugBuild",
        "Ball.X",
        "Ball.Z",
        "T1.X",
        "T1.Z",
        "O1.X",
        "O1.Z",
        "Ctrl.Striker.MoveTo.X",
        "Ctrl.Striker.MoveTo.Z",
        "Ctrl.P1.MoveTo.X",
        "Ctrl.P1.MoveTo.Z",
        "Ctrl.Charge",
        "In.Float.Team_Player_1_Stamina",
        "In.Bool.Team_Player_1_Has_Ball",
        "In.Bool.Opponent_Has_Ball",
        "In.Bool.Is_Kickoff",
        "Event.PossCode",
        "Event.Team_Score",
        "Event.Opp_Score",
        "Aim.Clear.Carrier.X",
        "Aim.Clear.Carrier.Z",
        "Aim.OppGoal.T1.X",
        "Aim.OppGoal.T1.Z",
        "Sensor.A.Hit",
        "Sensor.A.Dist",
        "Sensor.E.Hit",
        "Sensor.E.Dist",
    ]

    print("\n=== Priority channel RMSE (index-aligned) ===")
    rows = []
    for name in priority:
        if name not in shared:
            print(f"  MISSING  {name}  unity={name in um} sim={name in sm}")
            continue
        uy, sy = um[name].get("y") or [], sm[name].get("y") or []
        r = rmse(uy, sy)
        us, ss = span(uy), span(sy)
        rows.append((r or 999.0, name, r, len(uy), len(sy), us, ss))
        print(
            f"  rmse={r:8.3f}  n={min(len(uy), len(sy)):5d}  "
            f"spanU={us:7.2f} spanS={ss:7.2f}  {name}"
            if r is not None
            else f"  rmse=   n/a  {name}"
        )

    # Worst shared Ball/T1/Ctrl among all shared
    print("\n=== Worst 15 shared by RMSE (Ball/T/Ctrl/In/Event/Aim) ===")
    scored = []
    for name in shared:
        if not any(
            name.startswith(p)
            for p in ("Ball", "T1", "T2", "O1", "Ctrl", "In.", "Event", "Aim.")
        ):
            continue
        r = rmse(um[name].get("y") or [], sm[name].get("y") or [])
        if r is not None:
            scored.append((r, name))
    for r, name in sorted(scored, reverse=True)[:15]:
        print(f"  rmse={r:8.3f}  {name}")

    print("\n=== Unity-only (first 30) ===")
    for n in only_u[:30]:
        print(f"  {n}")
    print("\n=== Sim-only (first 30) ===")
    for n in only_s[:30]:
        print(f"  {n}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
