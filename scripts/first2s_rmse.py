#!/usr/bin/env python3
"""First-2s RMSE between Unity and sim TimePlots (EU commas OK)."""
from __future__ import annotations

import json
import math
import re
import sys
from pathlib import Path


def load(p: Path) -> dict:
    t = p.read_text(encoding="utf-8")
    # Unity EU decimals use ',' and array separators ', '; US sim uses '.'
    # and compact ','. Only rewrite when simTime itself is EU-formatted.
    if re.search(r'"simTime"\s*:\s*-?\d+,', t):
        t = re.sub(r"(?<=\d),(?=\d)", ".", t)
    t = t.replace("NaN", "null")
    return json.loads(t)


def series_map(doc: dict) -> dict:
    return {s["name"]: s for s in doc["series"]}


def interp(ser: dict, t: float) -> float | None:
    xs, ys = ser["x"], ser["y"]
    if not xs:
        return None
    if t <= xs[0]:
        return ys[0]
    if t >= xs[-1]:
        return ys[-1]
    lo, hi = 0, len(xs) - 1
    while lo + 1 < hi:
        mid = (lo + hi) // 2
        if xs[mid] <= t:
            lo = mid
        else:
            hi = mid
    x0, x1 = xs[lo], xs[hi]
    y0, y1 = ys[lo], ys[hi]
    if x1 == x0:
        return y0
    a = (t - x0) / (x1 - x0)
    return y0 + (y1 - y0) * a


def rmse(um: dict, sm: dict, name: str, tmax: float = 2.0, step: float = 0.019) -> float | None:
    if name not in um or name not in sm:
        return None
    errs = []
    t = 0.0
    while t <= tmax + 1e-9:
        u = interp(um[name], t)
        s = interp(sm[name], t)
        if u is not None and s is not None:
            errs.append((u - s) * (u - s))
        t += step
    return math.sqrt(sum(errs) / len(errs)) if errs else None


def samp(m: dict, name: str, tmax: float = 2.0, step: float = 0.2):
    out = []
    t = 0.0
    while t <= tmax + 1e-9:
        y = interp(m[name], t) if name in m else None
        out.append((round(t, 2), None if y is None else round(y, 3)))
        t += step
    return out


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: first2s_rmse.py <unity.json> <sim.json>")
        return 2
    um = series_map(load(Path(sys.argv[1])))
    sm = series_map(load(Path(sys.argv[2])))
    names = [
        "Ball.X",
        "Ball.Z",
        "T1.X",
        "T1.Z",
        "O1.X",
        "O1.Z",
        "Ctrl.Striker.MoveTo.X",
        "Ctrl.Striker.MoveTo.Z",
        "Ctrl.Charge",
        "In.Bool.Is_Kickoff",
        "In.Bool.Opponent_Has_Ball",
        "Event.PossCode",
    ]
    print("first-2s RMSE:")
    for n in names:
        r = rmse(um, sm, n)
        print(f"  {n:28s} {r if r is not None else float('nan'):.3f}")
    for n in ["O1.X", "O1.Z", "Ball.X", "Ball.Z", "T1.X", "T1.Z", "In.Bool.Is_Kickoff"]:
        print(f"U {n}: {samp(um, n)}")
        print(f"S {n}: {samp(sm, n)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
