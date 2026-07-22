"""Diff Unity dropdown catalog vs wired sim API labels."""
from __future__ import annotations

import re
from pathlib import Path

root = Path(__file__).resolve().parents[1]
dd = (root / "src/graph/dropdowns.rs").read_text(encoding="utf-8")
labels = (root / "src/api/labels.rs").read_text(encoding="utf-8")
snap = (root / "src/api/snapshot.rs").read_text(encoding="utf-8")


def extract_array(src: str, name: str) -> list[str]:
    m = re.search(rf"pub const {name}: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not m:
        return []
    return re.findall(r'"([^"]+)"', m.group(1))


cats = [
    ("BOOL", "SOCCER_GET_BOOL", "GET_BOOL"),
    ("FLOAT", "SOCCER_GET_FLOAT", "GET_FLOAT"),
    ("TRANSFORM", "SOCCER_GET_TRANSFORM", "GET_TRANSFORM"),
    ("VECTOR3", "SOCCER_GET_VECTOR3", "GET_VECTOR3"),
]
field = set(extract_array(labels, "GET_FLOAT_FIELD_MARKS"))

print("=== DROPDOWN not in labels.rs ===")
for kind, dd_name, lab_name in cats:
    dd_set = set(extract_array(dd, dd_name))
    lab_set = set(extract_array(labels, lab_name))
    if kind == "FLOAT":
        lab_set |= field
    missing = sorted(dd_set - lab_set)
    print(f"\n[{kind}] {len(missing)} missing:")
    for x in missing:
        status = "in_snapshot" if f'"{x}"' in snap else "BLANK"
        print(f"  {status:12} {x}")
