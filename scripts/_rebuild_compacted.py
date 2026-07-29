#!/usr/bin/env python3
import json
import sys
from pathlib import Path

sys.path.insert(0, r"C:\gitProjects\worldcup\worldcupteams\AIGamePyLibrary")
from AIGamePyLibrary import OptimizeFile

saves = Path.home() / "AppData" / "LocalLow" / "Unicorn One" / "AIComp" / "Saves" / "Soccer"
out = Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim\data\titanium\compacted")
jobs = [
    ("AIA", saves / "AIA.txt"),
    ("AIA3", saves / "AIA3.txt"),
    ("Titanium", Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim\data\titanium\Titanium.txt")),
]
for name, src in jobs:
    dest = out / f"{name}__leaner_remap.txt"
    before = src.stat().st_size
    OptimizeFile(
        str(src),
        str(dest),
        optimize="normal",
        pruneUnusedNodes=False,
        layout=None,
        leaner=True,
        remap_sids=True,
        verbose=True,
    )
    d = json.loads(dest.read_text(encoding="utf-8"))
    sets = sum(1 for n in d["serializableNodes"] if n["id"] == "SetVariable")
    gets = sum(1 for n in d["serializableNodes"] if n["id"] == "GetVariable")
    print(
        f"{name}: {before} -> {dest.stat().st_size} "
        f"nodes={len(d['serializableNodes'])} sets={sets} gets={gets}"
    )
    for label in (f"{name}_compacted.txt", f"{name}_leaner_remap.txt"):
        (saves / label).write_bytes(dest.read_bytes())
        print("deployed", saves / label)
