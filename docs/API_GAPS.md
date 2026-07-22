# API / graph gaps — blank stubs & soft semantics

Living checklist. **Blank** = returns Null / missing (must implement for
parity). **Unsure** = wired with a best-guess rule (needs TimePlot / AIA / Frida
before locking). Update when anything moves.

Scan helper: `python scripts/api_gap_scan.py` (dropdown vs `labels.rs`).

---

## 1. Graph **node types** still blank (eval → Null)

These appear in Unity saves / the Legia sidebar. GraphBrain + RuntimeBrain treat
them as Null today (except controllers, which are **write-only** sinks).

| Node | Used in saves? | Notes / how to infer |
| ---- | -------------- | -------------------- |
| **Spherecast** | yes (×4 in some graphs) | Frida: inputs Float1/Float2. Likely origin+radius or length+radius. Feeds sensors. |
| **SoccerPlayerSensors1..4** | yes | Frida: 8× Spherecast in → 8× RaycastHit out (A–H). Clear-dir system in Unity. |
| **HitInfo** | rare / via sensors | Frida outs: Bool1, Float1, String1 from RaycastHit1. Hit? / distance? / collider name? |
| **ConstructSoccerProperties** | yes (faceoff) | Faceoff Vector31–34 + Country. Match setup, not per-tick brain I/O. |
| **Country** | yes (w/ Construct) | Team/country pick for ConstructSoccerProperties. |
| **Color** | yes (debug draw) | Visual only; Null OK for headless. |
| **Stat** | ? | Catalog stub; purpose unknown. |
| **Keypress** | yes (some graphs) | Falls through catch-all Null. Keyboard gate for debug bots. |
| **Debug / DebugDrawDisc / DebugDrawLine / TimePlot / Region** | yes | Side effects / viz. Null as values is OK; TimePlot recording is separate (`TimePlotRecorder`). |

**SoccerController1..4** — not blank for gameplay: graph **writes** moveTo/sprint/interact.
Reading them as values yields Null (correct).

---

## 2. Partial / incomplete implementations

| Item | Status | Gap |
| ---- | ------ | --- |
| **RelativePosition** | Returns **world position** of input transform only | Does **not** honor modifier **Self** vs **World**. Quirk #8: Self = local frame of *input* transform, not controlling player. Legia `Relative Direction` uses World — works today. Self-offset graphs will be wrong until fixed. |
| **Clear-dir / OppGoal dir** | Implemented in API snapshot | Sensors path (Spherecast → HitInfo → Sensors) still blank; we **approximate** with geometric clear rays. |

---

## 3. SoccerGet\* dropdown coverage

Unity dropdown labels vs sim:

- Almost all BOOL / FLOAT / TRANSFORM / VECTOR3 labels are **wired**.
- Still missing from `labels.rs` (but already filled in snapshot): `Is Away Team`, `Is Kickoff`, `Is Opponent Kicking off`.
- Still **BLANK** float: **`Pi`** (trivial constant).

---

## 4. Wired SoccerGet\* with **UNSURE** semantics

Implementations exist so graphs compile; **do not treat as locked** until measured.

| Label | Current sim rule | Why unsure |
| ----- | ---------------- | ---------- |
| **Goal Height** | `2.44` | Never TimePlotted; width `2×goal_half_width` is solid. |
| **Team / Opponent Shots** | +1 on every charged kick release | Unity may count only on-target / past a plane / Interact edge cases. |
| **Team / Opponent Possession %** | Share of Play-time each side *held* ball | May include loose-ball rules or different clock. |
| **Team / Opponent Attacking %** | Share of Play-time ball on attacking half (`x≷0`) | Definition of “attacking” unverified. |
| **Is Ball Headed Towards Team/Opponent Goal** | `speed>0.5` and `vel·to_goal > 0.5` | Threshold / 3D height unknown. |
| **Player With Ball Shot Charge %** | Same as Ball Carrier Shot Charge (**0..1**) | **LOCKED** AIA 2026-07-22 — label has `%`, value is 0–1. |
| **Opponent Nearest Teammate Player N Stamina** | Stamina of nearest **opponent** to team player N | Awkward name; could mean nearest teammate of opp player N. |
| **Opponent/Teammate Nearest Team/Opponent Goal** | Player of that side closest to that goal | Plausible; not dumped. |
| **Delta Time** | Always `FIXED_DT` (0.019) | Viewer may want wall Δt; soccer graphs usually want fixed. |

---

## 5. Implementation priority (suggested)

1. **`Pi`** — trivial.
2. **RelativePosition Self vs World** — blocks correct distance helpers that use Self.
3. **Spherecast + HitInfo + SoccerPlayerSensors1..4** — needed if a bot relies on raw sensor hits instead of SoccerGet clear-dirs.
4. **ConstructSoccerProperties / Country** — only if debugging kickoff faceoff construction in-graph.
5. Lock §4 UNSURE rows via TimePlot / AIA answers.

---

## 6. Done recently (not blank)

Opp has-ball / nearby / closest ×4, winning, scored-last, opp side, headed-towards,
shots, possession%, attacking%, ball speed, charge%, goal W/H, sim clocks, Δt,
nearest-opp stamina ×4, opp nearest-TP transforms, opp posts, nearest-goal
transforms, direction of ball from Opponent 1–4. See §4 for soft ones.
