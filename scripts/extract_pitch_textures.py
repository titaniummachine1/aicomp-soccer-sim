"""List / extract Soccer pitch textures from AIComp Unity assets via UnityPy."""

from __future__ import annotations

import argparse
from pathlib import Path

import UnityPy

DATA = Path(r"C:\gitProjects\worldcup\worldcupv0.5\Aialanders_Data")
OUT = Path(r"C:\gitProjects\worldcup\aicomp-soccer-sim\assets\from_game")

ASSET_FILES = [
    DATA / "sharedassets0.assets",
    DATA / "resources.assets",
    DATA / "globalgamemanagers.assets",
]

KEYS = (
    "grass",
    "pitch",
    "field",
    "goal",
    "line",
    "stripe",
    "soccer",
    "turf",
    "net",
    "post",
    "mark",
    "ground",
    "stadium",
    "court",
    "football",
    "lawn",
    "yard",
    "arena",
)


def iter_textures(path: Path):
    env = UnityPy.load(str(path))
    for obj in env.objects:
        if obj.type.name != "Texture2D":
            continue
        try:
            data = obj.read()
        except Exception:
            continue
        name = getattr(data, "m_Name", None) or getattr(data, "name", "") or ""
        yield obj, data, name


def list_hits() -> None:
    for root in ASSET_FILES:
        if not root.exists():
            print(f"missing {root}")
            continue
        print(f"\n=== {root.name} ===")
        hits = []
        for obj, data, name in iter_textures(root):
            low = name.lower()
            if any(k in low for k in KEYS):
                w = getattr(data, "m_Width", None) or "?"
                h = getattr(data, "m_Height", None) or "?"
                hits.append((name, w, h, obj.path_id))
        hits.sort(key=lambda x: x[0].lower())
        for name, w, h, pid in hits:
            print(f"  {name:50s} {w}x{h}  path_id={pid}")
        print(f"  total hits: {len(hits)}")


def extract(substring_filters: list[str] | None = None) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    filters = [f.lower() for f in (substring_filters or KEYS)]
    saved = 0
    for root in ASSET_FILES:
        if not root.exists():
            continue
        for obj, data, name in iter_textures(root):
            low = name.lower()
            if not any(k in low for k in filters):
                continue
            try:
                img = data.image
            except Exception as e:
                print(f"  skip {name}: {e}")
                continue
            safe = "".join(c if c.isalnum() or c in "-_" else "_" for c in name)
            out = OUT / f"{safe}__{root.stem}__{obj.path_id}.png"
            img.save(out)
            print(f"saved {out.name}  {img.size}")
            saved += 1
    print(f"\nextracted {saved} textures -> {OUT}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["list", "extract"], nargs="?", default="list")
    ap.add_argument(
        "--filter",
        action="append",
        default=None,
        help="substring filter (repeatable). default=pitch keywords",
    )
    args = ap.parse_args()
    if args.cmd == "list":
        list_hits()
    else:
        extract(args.filter)


if __name__ == "__main__":
    main()
