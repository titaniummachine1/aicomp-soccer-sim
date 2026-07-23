# AIA / AIComp Soccer — quirks & bug-like behavior

Living list of unnatural, surprising, or likely-buggy behaviors seen in the
**real game** (via AIA_Debug TimePlots) and in AIA’s own graph. Use this to
report upstream to AIA / Unicorn One for possible patches.

Evidence baseline: `timeplot_2026-07-22_05-01-57.json` (Home=AIA_Debug build 2,
Away=AIA, ~23s, no duplicate channels).

Status legend: `ENGINE` = Unity/AIComp soccer runtime · `BOT` = AIA graph logic ·
`CONFIRMED` = measured in TimePlot · `SUSPECTED` = inferred from graph + plots ·
`AIA-TIP` = author / Discord tip (treat as locked unless TimePlot contradicts).

---

## AIA author tips (LOCKED 2026-07-22)

Editor / saves (workflow, not sim physics):

- Open visual scripting with **`~`** or the top-left button (need a team
  selected — click a player first).
- Saves under AppData LocalLow `…/AIComp/Saves/Soccer/`; Load → **Open folder**
  drops files; menu turns each save into a clickable button.

### Tackle (verbal engine rule)

- **Where:** `ENGINE` · `AIA-TIP` · **LOCKED 2026-07-23** (equal-draw clarified 2026-07-23b)
- **What:** On a contested tackle, subtract **`min(tackler, carrier)`** from
  **both** players. Whoever has stamina left keeps the ball. If both end at 0
  (equal stam), the **carrier** keeps the ball (no free steal).
- **Examples:** T=0.3 C=0.8 → both −0.3 → C keeps (0.5). T=0.8 C=0.3 → both −0.3
  → T keeps (0.5) + ball. T=C=0.5 → both 0 → carrier keeps.

### Vector3 “directional” SoccerGets (clear / opp-goal / clear-mate, …)

- **Where:** `ENGINE` · `AIA-TIP` · matches §1 / game-model §5
- **What:** Check the **8 sensor spherecasts**; return the **first** direction
  that has an unobstructed view of the option, else **null**. Priority =
  attacking direction:
  - **Home:** `E, C, H, B, G, A, F, D`
  - **Away:** `D, F, A, G, B, H, C, E`
    (letters = Player Sensor graphic A–H.)

### Kickoff / whistle / extra time

- **Where:** `ENGINE` · `AIA-TIP` · sim mostly matches
- **What:**
  - Opening receiving team = **random**.
  - After a goal, **scored-on** team receives next kickoff.
  - **Extra time:** receiving team = **opposite** of who opened the match.
  - **Whistle** (no ball movement after timeout): flip to opposite of who
    **last received** a kickoff.
- **Sim:** opening random + scored-on + whistle flip wired; **extra time not
  implemented** yet (note when adding ET clock).

---

## High priority (break intent / look broken)

### 1. `Direction of opponent goal from Teammate N` is usually **null**

- **Where:** `ENGINE` · `CONFIRMED` (TimePlot + **AIA Discord 2026-07-22**:
  “the clear direction can be null yes. If there's no clear sensor direction
  of the goal”)
- **What:** Not a raw vector to the goal center. Present only ~4–12% of ticks.
  When present it is an **8-way unit dir** whose ray reaches the **goal mouth**
  unobstructed. First present for T1 in the baseline plot: **t≈14.5s** near x≈35.
