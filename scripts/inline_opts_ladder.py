#!/usr/bin/env python3
"""Apply extra inline opts one-by-one with parity_check after each."""
from __future__ import annotations

import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

sys.path.insert(0, str(Path(r"C:\gitProjects\worldcup\worldcupteams\AIGamePyLibrary")))
from AIGamePyLibrary import lib  # noqa: E402

SIM = Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim")
OUT = SIM / "data" / "titanium" / "compacted" / "opt_ladder"
SAVES = Path.home() / "AppData" / "LocalLow" / "Unicorn One" / "AIComp" / "Saves" / "Soccer"


def load_raw(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def set_graph(doc: dict) -> None:
    lib.data["serializableNodes"][:] = deepcopy(doc["serializableNodes"])
    lib.data["serializableConnections"][:] = deepcopy(doc["serializableConnections"])


def snapshot() -> dict:
    return {
        "serializableNodes": deepcopy(lib.data["serializableNodes"]),
        "serializableConnections": deepcopy(lib.data["serializableConnections"]),
    }


def save(doc: dict, path: Path) -> int:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, separators=(",", ":"), ensure_ascii=False), encoding="utf-8")
    return path.stat().st_size


def parity(a: Path, b: Path, secs: int = 60) -> tuple[bool, str]:
    cmd = [
        "cargo", "run", "--release", "--bin", "parity_check", "--",
        "--a", f"graph:{a}",
        "--b", f"graph:{b}",
        "--away", "idle",
        "--secs", str(secs),
        "--seed", "12345",
        "--seeds", "1",
    ]
    done = subprocess.run(cmd, cwd=str(SIM), capture_output=True, text=True)
    text = (done.stdout or "") + (done.stderr or "")
    ok = done.returncode == 0 and "RESULT: equivalent" in text
    lines = [
        ln for ln in text.splitlines()
        if any(k in ln for k in ("IDENTICAL", "DIFF", "RESULT", "error", "Error", "panic"))
    ]
    return ok, "\n".join(lines[-8:]) or text[-500:]


def _port_maps():
    polarity = {}
    node_of = {}
    ports_by_node = {}
    for n in lib.data["serializableNodes"]:
        ports_by_node[n["sID"]] = n.get("serializablePorts", [])
        for p in n.get("serializablePorts", []):
            if not p.get("sID"):
                continue
            polarity[p["sID"]] = int(p.get("polarity", 0))
            node_of[p["sID"]] = n["sID"]
    return polarity, node_of, ports_by_node


def _edges(polarity):
    """Return list of (producer_port, consumer_port)."""
    out = []
    for c in lib.data["serializableConnections"]:
        a, b = c.get("port0SID"), c.get("port1SID")
        if not a or not b or a not in polarity or b not in polarity:
            continue
        pa, pb = polarity[a], polarity[b]
        if pa != 0 and pb == 0:
            out.append((a, b))
        elif pb != 0 and pa == 0:
            out.append((b, a))
    return out


def _norm_float(mod) -> str:
    s = str(mod).replace(",", ".").strip()
    try:
        return f"{float(s):.6g}"
    except Exception:
        return s


def _bypass_node(node_sid: str, in_port: str, out_port: str) -> bool:
    """Rewire producer-of(in_port) to consumers-of(out_port); drop node."""
    polarity, node_of, _ = _port_maps()
    edges = _edges(polarity)
    producers = [a for a, b in edges if b == in_port]
    consumers = [b for a, b in edges if a == out_port]
    if len(producers) != 1 or not consumers:
        return False
    prod = producers[0]
    dead = {in_port, out_port}
    # also kill other ports on this node
    for n in lib.data["serializableNodes"]:
        if n["sID"] == node_sid:
            for p in n.get("serializablePorts", []):
                if p.get("sID"):
                    dead.add(p["sID"])
            break
    new_conns = []
    seen = set()
    for c in lib.data["serializableConnections"]:
        a, b = c.get("port0SID"), c.get("port1SID")
        if a in dead or b in dead:
            continue
        key = (a, b)
        if key in seen:
            continue
        seen.add(key)
        new_conns.append(c)
    for cons in consumers:
        key = (prod, cons)
        if key in seen or prod == cons:
            continue
        seen.add(key)
        new_conns.append(
            {
                "sID": lib.generateId(),
                "port0SID": prod,
                "port1SID": cons,
                "port0InstanceID": 0,
                "port1InstanceID": 0,
            }
        )
    lib.data["serializableConnections"] = new_conns
    lib.data["serializableNodes"] = [
        n for n in lib.data["serializableNodes"] if n["sID"] != node_sid
    ]
    return True


