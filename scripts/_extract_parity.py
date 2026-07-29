import json
from pathlib import Path

p = Path(
    r"C:\Users\Terminatort8000\.cursor\projects\c-gitProjects-worldcup\agent-transcripts"
    r"\e564e6e6-25f7-4ec4-a744-d5929b7883bf\e564e6e6-25f7-4ec4-a744-d5929b7883bf.jsonl"
)
out = Path(
    r"C:\Users\Terminatort8000\.cursor\projects\c-gitProjects-worldcup\agent-tools"
    r"\parity_extract.txt"
)
needles = (
    "passed parity",
    "======== SUMMARY",
    "PARITY idle:",
    "PARITY vs Haialand:",
    "every Titanium version",
    "promotion target slate",
    "Zudan",
)
chunks = []
for i, line in enumerate(p.open(encoding="utf-8"), 1):
    if not any(n in line for n in needles):
        continue
    try:
        o = json.loads(line)
    except Exception:
        continue
    content = o.get("message", {}).get("content")
    if not isinstance(content, list):
        continue
    for c in content:
        if not isinstance(c, dict) or c.get("type") != "text":
            continue
        t = c.get("text") or ""
        if any(n in t for n in needles):
            chunks.append(f"==== HIT line {i} role={o.get('role')} ====\n{t[:6000]}")

# Also grab shell tool results that might be stored as assistant text after 1046
for i, line in enumerate(p.open(encoding="utf-8"), 1):
    if i < 1046 or i > 1052:
        continue
    try:
        o = json.loads(line)
    except Exception:
        continue
    content = o.get("message", {}).get("content")
    if not isinstance(content, list):
        continue
    texts = []
    for c in content:
        if isinstance(c, dict) and c.get("type") == "text":
            texts.append(c.get("text") or "")
    if texts:
        chunks.append(f"==== WINDOW {i} ====\n" + "\n".join(texts)[:8000])

out.write_text("\n\n".join(chunks), encoding="utf-8")
print(f"wrote {out} chunks={len(chunks)}")
