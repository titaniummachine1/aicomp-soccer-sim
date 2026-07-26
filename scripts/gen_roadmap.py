"""Regenerate ROADMAP.md from the code, so it cannot drift.

A hand-written roadmap is stale the day after it is written. This one is
derived: fidelity grades come from `cargo run --bin fidelity_report`, open work
comes from the GitHub issue list, so the only way to change the roadmap is to
change the thing it describes.

    python scripts/gen_roadmap.py            # rewrite ROADMAP.md
    python scripts/gen_roadmap.py --check    # non-zero if it is out of date (CI)
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "ROADMAP.md"
REPO = "titaniummachine1/aicomp-soccer-sim"


def fidelity_rows():
    out = subprocess.run(["cargo", "run", "--release", "--quiet", "--bin", "fidelity_report"],
                         capture_output=True, text=True, cwd=str(ROOT))
    rows, current = [], None
    for line in out.stdout.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if "(" in line and line.startswith("  ") and not line.startswith("      "):
            parts = stripped.split("  ")
            label = parts[0].strip()
            state = stripped[len(label):].strip()
            if state:
                current = (label, state, "")
                rows.append(current)
        elif line.startswith("      ") and rows:
            rows[-1] = (rows[-1][0], rows[-1][1], stripped)
    return rows


def issues():
    try:
        out = subprocess.run(
            ["gh", "issue", "list", "--repo", REPO, "--state", "all",
             "--json", "number,title,state", "--limit", "100"],
            capture_output=True, text=True, cwd=str(ROOT))
        return json.loads(out.stdout) if out.stdout.strip() else []
    except Exception:
        return []


def build() -> str:
    rows = fidelity_rows()
    by_state = {}
    for label, state, note in rows:
        by_state.setdefault(state.split(" (")[0], []).append((label, note))

    lines = [
        "# Roadmap",
        "",
        "**Generated — do not edit by hand.** `python scripts/gen_roadmap.py`",
        "",
        "This file is derived from the code and the issue tracker, because a",
        "hand-written roadmap is stale the day after it is written. Fidelity",
        "grades come from `cargo run --bin fidelity_report`; the work list comes",
        "from GitHub issues. To change the roadmap, change what it describes.",
        "",
        "## What the simulator is verified to get right",
        "",
        "Every API getter carries a confidence grade. The default is UNCERTAIN:",
        "a getter earns CONFIRMED only by being compared against a real-game",
        "recording, and the note must cite which one — a test enforces that.",
        "This matters because \"returns a plausible number\" is not a testable",
        "property, and the bugs that have actually cost us were all of that shape.",
        "",
    ]
    order = ["CONFIRMED", "APPROXIMATED", "UNCERTAIN", "WRONG/ABSENT"]
    summary = " · ".join(f"**{s.title()}** {len(by_state.get(s, []))}" for s in order)
    lines += [summary, ""]

    for state in order:
        entries = by_state.get(state, [])
        if not entries:
            continue
        heading = {
            "CONFIRMED": "### Confirmed against a real-game reading",
            "APPROXIMATED": "### Approximated — documented model, not the engine's own",
            "UNCERTAIN": "### Uncertain — implemented, never measured",
            "WRONG/ABSENT": "### Known wrong — do not trust results that use these",
        }[state]
        lines += [heading, ""]
        if state == "UNCERTAIN":
            lines += ["These are the backlog. Each needs one real-game reading to",
                      "settle, and several feed the tackle economy directly.", ""]
        for label, note in entries:
            lines.append(f"- `{label}`" + (f" — {note}" if note else ""))
        lines.append("")

    open_issues = [i for i in issues() if i["state"].upper() == "OPEN"]
    closed = [i for i in issues() if i["state"].upper() != "OPEN"]
    lines += ["## Open issues", ""]
    if open_issues:
        for i in open_issues:
            lines.append(f"- [#{i['number']}](https://github.com/{REPO}/issues/{i['number']}) {i['title']}")
    else:
        lines.append("None open.")
    lines += ["", f"## Closed ({len(closed)})", ""]
    for i in closed:
        lines.append(f"- ~~[#{i['number']}](https://github.com/{REPO}/issues/{i['number']}) {i['title']}~~")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    text = build()
    if "--check" in sys.argv:
        if OUT.exists() and OUT.read_text(encoding="utf-8") == text:
            print("ROADMAP.md is up to date")
            return 0
        print("ROADMAP.md is STALE - run: python scripts/gen_roadmap.py")
        return 1
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
