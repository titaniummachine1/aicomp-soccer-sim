"""Generate src/graph/dropdowns.rs from AIGamePyLibrary DROPDOWN_OPTIONS."""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "worldcupteams" / "AIGamePyLibrary"))
from AIGamePyLibrary.data import DROPDOWN_OPTIONS  # noqa: E402

OUT = Path(__file__).resolve().parents[1] / "src" / "graph" / "dropdowns.rs"

KEYS = [
    ("SoccerGetBool", "SOCCER_GET_BOOL"),
    ("SoccerGetFloat", "SOCCER_GET_FLOAT"),
    ("SoccerGetTransform", "SOCCER_GET_TRANSFORM"),
    ("SoccerGetVector3", "SOCCER_GET_VECTOR3"),
]


def main() -> None:
    lines = [
        "//! Auto-generated SoccerGet* dropdown labels (index -> label).",
        "//! Source: AIGamePyLibrary.data.DROPDOWN_OPTIONS",
        "//! Regenerate: python scripts/gen_graph_dropdowns.py",
        "",
    ]
    for src, rust in KEYS:
        opts = DROPDOWN_OPTIONS[src]
        lines.append(f"pub const {rust}: &[&str] = &[")
        for o in opts:
            lines.append(f"    {json.dumps(o)},")
        lines.append("];")
        lines.append("")

    lines += [
        "pub fn resolve(node_id: &str, modifier: &str) -> &str {",
        "    let opts: &[&str] = match node_id {",
        '        "SoccerGetBool" => SOCCER_GET_BOOL,',
        '        "SoccerGetFloat" => SOCCER_GET_FLOAT,',
        '        "SoccerGetTransform" => SOCCER_GET_TRANSFORM,',
        '        "SoccerGetVector3" => SOCCER_GET_VECTOR3,',
        "        _ => return modifier,",
        "    };",
        "    if let Ok(i) = modifier.parse::<usize>() {",
        "        if let Some(label) = opts.get(i) {",
        "            return label;",
        "        }",
        "    }",
        "    modifier",
        "}",
        "",
    ]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
