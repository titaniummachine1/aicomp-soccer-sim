#!/usr/bin/env python3
"""Progressively strip Titanium JSON chrome until parity_check breaks."""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

SIM = Path(__file__).resolve().parent.parent
OUT = SIM / "data" / "titanium"
LEAN = OUT / "Titanium_lean.txt"


def save(doc: dict, path: Path) -> int:
    path.write_text(
        json.dumps(doc, separators=(",", ":"), ensure_ascii=False),
        encoding="utf-8",
    )
    return path.stat().st_size


def parity(path_a: Path, path_b: Path, secs: int = 60, seeds: int = 1) -> tuple[bool, int, str]:
    cmd = [
        "cargo",
        "run",
        "--release",
        "--bin",
        "parity_check",
        "--",
        "--a",
        f"graph:{path_a}",
        "--b",
        f"graph:{path_b}",
        "--away",
        f"graph:{OUT / 'Haialand-v2.txt'}",
        "--secs",
        str(secs),
        "--seed",
        "12345",
        "--seeds",
        str(seeds),
    ]
    done = subprocess.run(cmd, cwd=str(SIM), capture_output=True, text=True)
    text = done.stdout + done.stderr
    ok = done.returncode == 0 and "RESULT: equivalent" in text
    lines = [
        line
        for line in text.splitlines()
        if any(
            key in line
            for key in ("IDENTICAL", "DIFF", "RESULT", "error", "Error", "panic", "failed")
        )
    ]
    return ok, done.returncode, "\n".join(lines[-12:])


def step_leaner_base(d: dict) -> None:
    for n in d["serializableNodes"]:
        if not n.get("ownerFunctionSID"):
            n.pop("ownerFunctionSID", None)
        if n.get("modifier", None) in ("", None):
            n.pop("modifier", None)
        t = n.get("serializableRectTransform")
        if isinstance(t, dict):
            t.pop("position", None)
            t.pop("scale", None)
            for key in ("localPosition", "anchorMin", "anchorMax", "sizeDelta"):
                t.pop(key, None)
        for p in n.get("serializablePorts", []):
            p.pop("serializableRectTransform", None)
            p.pop("controlPointSerializableRectTransform", None)
        if n.get("id") not in ("Region", "CreateFunction"):
            n.pop("serializeSizeDelta", None)
            n.pop("serializeColor", None)
            n.pop("defaultColor", None)
            n.pop("serializableDefaultColor", None)
    slim = []
    for c in d["serializableConnections"]:
        slim.append({k: c[k] for k in ("sID", "port0SID", "port1SID") if k in c})
    d["serializableConnections"] = slim


def step_drop_conn_sid(d: dict) -> None:
    for c in d["serializableConnections"]:
        c.pop("sID", None)


def step_drop_port_nodesid(d: dict) -> None:
    for n in d["serializableNodes"]:
        for p in n.get("serializablePorts", []):
            p.pop("nodeSID", None)


def step_drop_all_rects(d: dict) -> None:
    for n in d["serializableNodes"]:
        n.pop("serializableRectTransform", None)


def step_drop_serialize_flags(d: dict) -> None:
    for n in d["serializableNodes"]:
        n.pop("serializeSizeDelta", None)
        n.pop("serializeColor", None)
        n.pop("defaultColor", None)
        n.pop("serializableDefaultColor", None)


def step_drop_empty_modifiers(d: dict) -> None:
    for n in d["serializableNodes"]:
        mod = n.get("modifier", None)
        if mod in ("", None) or (isinstance(mod, str) and not mod.strip()):
            n.pop("modifier", None)


def step_drop_all_modifiers(d: dict) -> None:
    """Almost certainly breaks — constants / dropdowns live here."""
    for n in d["serializableNodes"]:
        n.pop("modifier", None)


def step_drop_port_polarity(d: dict) -> None:
    for n in d["serializableNodes"]:
        for p in n.get("serializablePorts", []):
            p.pop("polarity", None)


def main() -> int:
    if not LEAN.is_file():
        print(f"missing {LEAN}", file=sys.stderr)
        return 1
    base = json.loads(LEAN.read_text(encoding="utf-8"))
    lean_size = LEAN.stat().st_size
    print(f"baseline lean: {lean_size:,} bytes")

    layers: list[tuple[str, list]] = [
        ("L1_leaner_base", [step_leaner_base]),
        ("L2_drop_conn_sID", [step_leaner_base, step_drop_conn_sid]),
        (
            "L3_drop_port_nodeSID",
            [step_leaner_base, step_drop_conn_sid, step_drop_port_nodesid],
        ),
        (
            "L4_drop_all_rects",
            [
                step_leaner_base,
                step_drop_conn_sid,
                step_drop_port_nodesid,
                step_drop_all_rects,
            ],
        ),
        (
            "L5_drop_serialize_flags",
            [
                step_leaner_base,
                step_drop_conn_sid,
                step_drop_port_nodesid,
                step_drop_all_rects,
                step_drop_serialize_flags,
                step_drop_empty_modifiers,
            ],
        ),
        (
            "L6_drop_all_modifiers_EXPECT_FAIL",
            [
                step_leaner_base,
                step_drop_conn_sid,
                step_drop_port_nodesid,
                step_drop_all_rects,
                step_drop_serialize_flags,
                step_drop_all_modifiers,
            ],
        ),
        (
            "L7_drop_polarity_EXPECT_FAIL",
            [
                step_leaner_base,
                step_drop_conn_sid,
                step_drop_port_nodesid,
                step_drop_all_rects,
                step_drop_port_polarity,
            ],
        ),
    ]

    last_ok: tuple[str, Path, int] | None = None
    broke_at: str | None = None

    for name, steps in layers:
        doc = deepcopy(base)
        for step in steps:
            step(doc)
        path = OUT / f"Titanium_{name}.txt"
        size = save(doc, path)
        pct = 100.0 * (lean_size - size) / lean_size
        print(f"\n== {name} == {size:,} bytes ({pct:.1f}% smaller than lean)")
        # Quick screen first; full check only if quick passes? Always use 60s for speed.
        ok, rc, summary = parity(LEAN, path, secs=60, seeds=1)
        print(summary or "(no parity summary lines)")
        print("PASS" if ok else f"FAIL rc={rc}")
        if ok:
            last_ok = (name, path, size)
        else:
            broke_at = name
            # Continue only through EXPECT_FAIL layers for mapping the cliff.
            if "EXPECT_FAIL" not in name:
                break

    if last_ok:
        print(f"\nLast good layer: {last_ok[0]} ({last_ok[2]:,} bytes)")
        # Longer confirmation on the winner.
        print("\n== confirming last good with 180s x 2 seeds ==")
        ok, rc, summary = parity(LEAN, last_ok[1], secs=180, seeds=2)
        print(summary)
        print("PASS" if ok else f"FAIL rc={rc}")
        if ok:
            dest = OUT / "Titanium_leaner.txt"
            shutil.copy2(last_ok[1], dest)
            print(f"updated {dest} ({dest.stat().st_size:,} bytes)")
        else:
            print("long confirmation failed; not updating Titanium_leaner.txt")
    if broke_at:
        print(f"First hard break (or expected): {broke_at}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