def opt_double_not() -> int:
    """Not(Not(x)) -> x for pairs where inner Not has only the outer as consumer."""
    changed = 0
    # iterate to fixpoint
    while True:
        polarity, node_of, ports_by_node = _port_maps()
        edges = _edges(polarity)
        nodes = {n["sID"]: n for n in lib.data["serializableNodes"]}
        did = False
        for n2 in list(lib.data["serializableNodes"]):
            if n2.get("id") != "Not":
                continue
            in2 = next((p["sID"] for p in n2["serializablePorts"] if p.get("polarity") == 0), None)
            out2 = next((p["sID"] for p in n2["serializablePorts"] if p.get("polarity") == 1), None)
            if not in2 or not out2:
                continue
            prods = [a for a, b in edges if b == in2]
            if len(prods) != 1:
                continue
            n1_sid = node_of.get(prods[0])
            n1 = nodes.get(n1_sid)
            if not n1 or n1.get("id") != "Not":
                continue
            # inner Not output should be prods[0]
            out1 = prods[0]
            # if inner has other consumers besides n2, only bypass this path:
            # wire n1's input to n2's consumers; remove only n2; leave n1 if still used
            in1 = next((p["sID"] for p in n1["serializablePorts"] if p.get("polarity") == 0), None)
            if not in1:
                continue
            inner_prods = [a for a, b in edges if b == in1]
            if len(inner_prods) != 1:
                continue
            src = inner_prods[0]
            consumers = [b for a, b in edges if a == out2]
            if not consumers:
                continue
            # remove n2 edges and node; connect src -> consumers
            dead = {p["sID"] for p in n2["serializablePorts"] if p.get("sID")}
            new_conns = []
            seen = set()
            for c in lib.data["serializableConnections"]:
                a, b = c.get("port0SID"), c.get("port1SID")
                if a in dead or b in dead:
                    continue
                key = (a, b)
                if key in seen:
                    continue
                seen.add(key)
                new_conns.append(c)
            for cons in consumers:
                key = (src, cons)
                if key in seen or src == cons:
                    continue
                seen.add(key)
                new_conns.append(
                    {
                        "sID": lib.generateId(),
                        "port0SID": src,
                        "port1SID": cons,
                        "port0InstanceID": 0,
                        "port1InstanceID": 0,
                    }
                )
            lib.data["serializableConnections"] = new_conns
            lib.data["serializableNodes"] = [
                n for n in lib.data["serializableNodes"] if n["sID"] != n2["sID"]
            ]
            # drop inner Not if unused
            polarity2, _, _ = _port_maps()
            edges2 = _edges(polarity2)
            if not any(a == out1 for a, _ in edges2):
                dead1 = {p["sID"] for p in n1["serializablePorts"] if p.get("sID")}
                lib.data["serializableConnections"] = [
                    c for c in lib.data["serializableConnections"]
                    if c.get("port0SID") not in dead1 and c.get("port1SID") not in dead1
                ]
                lib.data["serializableNodes"] = [
                    n for n in lib.data["serializableNodes"] if n["sID"] != n1["sID"]
                ]
            changed += 1
            did = True
            break  # restart scan
        if not did:
            break
    return changed


