# AIComp Soccer — game model (single source of truth)

**Purpose:** One living description of how Soccer works for bots, ball prediction, and a Bevy/Rust reimplementation.  
**Last updated:** 2026-07-22  
**Machine params:** `soccer_ball_sim_params.json`, `bevy_sim_params_v05.json`  
**API labels:** `soccer_api_dump.md` / `.json` (not repeated here)

### Confidence tags

| Tag           | Meaning                                                     |
| ------------- | ----------------------------------------------------------- |
| **CONFIRMED** | Asset scrape, timeplot fit, README, or AIA statement        |
| **COMMUNITY** | Discord / player report; consistent but not remeasured here |
| **CANDIDATE** | Plausible default; do not treat as physics truth            |
| **UNKNOWN**   | Open; needs measurement                                     |
| **BUG?**      | Observed; may be fixed in later builds (recheck on v0.5)    |

Units: meters, seconds. Pitch plane is **XZ**; **Y** up. Home/away long axis ≈ **±X** (goals); sidelines ≈ **±Z**.

**Bevy/Rust copy is fully 2D:** physics and controls are pitch-plane only. Unity 3D (meshes, rest Y, airborne arcs) is visual / optional later — not required for match testing.

---

## 1. Match format

- 4v4. Each team graph drives players `1`–`4` via `SoccerController(player, moveTo, sprint, interact)`.
- Save folder: `…/AIComp/Saves/Soccer/` (bots `*.txt`, settings, Timeplots).
- Match duration: settings `matchDurationSeconds` (often 180; Titanium uses 360 when extended).
- Flow: kickoff → play → goal → kickoff; overtime; whistle (stale ball).

### Kickoff (**CONFIRMED** — README / AIA)

| Event                                 | Receiving team                                      |
| ------------------------------------- | --------------------------------------------------- |
| Match start                           | Random                                              |
| After a goal                          | Team that was **scored on**                         |
| Extra time opening                    | **Opposite** of who received kickoff at match start |
| Whistle (no meaningful ball movement) | **Opposite** of who last received a kickoff         |

During kickoff restriction, only the kicking-off team gets graph control; others stay outside the center circle until first touch / delay expires.

---

## 2. Field geometry

### SoccerGetFloat marks (**COMMUNITY** — AIA Discord; labels are **CONFIRMED** API)

| Label                  |   Value | Notes                               |
| ---------------------- | ------: | ----------------------------------- |
| Field Width            |      50 | ≈ sideline span (our ±Z)            |
| Field Depth            |      80 | ≈ goal-axis span (our ±X)           |
| Kickoff Circle Radius  |    7.25 |                                     |
| Each ground line       |       5 | marking spacing                     |
| Area Depth             |    12.5 | penalty-area depth                  |
| Arena Semicircle Depth |     2.5 | box “D”                             |
| Goal Width / Height    | via API | mouth half-width used in sim: **6** |

### Ball-center playable AABB (**CONFIRMED** — collider audit)

Used by free-ball wall bounce (tighter than Field Width/Depth by ball radius / wall thickness):

- **X** ∈ [−39.5, 39.5] (span 79 vs Field Depth 80)
- **Z** ∈ [−24.7, 24.7] (span 49.4 vs Field Width 50)

### Goals & posts (**CONFIRMED** geometry; bounce model calibrated)

- Goal mouth open: endline walls do **not** collide when **|z| ≤ 6**; ball exits → **score / terminal** (no bounce).
- Endline outside mouth + sidelines: bounce.
- Upright posts: capsules at **(±40.2, ±6)**, world radius **0.3**, ball-center contact radius ≈ **0.706**.
- Nets / geometry behind the goal line: ignore for free-ball paths.

---

## 3. Ball physics

### Prefab / material (**CONFIRMED** — assets)

| Property                               |                                                                              Value |
| -------------------------------------- | ---------------------------------------------------------------------------------: |
| Mass                                   |                                                                               0.45 |
| Scale                                  |                                                                                0.9 |
| Collider radius (local)                |                                                                           ≈ 0.4515 |
| Radius (world)                         |                                                                           ≈ 0.4064 |
| Rest Y                                 |                                                                        ≈ 0.35–0.36 |
| Linear / angular damping               |                                                                                  0 |
| Constraints                            |                                     FreezeRotation XYZ — **slides, does not roll** |
| Soccer material μ_d / μ_s / bounciness |                                                    0.1 / 0 / 0.4 (Average combine) |
| Walls (no material)                    | Unity default μ_d≈0.6, bounciness 0 → effective wall **e ≈ 0.2**, contact μ ≈ 0.35 |

