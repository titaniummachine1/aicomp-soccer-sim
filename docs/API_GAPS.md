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
| **Spherecast** | yes (AIA ×1) | **LOCKED** AIA: Float radius **0.25**, distance **20** → shared into Sensors1..4. Unity SphereCast shape. Still Null in VM — clear-dirs approximated. |
| **SoccerPlayerSensors1..4** | yes (AIA ×1 each) | 8× casts A–H → RaycastHit. Debug: green=clear, red=hit. Clear-dir order in game-model §5. Still Null in VM. |
| **HitInfo** | rare / via sensors | **LOCKED** tags/ports; still Null in VM. |
| **ConstructSoccerProperties** | yes (faceoff) | Faceoff Vector31–34 + Country. Match setup, not per-tick brain I/O. |
| **Stat** | no (other sims) | Survival/Parking uniform token — not in Soccer Legia. Ignore / Null. |
| **Country** | yes (w/ Construct) | Outputs selected country (dropdown). Faceoff-only with ConstructSoccerProperties. |
| **Keypress** | some graphs | **Wired** → always `false` in sim (no keyboard in headless/AI). |

**SoccerController1..4** — not blank for gameplay: graph **writes** moveTo/sprint/interact.
Reading them as values yields Null (correct).

**Viz / org (no gameplay value):**

| Node | Status |
| ---- | ------ |
| **Color** | **Wired** as non-null constant (modifier name). Feeds TimePlot/DebugDraw. |
| **Region** | No ports — org-only; Null OK. |
| **Debug / DebugDraw*** / **TimePlot** | Side effects only (no outs). Null as values OK. Debug = “Displays the real-time value of the output connection” (Any1). TimePlot: name/color/icon/value + optional min/max; F1. |
| **Custom Function** | Calls Construct Custom Function **by name** (up to 4 Any params → Return). Sim already resolves CreateFunction by modifier name at compile. |

---

## 2. Partial / incomplete implementations

| Item | Status | Gap |
| ---- | ------ | --- |
| **RelativePosition** | **Self / World** = world pos of Transform1. Cardinal / Self+* use **world axes** (Forward=+X, Left=+Y/Z). Up/Down ignored in 2D. | No TeamApi facing yet — Self+Forward is world-axis, not transform.forward. Enough for stock graphs (Self/World). |
| **Clear-dir / OppGoal dir** | Implemented in API snapshot | Sensors path (Spherecast → HitInfo → Sensors) still blank; we **approximate** with geometric clear rays. |

---

## 3. SoccerGet\* dropdown coverage

Unity dropdown labels vs sim:

- BOOL / FLOAT / TRANSFORM / VECTOR3 dropdown labels used by graphs are **wired**, including `Pi`, `Is Away Team`, `Is Kickoff`, `Is Opponent Kicking off`.

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

## 5. Survival / Unity — what you *can* vs *cannot* deduce

Searched Survival (Aya / public writeups + Frida catalog) for Soccer sensor truth:

| Deduce? | What |
| ------- | ---- |
| **Yes (shape)** | Unity `Physics.SphereCast` → `RaycastHit`. Spherecast → Sensors (8 dirs A–H) → SoccerGet clear-dir. |
| **Yes (order)** | Home `E,C,H,B,G,A,F,D` / Away `D,F,A,G,B,H,C,E` — game-model §5; `api/clear.rs`. |
| **Yes (AIA numbers)** | **radius 0.25**, **distance 20** — locked from AIA graph + Debug (quirk #15). Not Survival. |
| **No** | Layer masks / collider name strings. Survival walk/sprint numbers do **not** transfer (Soccer walk **7** / sprint **8**). |
| **AIA reality** | Gameplay reads SoccerGet clear-dirs / opp-goal-dirs. Raw HitInfo path still Null in sim. OppGoal-dir often **null** (= clear lane). |

AIA usage scan: `python scripts/aia_gap_report.py`

Headless parity capture:

```bat
cargo run --release --bin timeplot_until_goal -- --secs 30 --home aia --away idle
```

Unity: load `AIA_Debug.txt`, same opening/length, export TimePlot → compare series.

---

## 6. Implementation priority (suggested)

1. **Spherecast + HitInfo + SoccerPlayerSensors1..4** — only if a bot relies on raw sensor hits instead of SoccerGet clear-dirs (AIA mostly uses the getters).
2. **ConstructSoccerProperties / Country** — only if debugging kickoff faceoff construction in-graph.
3. Lock §4 UNSURE rows via TimePlot / AIA answers.
4. Optional: RelativePosition facing from transform rotation (if Unity Self+Forward ever matters).

---

## 7. Done recently (not blank)

Opp has-ball / nearby / closest ×4, winning, scored-last, opp side, headed-towards,
shots, possession%, attacking%, ball speed, charge%, goal W/H, sim clocks, Δt,
nearest-opp stamina ×4, opp nearest-TP transforms, opp posts, nearest-goal
transforms, direction of ball from Opponent 1–4, **`Pi`**, RelativePosition
Self/World (+ world-axis Self+*), Is Away / Kickoff / Opp kicking off labels.
See §4 for soft ones.