def opt_identity_math() -> int:
    """Bypass MultiplyFloats(*1), ScaleVector3(*1), AddFloats(+0)."""
    changed = 0
    while True:
        polarity, node_of, _ = _port_maps()
        edges = _edges(polarity)
        float_val = {
            n["sID"]: _norm_float(n.get("modifier"))
            for n in lib.data["serializableNodes"]
            if n.get("id") == "Float"
        }

        def port_named(node, name, pol):
            for p in node.get("serializablePorts", []):
                if p.get("id") == name and int(p.get("polarity", -1)) == pol:
                    return p.get("sID")
            return None

        def producer_of(consumer_port):
            ps = [a for a, b in edges if b == consumer_port]
            return ps[0] if len(ps) == 1 else None

        did = False
        for n in list(lib.data["serializableNodes"]):
            tid = n.get("id")
            target = None
            if tid == "ScaleVector3":
                f_in = port_named(n, "Float1", 0)
                v_in = port_named(n, "Vector31", 0)
                v_out = port_named(n, "Vector31", 1)
                if f_in and v_in and v_out:
                    fp = producer_of(f_in)
                    if fp and node_of.get(fp) in float_val and float_val[node_of[fp]] == "1":
                        target = (n["sID"], v_in, v_out)
            elif tid == "MultiplyFloats":
                a_in = port_named(n, "Float1", 0)
                b_in = port_named(n, "Float2", 0)
                out = port_named(n, "Float1", 1)
                if a_in and b_in and out:
                    pa, pb = producer_of(a_in), producer_of(b_in)
                    keep = None
                    if pa and node_of.get(pa) in float_val and float_val[node_of[pa]] == "1":
                        keep = b_in
                    elif pb and node_of.get(pb) in float_val and float_val[node_of[pb]] == "1":
                        keep = a_in
                    if keep:
                        target = (n["sID"], keep, out)
            elif tid == "AddFloats":
                a_in = port_named(n, "Float1", 0)
                b_in = port_named(n, "Float2", 0)
                out = port_named(n, "Float1", 1)
                if a_in and b_in and out:
                    pa, pb = producer_of(a_in), producer_of(b_in)
                    keep = None
                    if pa and node_of.get(pa) in float_val and float_val[node_of[pa]] == "0":
                        keep = b_in
                    elif pb and node_of.get(pb) in float_val and float_val[node_of[pb]] == "0":
                        keep = a_in
                    if keep:
                        target = (n["sID"], keep, out)
            if target and _bypass_node(*target):
                changed += 1
                did = True
                break
        if not did:
            break
    return changed


def opt_split_construct() -> int:
    """ConstructVector3(split.x, split.y, split.z) from same Vector3Split -> use original vector."""
    changed = 0
    while True:
        polarity, node_of, _ = _port_maps()
        edges = _edges(polarity)
        nodes = {n["sID"]: n for n in lib.data["serializableNodes"]}
        did = False
        for n in list(lib.data["serializableNodes"]):
            if n.get("id") != "ConstructVector3":
                continue
            # map input port name -> producer port
            feeds = {}
            for p in n.get("serializablePorts", []):
                if int(p.get("polarity", -1)) != 0:
                    continue
                ps = [a for a, b in edges if b == p["sID"]]
                if len(ps) == 1:
                    feeds[p.get("id")] = ps[0]
            if not {"Float1", "Float2", "Float3"} <= set(feeds):
                continue
            # each producer should be an output of the same Vector3Split
            split_ids = []
            for pname, prod in feeds.items():
                ns = node_of.get(prod)
                sn = nodes.get(ns)
                if not sn or sn.get("id") != "Vector3Split":
                    split_ids = []
                    break
                # expected port: Float1/Float2/Float3 outputs
                pr = next((pp for pp in sn["serializablePorts"] if pp.get("sID") == prod), None)
                if not pr or pr.get("id") not in ("Float1", "Float2", "Float3"):
                    split_ids = []
                    break
                if pname != pr.get("id"):
                    # Construct Float1 should come from Split Float1 etc.
                    split_ids = []
                    break
                split_ids.append(ns)
            if len(split_ids) != 3 or len(set(split_ids)) != 1:
                continue
            split = nodes[split_ids[0]]
            vin = next((p["sID"] for p in split["serializablePorts"] if p.get("id") == "Vector31" and int(p.get("polarity", -1)) == 0), None)
            vout_c = next((p["sID"] for p in n["serializablePorts"] if p.get("id") == "Vector31" and int(p.get("polarity", -1)) == 1), None)
            if not vin or not vout_c:
                continue
            # producer of split input
            sprods = [a for a, b in edges if b == vin]
            if len(sprods) != 1:
                continue
            src = sprods[0]
            consumers = [b for a, b in edges if a == vout_c]
            if not consumers:
                continue
            # remove construct; wire src -> consumers
            dead = {p["sID"] for p in n["serializablePorts"] if p.get("sID")}
            new_conns = []
            seen = set()
            for c in lib.data["serializableConnections"]:
                a, b = c.get("port0SID"), c.get("port1SID")
                if a in dead or b in dead:
                    continue
                key = (a, b)
                if key in seen:
                    continue
                seen.add(key)
                new_conns.append(c)
            for cons in consumers:
                key = (src, cons)
                if key in seen or src == cons:
                    continue
                seen.add(key)
                new_conns.append(
                    {
                        "sID": lib.generateId(),
                        "port0SID": src,
                        "port1SID": cons,
                        "port0InstanceID": 0,
                        "port1InstanceID": 0,
                    }
                )
            lib.data["serializableConnections"] = new_conns
            lib.data["serializableNodes"] = [
                x for x in lib.data["serializableNodes"] if x["sID"] != n["sID"]
            ]
            # drop split if unused
            polarity2, _, _ = _port_maps()
            edges2 = _edges(polarity2)
            split_outs = {
                p["sID"] for p in split["serializablePorts"]
                if p.get("sID") and int(p.get("polarity", -1)) == 1
            }
            if not any(a in split_outs for a, _ in edges2):
                dead_s = {p["sID"] for p in split["serializablePorts"] if p.get("sID")}
                lib.data["serializableConnections"] = [
                    c for c in lib.data["serializableConnections"]
                    if c.get("port0SID") not in dead_s and c.get("port1SID") not in dead_s
                ]
                lib.data["serializableNodes"] = [
                    x for x in lib.data["serializableNodes"] if x["sID"] != split["sID"]
                ]
            changed += 1
            did = True
            break
        if not did:
            break
    return changed