### Ground slide (**CONFIRMED** — timeplot; **COMMUNITY** cross-check ≈ 6)

- Model: Coulomb constant deceleration while grounded: **a = 5.95 m/s²** (Merijn ≈ 6).
- **Not** μ·g from materials alone (that underpredicts ≈ 3.43).
- Stop: `t = |v|/a`, `s = |v|²/(2a)`.
- Free-ball predictor input safety cap: 80 m/s (spikes higher exist).

### Wall bounce (**CONFIRMED** effective)

- Normal restitution **e ≈ 0.2** (~20% normal speed retained).
- Tangential: Coulomb friction up to `μ(1+e)|v_n|` with μ≈0.35.
- Goal mouth: no bounce (terminal).

### Posts (**CONFIRMED** exist; bounce heuristics **COMMUNITY**/timeplot)

- Same e≈0.2 baseline; grazing endline-rail hits need graze floor / tangent scrape (see `soccer_ball_sim_params.json` `goal.post_bounce`). Live Titanium graph may leave post bounce **off** for cost.

### Kicks / airborne (**COMMUNITY**)

| Claim                    |   Value | Notes                                             |
| ------------------------ | ------: | ------------------------------------------------- |
| Max-power launch speed   | ~30 m/s | slight upward pitch                               |
| Max apex height          |  ~1.4 m |                                                   |
| Hang before first bounce |    ~1 s | g=9.81 + rest≈0.35→1.4 ⇒ hang≈0.93 s — consistent |
| Implied pitch            |   ~8–9° | from \|v\|=30 and that `v_y`                      |

Held ball → live carrier; loose → free coast (XZ sim flattens Y until airborne kicks are modeled).

---

## 4. Players

### Control (**CONFIRMED**)

```text
SoccerController(player, moveTo: Vector3, sprint: Bool, interact: Bool)
```

- **moveTo** — world destination (simple mover toward target).
- **sprint** — faster when stamina allows.
- **interact** — with ball: hold to **charge shot**, release (`false`) to kick along that frame’s move direction; without ball: **tackle / pickup**.

Locomotion is **controller-driven**, not free Coulomb slide like the ball. Community: **perfect simple movers** (reach ≈ circle growing at max speed). Supposedly similar _shape_ to Survival (move + sprint + stamina), but Soccer speeds are **not** audited.

### Speeds / accel

| Quantity                                |     Value | Tag                                |
| --------------------------------------- | --------: | ---------------------------------- |
| Intercept default max speed             |   4.5 m/s | **CANDIDATE** (Titanium ReachTime) |
| Intercept default acceleration          |  2.5 m/s² | **CANDIDATE**                      |
| Survival walk / sprint (reference only) | 5 / 9 m/s | **not Soccer evidence**            |
| Soccer walk/sprint/accel                |         — | **UNKNOWN**                        |

### Stamina

| Claim                    |                                          Value | Tag                          |
| ------------------------ | ---------------------------------------------: | ---------------------------- |
| Exposed                  | `Team Player N Stamina`, carrier stamina, etc. | **CONFIRMED** API            |
| Full drain / regen times |                                  ~30 s / ~15 s | **COMMUNITY** (extrapolated) |
| Survival drain/regen     |  0.15/tick sprint; regen 2/tick after 3 s idle | **not Soccer evidence**      |

### Tackle / possession (**CONFIRMED** API intent; wording varies)

- Interact near the **ball** (`Player Interact Radius`), not body hitbox alone.
- Free ball: **must interact/tackle** — not automatic (**CONFIRMED** AIA).
- Contest: stamina drained from both (README: stamina **delta**; Merijn: deduct **min** of the two — same family of rules); higher stamina after drain wins / keeps ball.

### Pickup delay after shot (**CONFIRMED** AIA Jul 18; verify build)

- **0.3 s**, was **global** (nobody can grab after any shot). AIA said they would change it — **recheck on worldcupv0.5**.
- Explains standing on a still ball without pickup and terrible “pass then reclaim” if delay ignored.

