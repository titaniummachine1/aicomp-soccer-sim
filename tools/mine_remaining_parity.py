#!/usr/bin/env python3
"""Mine bounce / stamina / hang from existing TimePlots."""
from __future__ import annotations

import json
import math
import re
from pathlib import Path

TP = Path.home() / r"AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/Timeplots"


def load(p: Path) -> dict:
    raw = p.read_text(encoding="utf-8")
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return json.loads(re.sub(r"(\d),(\d)", r"\1.\2", raw))


def names_via_regex(p: Path) -> set[str]:
    raw = p.read_text(encoding="utf-8", errors="ignore")
    return set(re.findall(r'"name"\s*:\s*"([^"]+)"', raw))


def yi_align(by: dict, ref: str, n: str) -> tuple[list[float], list[float]]:
    t = [float(x) for x in by[ref]["x"]]
    yy = [float(v) for v in by[n]["y"]]
    tt = [float(x) for x in by[n]["x"]]
    out: list[float] = []
    j = 0
    for ti in t:
        while j + 1 < len(tt) and abs(tt[j + 1] - ti) <= abs(tt[j] - ti):
            j += 1
        out.append(yy[j])
    return t, out


def bounce_e(path: Path) -> None:
    d = load(path)
    by = {s["name"]: s for s in d["series"]}
    t, byy = yi_align(by, "Ball.X", "Ball.Y")
    _, vy = yi_align(by, "Ball.X", "BallVel.Y")
    rest = 0.40
    ratios = []
    i = 1
    while i < len(t) - 5:
        if byy[i - 1] > rest + 0.08 and byy[i] <= rest + 0.05 and vy[i] < -0.2:
            impact = abs(vy[i])
            j = i
            rebound = 0.0
            while j < len(t) and t[j] - t[i] < 0.45:
                rebound = max(rebound, vy[j])
                j += 1
            if impact > 1.0:
                ratios.append(rebound / impact)
            i = j
        else:
            i += 1
    if ratios:
        ratios.sort()
        print(
            f"bounce {path.name}: n={len(ratios)} median={ratios[len(ratios)//2]:.3f} "
            f"mean={sum(ratios)/len(ratios):.3f}"
        )


def scan_stamina_channels() -> None:
    for p in sorted(TP.glob("timeplot_2026-07-22_*.json"), key=lambda x: x.stat().st_mtime, reverse=True)[
        :20
    ]:
        try:
            names = names_via_regex(p)
        except OSError:
            continue
        stam = sorted(n for n in names if "Stamina" in n)
        if stam:
            print(p.name, "stam=", stam[:12])


if __name__ == "__main__":
    bounce_e(TP / "timeplot_2026-07-22_16-19-09.json")
    scan_stamina_channels()
