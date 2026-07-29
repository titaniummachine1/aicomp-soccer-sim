#!/usr/bin/env python3
"""Compact every promotion-gate target (leaner + UUID remap) and parity-check
each compacted graph against its original.

Usage:
  python scripts/compact_promote_targets_parity.py
  python scripts/compact_promote_targets_parity.py --secs 60 --seeds 1
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

SIM = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SIM / "scripts"))
sys.path.insert(0, str(SIM.parent / "worldcupteams" / "AIGamePyLibrary"))

import promote_pipeline as promote  # noqa: E402
from AIGamePyLibrary import OptimizeFile  # noqa: E402

OUT = SIM / "data" / "titanium" / "compacted"
OPPONENT = SIM / "data" / "titanium" / "Haialand-v2.txt"


def parity(original: Path, compacted: Path, secs: int, seeds: int) -> tuple[bool, str]:
    cmd = [
        "cargo",
        "run",
        "--release",
        "--bin",
        "parity_check",
        "--",
        "--a",
        f"graph:{original}",
        "--b",
        f"graph:{compacted}",
        "--away",
        "idle",
        "--secs",
        str(secs),
        "--seed",
        "12345",
        "--seeds",
        str(seeds),
    ]
    # Stronger: also vs Haialand if available — run idle first for load/decision parity
    # of the graph under test itself. Away idle isolates the home brain.
    done = subprocess.run(cmd, cwd=str(SIM), capture_output=True, text=True)
    text = (done.stdout or "") + (done.stderr or "")
    ok = done.returncode == 0 and "RESULT: equivalent" in text
    lines = [
        line
        for line in text.splitlines()
        if any(
            k in line
            for k in (
                "IDENTICAL",
                "DIFF",
                "RESULT",
                "error",
                "Error",
                "panic",
                "failed",
                "load ",
            )
        )
    ]
    return ok, "\n".join(lines[-10:]) or text[-800:]


def parity_vs_opponent(
    original: Path, compacted: Path, away: Path, secs: int, seeds: int
) -> tuple[bool, str]:
    cmd = [
        "cargo",
        "run",
        "--release",
        "--bin",
        "parity_check",
        "--",
        "--a",
        f"graph:{original}",
        "--b",
        f"graph:{compacted}",
        "--away",
        f"graph:{away}",
        "--secs",
        str(secs),
        "--seed",
        "12345",
        "--seeds",
        str(seeds),
    ]
    done = subprocess.run(cmd, cwd=str(SIM), capture_output=True, text=True)
    text = (done.stdout or "") + (done.stderr or "")
    ok = done.returncode == 0 and "RESULT: equivalent" in text
    lines = [
        line
        for line in text.splitlines()
        if any(
            k in line
            for k in (
                "IDENTICAL",
                "DIFF",
                "RESULT",
                "error",
                "Error",
                "panic",
                "failed",
                "load ",
            )
        )
    ]
    return ok, "\n".join(lines[-10:]) or text[-800:]


def collect_bots() -> list[tuple[str, Path]]:
    bots: list[tuple[str, Path]] = []
    seen: set[str] = set()
    # Live champion + every promotion target (incl. Zudan — sim now latches cycles).
    for name, path in [("Titanium_live", promote.LIVE), *promote.build_targets()]:
        if path is None or not path.is_file():
            print(f"SKIP missing {name}: {path}")
            continue
        key = str(path.resolve())
        if key in seen:
            continue
        seen.add(key)
        bots.append((name, path.resolve()))
    return bots


def compact_one(name: str, src: Path) -> Path:
    OUT.mkdir(parents=True, exist_ok=True)
    dest = OUT / f"{name}__leaner_remap.txt"
    # Topology-preserving compact: strip debug sinks, leaner chrome, UUID remap.
    # pruneUnusedNodes=False avoids Zudan-style orphan GetVariable assert failures
    # and keeps decision graphs byte-identical in behaviour.
    OptimizeFile(
        str(src),
        str(dest),
        # Chrome + UUID remap only. Do NOT strip Debug*/TimePlot: several
        # promotion targets (AIA/AIA3/Poponeta/Titanium_v2) wire debug sinks
        # into decisions, so release strip changes play.
        optimize="normal",
        pruneUnusedNodes=False,
        layout=None,
        leaner=True,
        remap_sids=True,
        verbose=True,
    )
    return dest


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--secs", type=int, default=60)
    ap.add_argument("--seeds", type=int, default=1)
    ap.add_argument(
        "--vs-haialand",
        action="store_true",
        help="Also parity each bot under Haialand pressure (slower)",
    )
    args = ap.parse_args()

    bots = collect_bots()
    print(f"Compacting {len(bots)} promotion-line bots -> {OUT}")
    results = []
    for name, src in bots:
        before = src.stat().st_size
        print(f"\n==== {name} ====")
        print(f"src {src} ({before:,} bytes)")
        try:
            dest = compact_one(name, src)
        except Exception as exc:
            print(f"COMPACT FAIL: {exc}")
            results.append((name, False, before, 0, f"compact error: {exc}"))
            continue
        after = dest.stat().st_size
        pct = 100.0 * (before - after) / before if before else 0.0
        print(f"dst {dest} ({after:,} bytes, {pct:.1f}% smaller)")

        ok, summary = parity(src, dest, args.secs, args.seeds)
        print(summary)
        print("PARITY idle:", "PASS" if ok else "FAIL")
        detail = summary
        if ok and args.vs_haialand and OPPONENT.is_file() and name != "Haialand-v2":
            ok2, summary2 = parity_vs_opponent(
                src, dest, OPPONENT, args.secs, args.seeds
            )
            print(summary2)
            print("PARITY vs Haialand:", "PASS" if ok2 else "FAIL")
            ok = ok and ok2
            detail = summary + "\n" + summary2
        results.append((name, ok, before, after, detail))

    print("\n======== SUMMARY ========")
    print(f"{'bot':18s} {'ok':4s} {'before':>10s} {'after':>10s} {'save%':>7s}")
    fails = 0
    for name, ok, before, after, _ in results:
        pct = 100.0 * (before - after) / before if before else 0.0
        print(
            f"{name:18s} {'PASS' if ok else 'FAIL':4s} {before:10,d} {after:10,d} {pct:6.1f}%"
        )
        if not ok:
            fails += 1
    print(f"\n{len(results) - fails}/{len(results)} passed parity")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
