# Titanium handoff — resume here

**Date:** 2026-07-23 (~05:00 UTC+2)  
**Sim repo:** `C:\gitProjects\worldcup\aicomp-soccer-sim` (branch `main`, dirty — Titanium WIP)  
**Engine repo:** `C:\gitProjects\worldcup\titanim-socker-engine` (branch `challenge/finishing-open-look`, dirty)  
**Transcript:** [Titanium overnight](877cb007-a85a-443e-b5a4-d42acd64662c)

Do **not** treat the 1v1 harness as a general match mode. Keep sim match logic intact; iterate GK/attacker in brains + harness only. Engine `main` = champion — merge only if challenge beats it ([`docs/CHALLENGE.md`](../../titanim-socker-engine/docs/CHALLENGE.md)).

---

## Goal (user intent)

Build Titanium GK + attacker that are strong in a **scripted 1v1 training suite**, then scale to full matches vs AIA.

### Scenario 1 harness rules (`titanium_drill` + viewer)

| Win          | Condition                                                            |
| ------------ | -------------------------------------------------------------------- |
| **Attacker** | Scores a goal                                                        |
| **GK**       | Intercepts / takes control of the ball (`carrier == GK P4` and held) |
| **GK**       | Timeout (~**60s wall clock** per trial; headless is accelerated)     |

Setup each round (roles swap Home/Away):

- Attacker **P1 at center** `(0, ±3)` with ball
- GK **P4 already on cone-bisector cover** (≥8 m gap, own half, not past goal)
- Only GK + attacker are visible; all other roster players are parked well
  off-pitch and **frozen** in the harness

Viewer: `cargo run --release -- --scenario 1`  
Top bar: cycle `Full` / `S1 Atk vs GK`

The win table above is unchanged: attacker goal, or GK capture/hold or timeout.

```bat
cargo run --release --bin titanium_drill -- --trials 10
cargo run --release --bin titanium_drill -- --trials 10 --wall-secs 60 --quiet
```

JSONL appends to `data/titanium/eval_log.jsonl`.

---

## GK architecture to implement (user-locked design)

Three modes — **do not** build one giant policy. Priority:

```
1. Guaranteed intercept of ball (loose / pass / shot path)?
      yes → attack the ball (ignore geometry)
      no  ↓
2. Shot already kicked / emergency?
      yes → maximize touch before goal line
      no  ↓
3. Positioning: stay on cone bisector(s), cover width, press carrier
      — never past midfield (x=0)
      — closest safe point to ball carrier while cones stay covered
```

### Positioning geometry (O(1), no grid)

Single threat **A**, posts **L/R**, save/reach radius **R**:

- θ = ∠LAR
- Bisector **b** from normalized `(L−A)/|…| + (R−A)/|…|`
- Safe: `d ≤ R / tan(θ/2)` ⇔ cone width at distance ≤ `2R`
- Ideal stand: `G = A + b·d` (closest to attacker that still covers whole cone)
- Clamp: own half only (`x` not past midfield)

Multi-threat (next step): each threat gets cone + effective `R_i = v_gk · time_i` (pass+control+kick+travel). Safe region = intersection of constraints; if empty, prioritize fastest / biggest angle. Weight near-ball threats with `exp(-dist_ball / scale)`.

Already in code (approx): `gk_cone_bisector_cover`, `gk_cover_from_threats` in:

- `aicomp-soccer-sim/src/predict/mod.rs`
- `titanim-socker-engine/src/predict.rs`

**Still needed in `think_gk`:** clean mode switch — if `truncate_to_guaranteed_intercept` says GK is first, **go to ball**; else hold cover / press along bisector. Do not abandon cone for fake passes without a confidence/delay (user note).

### Attacker (1v1)

- Carry wide toward box; **don’t full-power from midfield** (`dist_goal < ~22` before release)
- Far-post / evade flick; panic early only if GK in tackle range
- Ignore parked wide corners when detecting “1v1” (`|z| < 14`)

---

## Latest measured results

| Test                             | Result                                                      |
| -------------------------------- | ----------------------------------------------------------- |
| Harness 20× (2026-07-23)         | **8 atk goals / 12 GK catches** (0 timeouts) — mixed OK     |
| Harness before this session      | 10/10 GK catches (held-ball Mode1 chase + early tackle)     |
| Titanium vs AIA first-to-5       | **1–5** AIA (~125s) — still weak in full match              |

Lib tests green (76 sim / 11 engine). Re-run after edits: `cargo test --lib`.

---

## Key files

| Path                                                 | Role                                          |
| ---------------------------------------------------- | --------------------------------------------- |
| `src/bin/titanium_drill.rs`                          | **Only** 1v1 training harness                 |
| `src/titanium/mod.rs`                                | Sim Titanium brain (atk + GK)                 |
| `src/predict/mod.rs`                                 | Cone cover, intercept, shot/clear helpers     |
| `src/bin/titanium_eval.rs`                           | Batch vs AIA / self-play logging              |
| `../titanim-socker-engine/src/{titanium,predict}.rs` | Engine mirror — keep in sync for challenges   |
| `data/titanium/eval_log.jsonl`                       | Append-only results                           |
| `AGENTS.md`                                          | Prefer `soccer_headless` / `cargo test --lib` |

Pitch: sim `Vec2(x,y)` = Unity `(X,Z)`; X = goals; walk 7 / sprint **8**.

---

## Suggested next steps (fresh session)

1. Re-run harness smoke:  
   `cargo run --release --bin titanium_drill -- --trials 20 --wall-secs 60 --quiet`

2. **Full-match finishing** vs AIA — still 1–5. Port 1v1 outside-carry + evade finish into open play; fix Away Clear-F interaction for non-drill shots.

3. Optional GK: multi-threat cone intersection (handoff geometry note).

4. Don't merge engine `main` until challenge protocol wins.

5. Overnight loop (optional): check terminals before duplicating `AGENT_LOOP_WAKE_titanium`.

---

## Do / don’t

- **Do** change brains + `titanium_drill` + predict helpers.
- **Don’t** rewrite core match possession for the harness.
- **Don’t** invent Survival speeds.
- **Don’t** merge engine champion without beating it.