### Known interaction bugs (**BUG?** — may be fixed)

- Catch moving ball but not stationary.
- Full charge → interact off → still holding ball next frame (failed release / instant regrab).

---

## 5. Sensors & clear directions (**CONFIRMED**)

- `SoccerPlayerSensors` — 8 spherecasts **A–H** around a player.
- “Clear direction …” Vector3 getters: first unobstructed direction in attack-priority order, or null.

| Side | Order                  |
| ---- | ---------------------- |
| Home | E, C, H, B, G, A, F, D |
| Away | D, F, A, G, B, H, C, E |

---

## 6. Graph / coordinate gotchas (**CONFIRMED** AIA)

- `RelativePosition(transform, "Self")` = local frame of the **input transform**, **not** “the player running this graph.” Prefer World: `target_world − player_world`.
- Bot export: `SaveData` → `Saves/Soccer/*.txt`. In-game **Export JSON** path: **UNKNOWN**.

---

## 7. What Titanium / Bevy use today

| Layer                                        | Status                                 |
| -------------------------------------------- | -------------------------------------- |
| Free-ball XZ coast + walls + friction a=5.95 | Implemented / calibrated               |
| Open goal mouths terminal                    | Implemented                            |
| Post bounce                                  | Optional / often shelved               |
| Airborne kick parabola                       | Not in live XZ predictor yet           |
| Player ReachTime 4.5 / 2.5                   | Candidate only                         |
| Pickup delay 0.3 s                           | Documented; not wired as sim event yet |

Offline sim: `ball_physics_sim.py`. Bevy pack: `bevy_sim_params_v05.json`.

---

## 8. Gap to a match-capable Bevy/Rust sim

**Have now:** calibrated free-ball XZ coast + walls + goals (Python); param packs; rules doc. **No Bevy crate yet.**

### Must-have for simulated matches (MVP)

| Piece                                                              | Status                    | Notes                                                     |
| ------------------------------------------------------------------ | ------------------------- | --------------------------------------------------------- |
| Pitch + walls + open mouths + score                                | Params ready; code not    | Port `ball_physics_sim`                                   |
| Ground slide a=5.95                                                | Ready                     |                                                           |
| 8 simple movers (accel→cruise, moveTo)                             | Speeds **CANDIDATE**      | Ship with 4.5/2.5 or Survival 5/9; measure later          |
| Possession: interact pickup / tackle / held ball follows carrier   | Rules known; numbers soft | Need interact radius value from API at runtime or measure |
| Kick: charge→release→impulse                                       | Max ~30 m/s **COMMUNITY** | Flat XZ kick OK for v1; Y optional                        |
| Global pickup delay 0.3 s                                          | Documented                | Wire as timer                                             |
| Kickoff + goal reset + match clock + whistle                       | Rules known               | Faceoff positions from ConstructSoccerProperties          |
| Fixed-dt match loop + two “brain” hooks (moveTo/sprint/interact×4) | Not built                 | Brains can be scripted, not full AIComp graphs            |

### Nice-for-later (not blocking scripted matches)

- Airborne kick / bounce-in-Y, post graze heuristics
- Accurate stamina drain/regen, sprint multiplier
- 8-way spherecast / clear-dir sensors
- Full AIComp graph interpreter (load Titanium.txt)
- Pixel-perfect match to Unity bugs

### Open measurements (priority)

1. Soccer walk vs sprint **max speed** and **acceleration** (timeplot player positions).
2. Confirm **pickup delay** still 0.3 s / still global on v0.5.
3. Confirm tackle stamina formula (delta vs min) + **Player Interact Radius**.
4. Kick launch: speed, pitch, **charge→speed curve**.
5. Stamina drain/regen rates under sprint-only.

---

## 9. Related files

| File                          | Role                                                      |
| ----------------------------- | --------------------------------------------------------- |
| `SOCCER_GAME_MODEL.md`        | **This document** — human game model                      |
| `soccer_ball_sim_params.json` | Numbers for ball sim + community block                    |
| `bevy_sim_params_v05.json`    | Bevy/Rust extract                                         |
| `soccer_api_dump.md`          | Getter label lists                                        |
| `ball_physics_sim.py`         | Offline free-ball simulator                               |
| `AIGamePyLibrary/README.md`   | Official Soccer node docs (kickoff, sensors, controllers) |