- **Why it hurts AIA:** `Set First Direction(player, OppGoalDir, ClearDir, …)`
  prefers OppGoalDir. When it is always non-null (naive implementations), the
  carrier marches straight at goal (Ball.Z≈0). When correctly null, AIA falls
  through to clear-dir and gets diagonals — which is what the real plot shows
  early (MoveTo ≈ pos + 2×clear). Also drives Ready Shoot: striker’s both dirs
  are OppGoal → Ready stays 0 midfield → full-charge kick (see #5).
- **Ask AIA/engine:** Document this getter as “clear shot lane into mouth or
  null”, or rename it. If the bot intended “always aim at goal”, wire
  `Normalize(OppGoalCenter − player)` (AIA already has `StrikerDirToOppGoal`)
  as Direction 1 instead of the SoccerGet.

### 1b. Custom functions **cannot nest** (engine limit)

- **Where:** `ENGINE` · `CONFIRMED` (AIA Discord bot, 2026-07-22; Maia /
  PunkSkeleton report: nested `Player Distance To The Ball` →
  `Relative Direction` made every player distance identical)
- **What:** “you can't currently nest functions inside of other functions. if
  you want to use them you'll need to pass a value from one function into
  another as a parameter.” Nested `Function` calls yield Null / identical
  wrong results (e.g. Player Distance always the same).
- **Sim:** GraphBrain + O0 lowerer reject nested `Function` (return/emit Null).
- **Note:** Stock `AIA.txt` has **0** nested `Function` calls — this quirk
  breaks _other_ graphs that nest helpers, not AIA itself.

### 2. Kickoff walk-in is **engine-scripted** (graph control locked)

- **Where:** `ENGINE` · `CONFIRMED` (user 2026-07-23 + empty `XD.txt`)
- **What:** During `Is Kickoff`, players **cannot** be graph-/keyboard-controlled
  until Play (or after pickup). The kicking **striker alone** is walked to the
  free ball at origin by the engine — even a near-empty team save (`XD.txt`)
  still walks in. Faceoff keeps both strikers on ±(1,7) wings; ball free at
  center. `Is_Kickoff` clears on pickup (~1s Away open in DB33/35).
- **AIA note:** `StrikerKickoffPos = Vector3Zero` when kicking off coincides
  with the engine walk target, but is **not** what drives the walk (empty
  graphs still move).
- **Sim:** `filter_kickoff` freezes all graph cmds in Kickoff; injects
  engine walk-in (`MoveTo=ball`, walk, Interact) for kicking P1 only.
- **Ask:** Document kickoff control lock + engine walk-in vs graph MoveTo.

### 3. `StrikerState1 = ClearDir × 2` (no `+ playerPos`)

- **Where:** `BOT` · `CONFIRMED` in decompile
- **What:** When team has ball and P1 does not:
  `Clear direction from team carrier * 2` as **world MoveTo**, not
  `player + clear*2`. Playmaker correctly does `pos + Normalize(clear)*2`.
- **Why weird:** MoveTo lands near the origin (±2 m), not a lead point ahead of
  the striker. Inconsistent with Playmaker/Goalie and with `Set First Direction`.
- **Ask AIA:** Almost certainly should be `pos + clear*2` (or Scale after Add).

### 4. Striker never sprints while chasing a loose ball

- **Where:** `BOT` · `CONFIRMED` (graph)
- **What:** Every branch of `StrikerSprint` requires `Team Player 1 Has Ball`.
  Real Away chase still hits ~7 m/s with Sprint=0 — so **walk speed ≈ sprint
  cruise**, and the sprint flag is useless for closing down.
- **Ask AIA:** Add loose-ball / closest-to-ball sprint terms (Defender already
  has similar logic).

---

## Medium (surprising API / graph semantics)

### 5. `Ready Shoot/Pass` + `Player Interact` charge/release machine

- **Where:** `BOT`/`ENGINE` · `CONFIRMED` (function body + baseline TimePlot +
  **AIA 2026-07-22**)
- **Ready Shoot/Pass(dir1, dir2, thresh):**
  `Ready = (present(dir1) AND charge>=thresh) OR (present(dir2) AND charge>=thresh)`
  i.e. charge past threshold **and** at least one desired shoot/pass dir non-null.
  Float thresh uses locale commas (`"0,5"` / `"0,35"` / `"0,75"`).
- **Player Interact(ready, nearby, hasBall):**
  If Ready → Interact **false** (Bool `"1"` = false).
  Else → `(hasBall AND charge<1) OR (nearby AND NOT TeamHasBall)`.
- **Engine kick (AIA + TimePlot 17-11-17):** once you have the ball (Interact
  while nearby), the **same Interact** charges the shot; when Interact goes
  **false**, the ball is kicked along that frame's **MoveTo** (not facing/hold).
  Confirmed **instant 90° flick**: walk/hold +X, MoveTo +Z → kick vel pure +Z
  (~29.4) with hold turn ≈0°/s that frame.
- **Striker quirk:** both dirs are OppGoal (usually **null**) → Ready stays 0
  even at charge≥0.5. Charge runs to **1.0**, then `hasBall AND charge<1`
  fails → Interact drops → full-power kick. Real Home kickoff: charge 0.4→1.0
  by ~t=0.76 with Striker Ready=0 the whole time.
- **Playmaker:** ClearMate often present → Ready flips true at thresh → earlier
  release (or charge-to-thresh then kick).
- **Sim:** release impulse uses `(move_to - pos)`; facing only as fallback —
  matches the flick TimePlot.
- **Ask:** Comment the invert; consider wiring striker dir2 to ClearMate so Ready
  can fire mid-charge instead of always dumping at 1.0.

### 5b. Body-push / hold-offset must not leave the stadium or phantom-score

- **Where:** `SIM` · fixed 2026-07-22
- **What:** `resolve_player_bodies` could teleport the loose ball past sidelines /
  endlines (no wall pass after separation) — looked like “ball bugs out of
  bounds.” Separately, held `hold_offset` past the goal line scored while the
  carrier was still on the pitch → easy goal spam (e.g. 50–0).
- **Sim:** Re-run wall/post containment after body resolve. Held goals require
  **carrier body** on/over the goal line, not only the hold point.
- **Wall settle (AIA 2026-07-22):** soft shove into solid wall (endline outside
  mouth / sideline / post) depenetrates onto the face and **stops dead**
  (into-speed ≤5 m/s). Fast free kicks still bounce with e≈0.2 — not an
  explosion on shove.

### 6. Bool constant modifiers are inverted (dropdown index)

- **Where:** `ENGINE` · `CONFIRMED` (AIGamePyLibrary + **AIA Discord 2026-07-22**:
  “it's because of the order of the dropdown and true is the first (default
  option) it's not a representation of the boolean”)
- **What:** `Bool(True)→"0"`, `Bool(False)→"1"` — modifier is dropdown index,
  not 0/1 truthiness. Our eval: `mod != "1"` ⇒ true.
- **Ask:** Document loudly; many bots will get this wrong.
- **AIA update (2026-07-22):** library now accepts **string or index** for
  dropdown options — prefer `"true"`/`"false"` (or labels) in new graphs; old
  saves still use `"0"`/`"1"`.

### 7. Clear-direction getters return **unit vectors**, not world points

- **Where:** `ENGINE` · `CONFIRMED` (Clear.Carrier ∈ {E,C,H,…} with |components|∈{0,0.707,1})
- **Ask:** Document. AIA mixes “×2 as MoveTo” (bug) vs `pos + n*clear` (correct).

### 7b. `Is * Player N Open` = no opposing body within **2× interact radius**

- **Where:** `ENGINE` · `CONFIRMED` (**AIA**, 2026-07-22)
- **What:** A player is open iff there is **no opposing player within
  `2 × Player Interact Radius`** of them. Otherwise not open.
  Equivalently: `nearest_opp_dist > 2×R` → true.
- **Sim:** Wired for `Is Team Player N Open` / `Is Opponent Player N Open`.
  Vector getters `Get nearest/most/furthest open *` only consider players that
  pass this test (`most` = largest nearest-opponent clearance among open).

### 8. `RelativePosition(transform, "Self")` ≠ controlling player frame

- **Where:** `ENGINE` · documented in SOCCER_GAME_MODEL
- **What:** Local frame of the **input** transform. World pos of Ball =
  RelativePosition(Ball, Self) with zero offset — confusing name.
- **Ask:** Rename or document; prefer World subtraction for “offset from me”.

### 9. Typo: `PlayermakerReadyToShoot`

- **Where:** `BOT`
- **Ask:** Rename to `PlaymakerReadyToShoot` (breaking for saves that bind the
  old name — or alias both).

### 10. Dual AIA_Debug = duplicated TimePlot series

- **Where:** `ENGINE` · `CONFIRMED`
- **What:** Same channel names from Home and Away; export has 2× series.
  Compare scripts must take the first (Home) or forbid Debug on both sides.
- **Ask:** Prefix by team, or only record the active/home graph.

---

## Lower / measurement notes

### 11. Walk ≈ 7 m/s · sprint = 8 m/s · **no stamina throttle**

- **Where:** `ENGINE` · `CONFIRMED`
  - Walk (Sprint=0): ~7 m/s (Away chase + StaminaProbe).
  - Sprint cruise: **exactly 8.0 m/s** flat (clips hard).
  - TimePlot **18-53-07 DB2** (always-sprint to empty): median **8.0** in every
    stam bin down through `stam≈0.00015` (612 samples with stam≤0.01). Speed
    does **not** scale with stamina; Sprint flag alone picks 8 vs 7.
- Community “Survival-like 4.5 walk / 9 sprint” does **not** match Soccer.
- **Ask:** Publish Soccer walk/sprint/accel from the build.

### 12. `SoccerGetFloat("Ball Speed")` can spike very high

- **Where:** `ENGINE` · `SUSPECTED` (one capture max ≈86 while |BallVel| elsewhere
  looks lower)
- **Ask:** Confirm units / whether Y or noise is included.

### 13. Kickoff circle vs faceoff

- **Where:** `ENGINE`
- Non-kicking faceoff (±1,7) is **inside** r=7.25 (dist≈7.07). Real Home
  non-kick capture briefly showed T1 at (0, 7.75) then settled on (−1, 7) —
  possible circle push then walk-back.
- **Ask:** Document circle exclusion for the receiving team.

### 14. Ready Shoot/Pass thresholds differ by role

- Striker/Playmaker 0.5, Defender 0.35, Goalie 0.75 (Float `"0,5"` locale).
- Not a bug; note for parity. Locale commas in Float modifiers are real
  (`ENGINE` stores European decimal commas in Float node modifiers).

---

### 15. Player sensors = 8× Spherecast @ 45° (AIA radius **0.25**, distance **20**)

- **Where:** `BOT`/`ENGINE` · `LOCKED` (AIA graph Floats + SoccerPlayerSensors
  **Debug** viz 2026-07-22)
- **What:** Stock AIA wires one `Spherecast` → all four `SoccerPlayerSensors`:
  - **radius = 0.25**, **distance = 20** (European Float `"0,25"`).
  - Each sensor fires **8 spherecasts** A–H every **45°** (compass rose).
  - Node **Debug** draws those rays: **green = clear** to max range, **red =
    hit** (shortens to collider). That overlay is **Player Sensors**, not a
    separate Spherecast debug.
  - Origin while held ≈ **ball / hold point** (looks “ball-sized”); AIA still
    hardcodes 0.25 — it is **not** the `Ball Radius` getter (sim ball_r ≈
    **0.406**). Plausible “ball hitbox probe” by design, but the Float is
    fixed in-graph.
- **Clear-dir / OppGoal-dir:** SoccerGets consume those hits (null = clear lane
  for OppGoal). Sim geometric approx: **range=20**, blocker ≈
  `body_radius + 0.25`. Goal-mouth dirs may still need thicker probe
  (≈ interact 1.75) — keep split until TimePlot disagrees.
- **HitInfo:** assume **default Unity** `RaycastHit` after `Physics.SphereCast`
  (has hit / distance / `collider.tag`). AIComp only packages those three outs;
  miss distance = **infinity** per tooltip. No custom Soccer hit logic assumed.
- **Tag strings (LOCKED user 2026-07-22):** `Ball`, `HomePlayer1..4` /
  `AwayPlayer1..4`, `Boundary`, `HomeGoal` / `AwayGoal`, `HomeGoalPost` /
  `AwayGoalPost`. (Seen when not dribbling for Ball; player tags while held
  may differ / self-filter.)
- **Ask:** Whether clear-dir SoccerGets ignore `Ball` / own posts / own goal;
  AwayPlayerX vs HomePlayerX exact N spelling if ever not 1–4.

### 16. Held-ball velocity mirrors carrier (real), not zero

- **Where:** `ENGINE` · `CONFIRMED` (baseline: Ball.Vel tracks carrier while held)
- **What:** Real held ball has non-zero velocity matching the dribbler.
- **Sim:** now copies `player.vel` while held (was zeroed in interact + sync).

### 17. Player↔ball contest + pickup delay

- **Where:** `ENGINE`/`SIM`
- **Pickup delay:** Was documented 0.3s **global**. Baseline `05-01-57`: Home
  releases ~~t=0.78, Away O2 has ball by ~t=0.84 (\*\*~~0.06s**). Sim uses **0.06s\*\*.
- **Body bounce:** Loose ball vs `body_r+ball_r` (wall e/mu). Main midfield
  stop in real is **fast re-pickup**, not a fat phantom collider.
- **Forward hold disc:** Unconfirmed hallucination — `BallHoldLocation` is the
  carry/kick origin (~0.9 m along facing); grab reach is still interact_r vs
  hold∪body.

### 18. Tackle: both lose `min(stam)`; remaining keeps ball; equal → carrier

- **Where:** `ENGINE`/`SIM` · `LOCKED` (user 2026-07-23; equal-draw = carrier keeps)
- **What:**
  - `drain = min(attacker.stam, carrier.stam)` applied to **both**.
  - After drain: remaining stam keeps the ball; if both **0** (were equal) →
    **carrier** keeps (tackler must have strictly more remaining to steal).
  - Exchange pickup lockout **0.25s** after win. Failed/equal contest → **0.40s**.
    Frida `tackleRegenDelay=1.5s` not live.
  - Both already at 0 stam: **no** free flip (was oppose-stack tackle lock).

### 19. Shot charge: ~0.30s warmup after pickup, then ~0.38s to 1.0

- **Where:** `ENGINE` · `CONFIRMED` (baseline Home T1 + Away O2)
- **What:** Interact can be true while charge stays 0 for ≈0.30s after claim.
  Then charge steps ≈+0.05 per tick (~52.6 Hz) → full in ≈0.38s (not 0.8s).
  Same warmup on Away after they steal.
- **Sim:** `shot_charge_warmup_s=0.30`, `shot_charge_time_s=0.38`.

### 20. Receiving team + kickoff circle / Away defender chase

- **Where:** `ENGINE`/`SIM` · working assumption
- **What:** Real Away O3 skirts then enters the center circle during Home's
  first charge (X≈5.5, Z 7→3) on a State0-like path — not a radial chase onto
  the carrier's C-lane. Sim: circle clamp only during `Kickoff` phase; suppress
  Away `Ball On Team Side` until first release so Defender stays State0 early.

---

### 21. BallHoldLocation offset = **1.67 m** (prefab local Z)

- **Where:** `ENGINE` · `CONFIRMED` (prefab `BallHoldLocation` local **(0, 0.25, 1.67)**;
  TimePlot |Ball−T1| while held ≈1.54–1.67, first sample ≈1.67)
- **Sim:** `hold_offset_m=1.67`. Capsule body radius **0.762**; NavMeshAgent
  radius 0.5 / stoppingDistance **1.25** (pathfinding only — DeterministicMover
  moves).

### 22. Tackle (locked): both lose `min(stam)`; remainder / tie → ball

- **Where:** `ENGINE`/`SIM` · `LOCKED` (user 2026-07-23)
- **What:** Always `drain = min(T,C)` on both. Remaining stam keeps ball; both
  0 → tackler. Post-steal exchange lockout **0.25s**.

### 23. Post-kick reclaim: ball lockout + **shooter-alone** exclude

- **Where:** `ENGINE`/`SIM` · scrapes + fixed 2026-07-23
- **What:** SoccerBall inspector: *“How long the **shooter alone** cannot
  re-pickup/tackle after releasing a shot…”* (~**3.0s** float). Global after-shot
  delay **0.125s** (ball lockout — nobody). Teammates/opponents may Interact
  mid-air after that (XZ only). Opening dump still has hang + fat hot reclaim.
- **Sim:** `pickup_lockout` = ball; `kick_exclude_shooter` = kicker only (not
  whole team). Old team-wide exclude + mid-air allow let the kicker reclaim
  instantly when Interact flipped back on.

### 24. Kickoff facing ±Z + sticky Clear facing while charging

- **Where:** `ENGINE` · `CONFIRMED` (baseline opening hold)
- **What:** Kicking striker's first hold is pure ±Z (Ball≈(0,±1.67) at T1(0,0)),
  **not** attack ±X. Facing then rotates onto Clear C by ~~t=0.15 and **stays on
  C through Clear→H** during warmup (MoveTo already tracks H; holdFace stays C).
  When warmup ends (~~t=0.35) facing snaps toward Clear H (held-ball Z crash)
  while charge is still 0; full kick ~t=0.75.
- **Sim:** `kickoff_facing` +Y/−Y for kicking T1; carrier facing aims Clear with
  sticky reject of ~90° flips while `charge_warmup_left > 0`.

### 25. Asymmetric post-first-kick chase

- **Where:** `SIM` (parity lever)
- **What:** Home closing on Away carrier ≈0.45× max (contest without sitting on
  Away −Z Clear). Away closing on Home ≈0.95× (reclaims / steals on schedule).

### 26. Away opening Clear / kick bias toward Clear F

- **Where:** `SIM` (parity lever)
- **What:** Baseline Away opening **faces/releases** Clear **F** (−X,−Z). Away
  order is D→F→A…; with sim `blocker_r=body+spherecast`, Home T3 on the
  (−5,−7) skirt blocks F so order falls to **A** (+Z) — O1.Z/Ball.Z flip vs
  Unity DB35 (Unity parks T3 there too; spherecast geometry differs). Lever:
  while Away holds before `first_kick_done`, force **charge facing** + full
  charge **kick dir** to F. Do **not** force Clear-F into MoveTo (that
  overshoots −Z and the dump flies past Home T2). Instead bias Away opening
  carrier MoveTo to a mostly-west lead `pos+(-2,-0.55)` matching Unity's
  charge track (O1≈(−3,−1.2) at release).

### 27. Held ball vs walls — clamp rotation / rub / walk-through bug

- **Where:** `ENGINE` · **OBSERVED** (user 2026-07-23,
  `PerfectController`)
- **Status:** **not yet TimePlot-locked**. These are hands-on observations,
  not confirmed collider or transform measurements.
- **Observations:**
  1. **Intended-ish clamp:** the player cannot rotate facing into a wall when
     that would put the held ball outside the pitch bounds.
  2. **Sideways rub:** after moving sideways to a wall, rotating toward it
     makes the ball rub along the wall while staying in bounds; the player
     cannot finish a “look down” into the wall.
  3. **Walk-through bug:** walking straight into a wall can push the held ball
     behind / outside the wall. Turning `Interact` off to kick is ignored
     there—no launch impulse; the ball stays near the hold / kick origin and
     slowly rolls back onto the pitch.
  4. **180° sweep loophole:** continuous left↔right yaw can sweep the facing
     and held ball through wall geometry even when discrete into-wall rotation
     is blocked. The clamp is therefore path-dependent.
- **Author reproduction tip:** load `PerfectController`, park the other
  players, hold `E` while using WASD at a wall, and try both a sideways rub
  and continuous 180° yaw. Offline reproduction:

  ```bat
  cargo run --release -- --home perfect --away idle
  ```

- **Sim:** Not mirrored yet; keep this as an upstream observation until
  hold-bounds and out-of-bounds held-kick rejection are measured.
- **Parity bar (2026-07-23):** Do **not** chase bit-perfect player move speed /
  rotation vs Unity while held. The wall rub / walk-through / path-dependent
  clamp injects **small unpredictable** residuals. Aim for **practical**
  trajectory parity (Ball / O1 / T1 / MoveTo shape and timing); treat leftover
  sub-meter jitter near walls as Unity noise, not a sim bug to overfit.

---

## Sim parity checklist (ours — not upstream)

### Parity invariant (critical)

**Never compare Unity against a simulation that starts from a different state.**

Parity is **not**: read Unity ball/player at some tick, then let the sim use a
“better” spawn, recomputed equivalent, or current defaults and call the RMSE a
match. That measures two different simulations.

Instead:

1. Take the Unity reference TimePlot / freeze **exact** observable state.
2. Inject that identical state into the sim.
3. Run forward.
4. Compare trajectories (e.g. `scripts/first2s_rmse.py`).

Frozen state includes **everything observable that affects AIA**, not just
positions: ball pos/vel, holder/held flags, player positions (and velocities if
available), facing/rotation if exposed, stamina/charge, possession, kickoff,
match/phase timers, cooldowns/lockouts, RNG seed / randomized decisions, and
every API value visible to the graph.

**One independent variable at a time:** when a parameter is calibrated from
Unity (friction, restitution, delay, threshold, …), freeze it and rerun parity
from the **same** initial snapshot. Never recalibrate while also changing the
initial state — an RMSE improvement then cannot be attributed to either change.

Also: do not change `FIXED_DT` / AIA clocks for fast scrubber (wall idle only);
cosmetic GoalPause delay may be stripped, but kickoff/hold/first kick/MoveTo
stay always-on.

**Practical bar:** Unity held-ball can still collide with board walls (quirk
#27) in a path-dependent / buggy way, so tiny move/rotation residuals are
expected. Stop chasing once Ball / O1 / T1 / MoveTo first-~2s shapes and
timings are practically close — not bit-perfect.

| Item            | Notes                                                                           |
| --------------- | ------------------------------------------------------------------------------- |
| Opp-goal dir    | Clear lane into mouth or null; goal-dir uses interact_r                         |
| ClearMate       | LOS + short corridor + other mates block; MIN_DOT 0.93; mate_r body×2.8; max 36 |
| Kicking striker | Spawns at (0,0) when kicking off; kickoff face ±Z (#24)                         |
| Speeds          | walk **7** / sprint **8**; no stam throttle (#11); Clear-sticky facing (#24)    |
| Clear blockers  | body1.5 + continuous closest-approach ray                                       |
| Charge          | 0.30s warmup + 0.38s to full (#19)                                              |
| Hold offset     | **1.67 m** prefab BallHoldLocation Z (#21); body capsule **0.762**              |
| Kickoff / Away  | Kickoff-phase circle clamp; suppress Away team-side + P3-closest                |
| Tackle          | both lose `min(stam)`; remainder keeps / equal→carrier; lockout 0.25s (#18/#22) |
| Pickup / loose  | Hot window 0.25s; no hang body-claim; settle `<2 m/s` (#23)                     |
| Held-ball vel   | Carrier vel (#16)                                                               |
| Early Ball      | **t<=2 X~~0.84 Z~~1.14** (DB35 Away); O1.Z~~0.41 after Clear-F face+capped lane |
| Loose / OppHas  | early/mid match good; full-match averages shift after late goals                |
| Chase           | Home 0.45× / Away 0.95× (#25); Away full kick → F (#26)                         |
| Mid Ball        | t<=5 X~~5.7 Z~~8.3; t=5 Ball≈(−17,−20) vs (−22,−22) — close                     |
| ClearMate mix   | T2≈0.62 matches; T1 0.23 vs 0.28; T3 0.68 vs 0.93; T4 0.43 vs 0.38              |
| Kick launch     | `horiz=min((10+290c)/9,29.42)`; lift `max(0,-0.323+6.667c)`;                    |
| Ground slide    | Coulomb 5.95 **only while grounded**; airborne XZ coasts (Y hang ⇒ carry)       |
| Whistle reset   | Positions/ball/charge snap to kickoff; **stamina persists** (no free refill)    |
| Stamina rates   | Drain **~34.5s** empty; regen **~20s** full; no snap@0; sprint 8 @ empty (#11)  |
| Kick flick      | Interact↓ aims **MoveTo**, not hold/facing — instant 90° OK (17-11-17 DB11)     |

### Whistle / kickoff: positions only (`CONFIRMED`)

- **Where:** `ENGINE` · `CONFIRMED` (observation + TimePlot 16-56-03)
- **What:** Stale whistle forces players + ball back to kickoff spots and
  restarts the round. **Stamina does not reset** and does not get a free regen
  bump — depleted stamina carries into the next kickoff. Charge/warmup clear
  with the reposition.
- **Sim:** `place_kickoff` already keeps `stamina` / `stamina_regen_lock_left`.

### Discord: stamina snap-refill at 0 (`NOT SEEN`)

Unlucky claimed instant refill at 0. TimePlot **17-05-04 DB14** hit ~0.05 and
regenerated smoothly at **0.05/s** (~20s full) — no snap. Drain while carrying

- sprinting: **−0.029/s** (~**34.5s** empty).

---

## Suggested report blurb (copy/paste)

> AIA Soccer notes from TimePlot parity work:
>
> 1. `Direction of opponent goal from Teammate N` is null most of the time; it
>    only returns an 8-way dir when a lane into the goal mouth is clear—not a
>    raw vector to goal center. `Set First Direction` therefore usually uses
>    clear-dir instead. Please document or rename.
> 2. `StrikerKickoffPos = 0` when kicking off is a **MoveTo** walk-in target;
>    engine spawn keeps both strikers on ±(1,7) wings with free ball at center.
> 3. `StrikerState1 = ClearDir * 2` omits adding player position (unlike
>    Playmaker). Likely a graph bug.
> 4. `StrikerSprint` never true while chasing a loose ball (all branches need
>    Has Ball). Walk speed is already ~7 m/s so it still closes—but sprint is
>    dead for press.
> 5. Striker `Ready Shoot/Pass` uses OppGoal for **both** dirs, so Ready stays
>    false until full charge (kick at 1.0). Playmaker can Ready earlier via
>    ClearMate. Intentional?
> 6. Soccer walk≈7 / sprint=8; Survival 4.5/9 does not match. Sprint speed does
>    **not** drop at stam≈0 (always-sprint TimePlot).
> 7. Opening kickoff hold faces **world ±Z** (~1.67 m), not attack ±X; facing
>    tracks Clear with sticky C→H during charge warmup.
> 8. Tackle: both lose `min(stam)`; remaining keeps ball; equal→both 0→carrier
>    keeps. Frida 1.5s tackle regen delay not live.

Update this file whenever a new confirmed quirk shows up.

---

## Coverage gaps (need more TimePlots / Frida)

What is still soft / optional:

1. _(tackle min-drain rule locked 2026-07-23 — both lose min(stam); remainder
   keeps / both 0 → carrier)_

### Locked (no new capture needed)

- Air planar drag = **0** while airborne
- Vertical bounce **e≈0.23**
- Hang vs charge (~0.28s@0.5 → ~0.61s@full)
- Stamina drain **~34.5s** / regen **~20s**; no snap@0; sprint works to empty
- Sprint-off regen delay = **0** (immediate next tick; Frida 1.0s not live)
- Walk **7** / sprint **8** m/s; **no speed throttle at stam≈0** (TimePlot
  18-53-07 DB2 always-sprint; med 8.0 through stam≤0.01 / floor ~1.5e-4)
- Kick flick: Interact↓ = **MoveTo** dir (TimePlot 17-11-17); sim already matches
- Exchange pickup lockout **0.25s** on tackle win
- Pickup airborne uses real grounded Y
- Facing turn: **angularSpeed=2500 deg/s** locked (TimePlot 17-46-22 DB18).
  Yaw steps are linear at 47.5° per FixedDt≈0.019s (=2500). `Forward.X` looks
  nonlinear because it is **cos(θ)** — peak |dForward.X/dt|≈40 is not deg/s.
  180 finishes in 0.095s (1 tick lag + 4 steps; last step 37.5°). Sticky
  charge-warmup reject of ~90° flips still applies while carrying.
- Post-goal pause → kickoff reset **~4.9s** (TimePlot 17-37-34); Frida 1.0s is
  not that freeze.
- Tackle: **both lose `min(stam)`**; remaining keeps ball; **equal → both 0 →
  carrier keeps** (user 2026-07-23b). tackleRegenDelay Frida 1.5s **not live** →
  wired 0.
- **Is \* Player N Open** (AIA lock 2026-07-22): true iff **no opposing player**
  is within **`2 × Player Interact Radius`** of that player
  (`nearest_opp_dist > 2×R`). Else false. Same rule for team and opponent
  slots; `Get nearest/most/furthest open *` only consider players that pass
  this test.

---

## Post-parity backlog (do **not** start during TimePlot loop)

Order: **parity → JIT → function inlining in JIT → (optional) pure-math task DAG**.

### Pure math task-graph scheduler (optional, after JIT+inline)

Idea: split graph nodes into Pure (stateless math) vs Impure (reads/writes
volatile world). Each tick: scan pure deps feeding impure sinks → dispatch a
DAG to worker threads → main thread runs impure nodes, waiting on cached pure
outputs.

Only worth it **after** JIT + inlining: interpreted per-node tasks lose to
sync overhead; JITed/inlined pure _chains_ (or whole sub-DAGs) are the right
grain. Candidate pool: `[micropool](https://crates.io/crates/micropool)`.

Gotchas to remember: trivial-task overhead; snapshot volatile SoccerGet
inputs before workers run; do not let pure workers touch live transforms.
