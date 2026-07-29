#!/usr/bin/env python3
"""Scan compacted graphs for leftover no-op / DCE opportunities."""
from __future__ import annotations

import json
from collections import Counter, defaultdict
from pathlib import Path

COMP = Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim\data\titanium\compacted")
FILES = {
    "AIA": COMP / "AIA__leaner_remap.txt",
    "AIA3": COMP / "AIA3__leaner_remap.txt",
    "Titanium": COMP / "Titanium_live__leaner_remap.txt",
}


def nf(m) -> str:
    s = str(m).replace(",", ".").strip()
    try:
        return f"{float(s):.6g}"
    except Exception:
        return s


def analyze(name: str, path: Path) -> None:
    d = json.loads(path.read_text(encoding="utf-8"))
    nodes = {n["sID"]: n for n in d["serializableNodes"]}
    port: dict = {}
    for n in d["serializableNodes"]:
        for pr in n.get("serializablePorts", []):
            if pr.get("sID"):
                port[pr["sID"]] = (
                    n["sID"],
                    n["id"],
                    pr.get("id"),
                    int(pr.get("polarity", 0)),
                )
    edges = []
    for c in d["serializableConnections"]:
        a, b = c.get("port0SID"), c.get("port1SID")
        if a not in port or b not in port:
            continue
        pa, pb = port[a][3], port[b][3]
        if pa != 0 and pb == 0:
            edges.append((a, b))
        elif pb != 0 and pa == 0:
            edges.append((b, a))
    cons: dict[str, list[str]] = defaultdict(list)
    for a, b in edges:
        cons[b].append(a)
    used_ports = {a for a, _ in edges} | {b for _, b in edges}

    floats = {
        n["sID"]: nf(n.get("modifier"))
        for n in nodes.values()
        if n["id"] == "Float"
    }

    def port_named(n, pid, pol):
        for pr in n.get("serializablePorts", []):
            if pr.get("id") == pid and int(pr.get("polarity", -1)) == pol:
                return pr.get("sID")
        return None

    def producer(cp):
        ps = cons.get(cp, [])
        return ps[0] if len(ps) == 1 else None

    print(
        f"\n==== {name} nodes={len(nodes)} "
        f"conns={len(d['serializableConnections'])} bytes={path.stat().st_size}"
    )
    print("types top:", Counter(n["id"] for n in nodes.values()).most_common(18))

    dn = 0
    for n in nodes.values():
        if n["id"] != "Not":
            continue
        inp = port_named(n, "Bool1", 0) or next(
            (
                pr["sID"]
                for pr in n["serializablePorts"]
                if int(pr.get("polarity", -1)) == 0
            ),
            None,
        )
        if not inp:
            continue
        pr = producer(inp)
        if pr and port[pr][1] == "Not":
            dn += 1
    print("Not->Not feeds", dn)

    for tid, lit, keep_ports in [
        ("MultiplyFloats", "1", ("Float1", "Float2")),
        ("DivideFloats", "1", ("Float1", "Float2")),
        ("AddFloats", "0", ("Float1", "Float2")),
        ("SubtractFloats", "0", ("Float1", "Float2")),
        ("ScaleVector3", "1", ("Float1",)),
    ]:
        hits = 0
        for n in nodes.values():
            if n["id"] != tid:
                continue
            for pname in keep_ports:
                pin = port_named(n, pname, 0)
                if not pin:
                    continue
                pr = producer(pin)
                if pr and port[pr][0] in floats and floats[port[pr][0]] == lit:
                    hits += 1
                    break
        tot = sum(1 for n in nodes.values() if n["id"] == tid)
        print(f"{tid} identity", hits, "total", tot)

    same = 0
    for n in nodes.values():
        if n["id"] not in ("MinFloats", "MaxFloats", "Min", "Max"):
            continue
        ins = []
        for pr in n.get("serializablePorts", []):
            if int(pr.get("polarity", -1)) == 0:
                p = producer(pr["sID"])
                if p:
                    ins.append(p)
        if len(ins) >= 2 and len(set(ins)) == 1:
            same += 1
    print("Min/Max same-input", same)

    sc = 0
    for n in nodes.values():
        if n["id"] != "ConstructVector3":
            continue
        feeds = {}
        for pr in n.get("serializablePorts", []):
            if int(pr.get("polarity", -1)) != 0:
                continue
            p = producer(pr["sID"])
            if p:
                feeds[pr.get("id")] = p
        if not {"Float1", "Float2", "Float3"} <= set(feeds):
            continue
        sids = []
        ok = True
        for pn, pp in feeds.items():
            ns, nid, pid, _ = port[pp]
            if nid != "Vector3Split" or pid != pn:
                ok = False
                break
            sids.append(ns)
        if ok and len(set(sids)) == 1:
            sc += 1
    print("split->construct", sc)
    print(
        "Relay",
        sum(1 for n in nodes.values() if n["id"] == "Relay"),
        "Region",
        sum(1 for n in nodes.values() if n["id"] == "Region"),
        "Set/Get",
        sum(1 for n in nodes.values() if n["id"] == "SetVariable"),
        sum(1 for n in nodes.values() if n["id"] == "GetVariable"),
    )

    orphan = Counter()
    for n in nodes.values():
        outs = [
            pr["sID"]
            for pr in n.get("serializablePorts", [])
            if pr.get("sID") and int(pr.get("polarity", -1)) == 1
        ]
        if not outs:
            continue
        if any(o in used_ports for o in outs):
            continue
        orphan[n["id"]] += 1
    print("orphan producers:", dict(orphan.most_common(20)))

    unused_f = 0
    for sid in floats:
        outs = [
            pr["sID"]
            for pr in nodes[sid].get("serializablePorts", [])
            if pr.get("sID") and int(pr.get("polarity", -1)) == 1
        ]
        if outs and not any(o in used_ports for o in outs):
            unused_f += 1
    print("unused Float constants", unused_f)

    # Bool const feeding Or/And identities
    or_absorb = and_absorb = 0
    for n in nodes.values():
        if n["id"] not in ("Or", "And"):
            continue
        for pr in n.get("serializablePorts", []):
            if int(pr.get("polarity", -1)) != 0:
                continue
            p = producer(pr["sID"])
            if not p:
                continue
            ns, nid, _, _ = port[p]
            if nid != "Bool":
                continue
            val = str(nodes[ns].get("modifier")).lower()
            if n["id"] == "Or" and val in ("true", "1"):
                or_absorb += 1
            if n["id"] == "And" and val in ("false", "0"):
                and_absorb += 1
    print("Or(true,x)/And(false,x) const feeds", or_absorb, and_absorb)

    # Vector ops: AddVectors with zero?
    zero_add = 0
    for n in nodes.values():
        if n["id"] not in ("AddVectors", "AddVector3", "SubtractVectors", "SubtractVector3"):
            continue
        for pr in n.get("serializablePorts", []):
            if int(pr.get("polarity", -1)) != 0:
                continue
            p = producer(pr["sID"])
            if not p:
                continue
            ns, nid, _, _ = port[p]
            if nid in ("Vector3", "Vector3Constant") and nf(nodes[ns].get("modifier")) in (
                "0",
                "(0, 0, 0)",
                "0,0,0",
            ):
                zero_add += 1
            # ConstructVector3 of three Float 0
            if nid == "ConstructVector3":
                zeros = 0
                for pr2 in nodes[ns].get("serializablePorts", []):
                    if int(pr2.get("polarity", -1)) != 0:
                        continue
                    p2 = producer(pr2["sID"])
                    if p2 and port[p2][0] in floats and floats[port[p2][0]] == "0":
                        zeros += 1
                if zeros == 3:
                    zero_add += 1
    print("vector +/- zero-ish", zero_add)

    # MultiplyVector / Scale with 0 (not identity but constant-foldable — note separately)
    for tid in (
        "Or",
        "And",
        "Xor",
        "If",
        "Select",
        "Lerp",
        "Clamp",
        "Remap",
        "Normalize",
        "Magnitude",
        "Dot",
        "Cross",
        "Distance",
        "Function",
        "CreateFunction",
        "Comment",
        "Debug",
        "TimePlot",
    ):
        c = sum(1 for n in nodes.values() if n["id"] == tid)
        if c:
            print(f"  {tid}: {c}")


def main() -> None:
    for name, path in FILES.items():
        if path.is_file():
            analyze(name, path)
        else:
            print("missing", path)


if __name__ == "__main__":
    main()
