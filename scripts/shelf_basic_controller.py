"""Rebuild PerfectController from Downloads/basic controller.txt — shelf P2-4 only."""
from __future__ import annotations

import json
import uuid
from pathlib import Path

SRC = Path(r"C:\Users\Terminatort8000\Downloads\basic controller.txt")
SAVES = (
    Path.home()
    / "AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/PerfectController.txt"
)
OUT_DL = Path(r"C:\Users\Terminatort8000\Downloads\PerfectController.txt")


def port(n, pid, pol=None):
    for pt in n["serializablePorts"]:
        if pt.get("id") != pid:
            continue
        if pol is not None and pt.get("polarity") != pol:
            continue
        return pt
    raise KeyError(pid)


def main() -> None:
    raw = json.loads(SRC.read_text(encoding="utf-8"))
    nodes = {n["id"] + "|" + n.get("modifier", ""): n for n in raw["serializableNodes"]}
    by_id = {}
    for n in raw["serializableNodes"]:
        by_id.setdefault(n["id"], []).append(n)

    for n in raw["serializableNodes"]:
        if n["id"] == "SoccerGetTransform" and n.get("modifier") == "Team Player 3":
            n["modifier"] = "Team Player 1"

    corner = next(
        n
        for n in raw["serializableNodes"]
        if n["id"] == "SoccerGetVector3" and "Corner" in n.get("modifier", "")
    )
    corner["modifier"] = "Upper Corner Team Side"
    corner_out = port(corner, "Vector31", 1)["sID"]

    c1 = by_id["SoccerController1"][0]
    c3 = by_id["SoccerController3"][0]
    add = by_id["AddVector3"][0]
    shift = next(n for n in by_id["Keypress"] if n["modifier"] == "LeftShift")
    e = next(n for n in by_id["Keypress"] if n["modifier"] == "E")

    c1_move, c1_sprint, c1_int = (
        port(c1, "Vector31", 0)["sID"],
        port(c1, "Bool1", 0)["sID"],
        port(c1, "Bool2", 0)["sID"],
    )
    add_out = port(add, "Vector31", 1)["sID"]
    shift_out = port(shift, "Bool1", 1)["sID"]
    e_out = port(e, "Bool1", 1)["sID"]

    def drop(a, b, conns):
        return [
            c
            for c in conns
            if not (
                (c["port0SID"] == a and c["port1SID"] == b)
                or (c["port0SID"] == b and c["port1SID"] == a)
            )
        ]

    conns = raw["serializableConnections"]
    for n in raw["serializableNodes"]:
        if n["id"].startswith("SoccerController"):
            mv = port(n, "Vector31", 0)["sID"]
            conns = drop(mv, corner_out, conns)
            conns = drop(mv, add_out, conns)
        if n["id"] == "SoccerController3":
            conns = drop(port(n, "Bool1", 0)["sID"], shift_out, conns)
            conns = drop(port(n, "Bool2", 0)["sID"], e_out, conns)
    conns = drop(c1_sprint, shift_out, conns)
    conns = drop(c1_int, e_out, conns)

    def link(a, b, label):
        conns.append(
            {
                "id": f"Connection ({label})",
                "sID": str(uuid.uuid4()),
                "port0InstanceID": 0,
                "port1InstanceID": 0,
                "port0SID": a,
                "port1SID": b,
                "selectedColor": {"r": 1, "g": 0.58, "b": 0.04, "a": 1},
                "hoverColor": {"r": 1, "g": 0.81, "b": 0.3, "a": 1},
                "defaultColor": {"r": 0.98, "g": 0.94, "b": 0.84, "a": 1},
                "curveStyle": 3,
                "label": "",
                "line": {
                    "capStart": {"active": False, "shape": 3, "size": 5, "angleOffset": 0},
                    "capEnd": {"active": False, "shape": 3, "size": 5, "angleOffset": 0},
                    "ID": "",
                    "startWidth": 3,
                    "endWidth": 3,
                    "dashDistance": 5,
                    "points": [],
                    "lineStyle": 0,
                    "length": 0,
                    "animation": {
                        "isActive": False,
                        "pointsDistance": 90,
                        "size": 10,
                        "color": {"r": 0.3, "g": 0.89, "b": 0.3, "a": 1},
                        "shape": 1,
                        "speed": 0,
                    },
                },
                "enableDrag": True,
                "enableHover": True,
                "enableSelect": True,
                "disableClick": False,
            }
        )

    link(add_out, c1_move, "move->P1")
    link(shift_out, c1_sprint, "sprint->P1")
    link(e_out, c1_int, "interact->P1")
    for cid in ("SoccerController2", "SoccerController3", "SoccerController4"):
        link(corner_out, port(by_id[cid][0], "Vector31", 0)["sID"], f"park->{cid}")

    raw["serializableConnections"] = conns
    text = json.dumps(raw, separators=(",", ":"))
    SAVES.parent.mkdir(parents=True, exist_ok=True)
    SAVES.write_text(text, encoding="utf-8")
    OUT_DL.write_text(text, encoding="utf-8")
    print(f"Wrote {SAVES}")
    print(f"Wrote {OUT_DL}")
    print("WASD kept from basic controller (W/S=Z, A/D=X). P2-4 shelved in corner.")


if __name__ == "__main__":
    main()