def opt_strip_debug() -> int:
    return lib.stripDebugSinks(verbose=False)


def prepare_base(src: Path) -> dict:
    """Current leaner+remap baseline (no new opts)."""
    raw = load_raw(src)
    set_graph(raw)
    # mimic OptimizeFile(normal, no prune, leaner, remap) but WITHOUT calling
    # the new opts if we temporarily remove them — instead call pieces manually.
    lib._prepare_for_unity_format(leaner=False, remap_sids=False)
    # manual leaner pieces except we'll call strip_leaner which includes vars/relays/regions
    # For ladder, base = current full leaner (including var inline already in lib)
    set_graph(raw)
    lib._prepare_for_unity_format(leaner=True, remap_sids=True)
    return snapshot()


def run_bot(name: str, src: Path) -> None:
    print(f"\n######## {name} ########")
    OUT.mkdir(parents=True, exist_ok=True)
    base_doc = prepare_base(src)
    base_path = OUT / f"{name}__L0_base.txt"
    save(base_doc, base_path)
    print(f"L0 base nodes={len(base_doc['serializableNodes'])} bytes={base_path.stat().st_size}")

    steps = [
        ("L1_double_not", opt_double_not),
        ("L2_identity_math", opt_identity_math),
        ("L3_split_construct", opt_split_construct),
        ("L4_strip_debug_EXPECT", opt_strip_debug),
    ]
    # each step starts from previous PASS doc
    current = base_doc
    current_path = base_path
    for label, fn in steps:
        set_graph(current)
        n = fn()
        doc = snapshot()
        # chrome already applied in base; after structural edits re-slim connections
        set_graph(doc)
        lib._strip_leaner_fields.__wrapped__ if False else None
        # re-run only chrome tail + remap? IDs already short. Just dump.
        path = OUT / f"{name}__{label}.txt"
        # apply remap again? nodes may have new generateId uuids from rewires — remap for size
        set_graph(doc)
        lib.remapSids(verbose=False)
        # drop chrome fields again on any new conn dicts
        for conn in lib.data["serializableConnections"]:
            conn.pop("sID", None)
            conn.pop("port0InstanceID", None)
            conn.pop("port1InstanceID", None)
        doc = snapshot()
        save(doc, path)
        ok, summary = parity(src, path, secs=60)
        print(f"\n== {label} == changed={n} nodes={len(doc['serializableNodes'])} bytes={path.stat().st_size}")
        print(summary)
        print("PASS" if ok else "FAIL")
        if ok:
            current = doc
            current_path = path
        else:
            print(f"(kept previous good: {current_path.name})")
            current = load_raw(current_path)

    # deploy last good for AIA bots
    if name in ("AIA", "AIA3", "Titanium"):
        dest1 = SAVES / f"{name}_compacted.txt"
        dest2 = SAVES / f"{name}_leaner_remap.txt"
        data = current_path.read_bytes()
        dest1.write_bytes(data)
        dest2.write_bytes(data)
        print(f"deployed last-good -> {dest1.name} ({dest1.stat().st_size} bytes)")


def main() -> int:
    bots = [
        ("AIA", SAVES / "AIA.txt"),
        ("AIA3", SAVES / "AIA3.txt"),
        ("Titanium", SIM / "data" / "titanium" / "Titanium.txt"),
    ]
    for name, src in bots:
        if not src.is_file():
            print("missing", src)
            continue
        run_bot(name, src)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
