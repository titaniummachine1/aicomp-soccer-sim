#!/usr/bin/env python3
"""Analyze kart-game Computer graphs for original latch invention pattern."""
from __future__ import annotations

import json
import re
from collections import Counter, defaultdict
from pathlib import Path

PATHS = [
    Path(r"c:\Users\Terminatort8000\Downloads\Computer basics.txt"),
    Path(r"c:\Users\Terminatort8000\Downloads\Computer.txt"),
]


def load(path: Path):
    raw = path.read_text(encoding="utf-8")
    try:
        return json.loads(raw), "json"
    except json.JSONDecodeError:
        fixed = re.sub(r"(?<=\d),(?=\d)", ".", raw)
        try:
            return json.loads(fixed), "json-eu"
        except json.JSONDecodeError:
            return raw, "text"


def analyze_graph(name: str, d: dict) -> None:
    nodes = {n["sID"]: n for n in d["serializableNodes"]}
    print(f"\n######## {name} ########")
    print("nodes", len(nodes), "conns", len(d.get("serializableConnections", [])))
    print("types", Counter(n["id"] for n in nodes.values()).most_common(25))

    port = {}
    for n in nodes.values():
        for p in n.get("serializablePorts", []):
            if p.get("sID"):
                port[p["sID"]] = (
                    n["sID"],
                    n["id"],
                    p.get("id"),
                    int(p.get("polarity", 0)),
                    str(n.get("modifier", ""))[:60],
                )

    directed = defaultdict(set)
    edges = []  # (prod, cons) port infos
    for c in d["serializableConnections"]:
        a, b = c.get("port0SID"), c.get("port1SID")
        if a not in port or b not in port:
            continue
        pa, pb = port[a], port[b]
        if pa[3] == 1 and pb[3] == 0:
            directed[pa[0]].add(pb[0])
            edges.append((pa, pb))
        elif pb[3] == 1 and pa[3] == 0:
            directed[pb[0]].add(pa[0])
            edges.append((pb, pa))
        elif pa[3] == 2 or pb[3] == 2:
            directed[pa[0]].add(pb[0])
            directed[pb[0]].add(pa[0])

    # Strings / debug labels
    for n in nodes.values():
        if n["id"] in ("String", "Debug", "Stat"):
            print(f"  {n['id']}: {repr(n.get('modifier'))[:120]}")

    csf_ids = [
        s
        for s, n in nodes.items()
        if n["id"] in ("ConditionalSetFloat", "ConditionalSetFloatV2", "ConditionalSetBool")
    ]
    print("ConditionalSet* count", len(csf_ids))

    for sid in csf_ids:
        n = nodes[sid]
        print(f"\n  CSF {sid[:8]} id={n['id']} mod={repr(n.get('modifier'))}")
        print("   ports", [(p.get("id"), p.get("polarity")) for p in n["serializablePorts"]])
        ins = {
            p.get("id"): p["sID"]
            for p in n["serializablePorts"]
            if int(p.get("polarity", -1)) == 0 and p.get("sID")
        }
        outs = [
            p["sID"]
            for p in n["serializablePorts"]
            if int(p.get("polarity", -1)) == 1 and p.get("sID")
        ]
        for prod, cons in edges:
            if cons[0] == sid:
                print(
                    f"   IN  {cons[2]} <- {prod[1]}.{prod[2]} ({prod[0][:8]}) mod={prod[4]!r}"
                )
            if prod[0] == sid:
                print(
                    f"   OUT {prod[2]} -> {cons[1]}.{cons[2]} ({cons[0][:8]})"
                )
        # direct self wire?
        for c in d["serializableConnections"]:
            a, b = c.get("port0SID"), c.get("port1SID")
            if a in outs and b in ins.values():
                pid = [k for k, v in ins.items() if v == b][0]
                print(f"   *** DIRECT SELF out -> {pid}")
            if b in outs and a in ins.values():
                pid = [k for k, v in ins.items() if v == a][0]
                print(f"   *** DIRECT SELF out -> {pid}")

    def find_cycle(start, maxlen=10):
        stack = [(start, [start])]
        seen = set()
        while stack:
            cur, path = stack.pop()
            for nxt in directed[cur]:
                if nxt in path:
                    return path[path.index(nxt) :] + [nxt]
                key = (cur, nxt)
                if key not in seen and len(path) < maxlen:
                    seen.add(key)
                    stack.append((nxt, path + [nxt]))
        return None

    print("\n  cycles touching ConditionalSet*:")
    seen_types = set()
    for sid in csf_ids:
        cyc = find_cycle(sid)
        if not cyc:
            continue
        types = tuple(nodes[s]["id"] for s in cyc)
        if types in seen_types:
            continue
        seen_types.add(types)
        print("   ", " -> ".join(types))


def main() -> None:
    for path in PATHS:
        if not path.is_file():
            print("missing", path)
            continue
        print(f"\nfile {path.name} bytes={path.stat().st_size}")
        data, kind = load(path)
        if kind == "text":
            text = data if isinstance(data, str) else ""
            print("TEXT file, first 3000 chars:\n")
            print(text[:3000])
            # also search keywords
            for kw in ("latch", "Latch", "memory", "Conditional", "Relay", "feedback", "loop"):
                if kw.lower() in text.lower():
                    print(f"  contains keyword: {kw}")
        else:
            analyze_graph(path.name, data)


if __name__ == "__main__":
    main()
