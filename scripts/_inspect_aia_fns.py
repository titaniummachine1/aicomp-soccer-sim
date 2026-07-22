import json
from pathlib import Path

p = Path.home() / r"AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/AIA.txt"
d = json.loads(p.read_text(encoding="utf-8"))
nodes = {n["sID"]: n for n in d["serializableNodes"]}
ports = {}
for n in d["serializableNodes"]:
    for pp in n["serializablePorts"]:
        ports[pp["sID"]] = (n["sID"], pp["id"], pp["polarity"])

cfs = [n for n in d["serializableNodes"] if n["id"] == "CreateFunction"]
print("CreateFunctions:")
for cf in cfs:
    owned = [n for n in d["serializableNodes"] if n.get("ownerFunctionSID") == cf["sID"]]
    types = sorted({x["id"] for x in owned})
    print(f"  {cf['modifier']!r} sid={cf['sID'][:8]} owned={len(owned)} types={types[:15]}")

fns = [n for n in d["serializableNodes"] if n["id"] == "Function"]
print("Function calls", len(fns), "names", sorted({n["modifier"] for n in fns}))

sets = [n for n in d["serializableNodes"] if n["id"] == "SetVariable"]
print("SetVariable count", len(sets))
print("names", [n["modifier"] for n in sets])

for cf in cfs:
    ret = next(pp for pp in cf["serializablePorts"] if pp["id"] == "Any1" and pp["polarity"] == 0)
    for c in d["serializableConnections"]:
        if c["port1SID"] == ret["sID"] or c["port0SID"] == ret["sID"]:
            other = c["port0SID"] if c["port1SID"] == ret["sID"] else c["port1SID"]
            if other not in ports:
                continue
            ns, pid, pol = ports[other]
            print(
                "  return of",
                repr(cf["modifier"]),
                "fed by",
                nodes[ns]["id"] + "." + pid,
                "pol",
                pol,
            )
