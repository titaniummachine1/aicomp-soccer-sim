# AIA / AIComp Soccer — quirks & bug-like behavior

Living list of unnatural, surprising, or likely-buggy behaviors seen in the
**real game** (via AIA_Debug TimePlots) and in AIA’s own graph. Use this to
report upstream to AIA / Unicorn One for possible patches.

Evidence baseline: `timeplot_2026-07-22_05-01-57.json` (Home=AIA_Debug build 2,
Away=AIA, ~23s, no duplicate channels).

Status legend: `ENGINE` = Unity/AIComp soccer runtime · `BOT` = AIA graph logic ·
`CONFIRMED` = measured in TimePlot · `SUSPECTED` = inferred from graph + plots.

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

### 2. Kicking striker spawn = `**(0,0)` on the ball\*\*

- **Where:** `BOT` + `ENGINE` · `CONFIRMED`
- **What:** AIA sets `StrikerKickoffPos = Vector3Zero` when `Is Team Kicking off`.
  Real sample0 with Home kicking: **T1 = (0, 0)** (on the ball). Kickoff flag
  clears by **t≈0.08s**. Instant pickup / first touch.
- **Why weird:** Faceoff docs / other slots use ±(1,7) style; Zero reads like a
  “default” but actually places the kicker on the ball.
- **Ask:** Confirm intended. If not, use a circle-edge faceoff (e.g. (0, ±r) or
  ±(1,7)) when kicking off.

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

### 11. Locomotion without Sprint ≈ 7 m/s

- **Where:** `ENGINE` · `CONFIRMED` (Away O1 chase, Sprint=0, mean ~6.4–7 m/s)
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

### 15. Clear-dir spherecast radius is slim (≈body); goal-dir is thicker

- **Where:** `ENGINE` · `SUSPECTED`→working assumption (split radii)
- **What:** General clear-dir matches better with slim ≈`body_radius * 1.1`.
  Goal-mouth dirs need a thicker probe (≈`Player Interact Radius` 1.75): slim
  midfield E often misses an opponent by <1 m → false Present → Ready at
  charge≈0.5. Real OppGoal Present ~12%, first ~t=14.5 near x≈35 (AIA: null if
  no clear sensor dir of the goal).
- **Ask:** Publish spherecast radius/length per getter family.

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

### 18. Tackle: strict `>` after drain + live carrier stam (no same-tick ping-pong)

- **Where:** `ENGINE` · `CONFIRMED` (SOCCER_GAME_MODEL) / was `SIM` bug
- **What:** Winner needs **more** stamina after mutual drain (carrier keeps ties).
  Sim also snapshotted carrier stam once per tick and applied drains after the
  player loop → later tacklers dueled a stale (often 0) stam and stole every
  player in the same frame (Away charge never reached 1.0). Fixed: live stam,
  immediate drain, lockout after steal/kick blocks further tackle/pickup.
- **Ask:** Confirm tie rule + whether steal has a short lockout in engine.

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

### 21. BallHoldLocation offset ≈ **1.60 m** (not body+ball ≈0.9)

- **Where:** `ENGINE` · `CONFIRMED` (baseline |Ball−T1| while held ≈1.54–1.67;
  first held sample ≈1.67)
- **Sim:** `hold_offset_m=1.60`. Old body+ball≈0.9 under-shot early Ball.Z.

### 22. Tackle: failed probes drain 0; success = full mutual min-drain

- **Where:** `ENGINE`/`SIM` · working assumption (updated from TimePlot)
- **What:** Opening probes must not attrit (tiny mutual drip left Home 0.992 <
  Away 0.993 → StrikerPos → TriangulatedOffPos). Successful steals deduct
  **min(attacker,carrier)** from **both** (real t≈1.36: T1 and O2 → 0). Equal
  stam after drain: **attacker wins** only when carrier `shot_charge ≥ 0.5`
  (mid-charge contest); otherwise carrier keeps (protects first Away touch).
  Post-steal lockout **0.40s** so a fresh full-stam teammate doesn't instantly
  yo-yo the zeroed carrier (real Home holds ~1.38–1.75 at stam 0).

### 23. Post-kick reclaim: hot window only, no hang body-snatch

- **Where:** `SIM` (2D stand-in for ~1s hang)
- **What:** Kicking team excluded ~2.5s. Opponents get ~0.25s fat interact
  (+1.0 m) for instant reclaim (Away O2 after Home release). **No** body-claim
  during hang — long flights (real t≈3.5–10) stay loose. After hang: body claim
  only if `ball_speed < 2`. Hot balls (>8 m/s) skip player-body bounce.

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

### 26. Away full-charge kick bias toward Clear F

- **Where:** `SIM` (parity lever)
- **What:** Baseline Away long release is v≈(−21,−21) = Clear F. Away Clear
  order prefers D (−X), so sim releases were v≈(−21,−11). When Away
  `shot_charge≥0.75`, `pos.y < -1`, and facing near −X, snap kick dir to F.

---

## Sim parity checklist (ours — not upstream)

| Item            | Notes                                                                               |
| --------------- | ----------------------------------------------------------------------------------- |
| Opp-goal dir    | Clear lane into mouth or null; goal-dir uses interact_r                             |
| ClearMate       | LOS + short corridor + other mates block; MIN_DOT 0.93; mate_r body×2.8; max 36     |
| Kicking striker | Spawns at (0,0) when kicking off; kickoff face ±Z (#24)                             |
| Speeds          | ~7.5 m/s, accel ~45; Clear-sticky facing; snap after warmup (#24)                   |
| Clear blockers  | body1.5 + continuous closest-approach ray                                           |
| Charge          | 0.30s warmup + 0.38s to full (#19)                                                  |
| Hold offset     | 1.60 m (#21)                                                                        |
| Kickoff / Away  | Kickoff-phase circle clamp; suppress Away team-side + P3-closest                    |
| Tackle          | Full mutual drain on success; equal→attacker if carrier ch≥0.5; lockout 0.40s (#22) |
| Pickup / loose  | Hot window 0.25s; no hang body-claim; settle `<2 m/s` (#23)                         |
| Held-ball vel   | Carrier vel (#16)                                                                   |
| Early Ball      | **t<=2 X~~0.77 Z~~1.22**; Zt2[-5.2,2.1] vs[-4.5,1.9]; t<=3 ~0.9/2.0                 |
| Loose / OppHas  | early/mid match good; full-match averages shift after late goals                    |
| Chase           | Home 0.45× / Away 0.95× (#25); Away full kick → F (#26)                             |
| Mid Ball        | t<=5 X~~5.7 Z~~8.3; t=5 Ball≈(−17,−20) vs (−22,−22) — close                         |
| ClearMate mix   | T2≈0.62 matches; T1 0.23 vs 0.28; T3 0.68 vs 0.93; T4 0.43 vs 0.38                  |
| Kick launch     | `horiz=min((10+290c)/9,29.42)`; lift `max(0,-0.323+6.667c)`;                        |
| Ground slide    | Coulomb 5.95 **only while grounded**; airborne XZ coasts (Y hang ⇒ carry)           |
| Whistle reset   | Positions/ball/charge snap to kickoff; **stamina persists** (no free refill)        |
| Stamina rates   | Drain **~34.5s** empty; regen **~20s** full (TimePlot 17-05-04 DB14); no snap@0     |
| Kick flick      | Interact↓ aims **MoveTo**, not hold/facing — instant 90° OK (17-11-17 DB11)         |

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
> 2. `StrikerKickoffPos = 0` when kicking off places the striker **on the ball**
>    at (0,0). Is that intended?
> 3. `StrikerState1 = ClearDir * 2` omits adding player position (unlike
>    Playmaker). Likely a graph bug.
> 4. `StrikerSprint` never true while chasing a loose ball (all branches need
>    Has Ball). Walk speed is already ~7 m/s so it still closes—but sprint is
>    dead for press.
> 5. Striker `Ready Shoot/Pass` uses OppGoal for **both** dirs, so Ready stays
>    false until full charge (kick at 1.0). Playmaker can Ready earlier via
>    ClearMate. Intentional?
> 6. Please publish Soccer walk/sprint/accel; Survival 4.5/9 does not match.
> 7. Opening kickoff hold faces **world ±Z** (~1.67 m), not attack ±X; facing
>    tracks Clear with sticky C→H during charge warmup.
> 8. Successful tackle deducts min(stam) from **both**; equal mid-charge contests
>    go to attacker (carrier keeps ties only before mid-charge).

Update this file whenever a new confirmed quirk shows up.

---

## Coverage gaps (need more TimePlots / Frida)

What is still soft / optional:

1. **Tackle regen delay 1.5s** — Frida candidate; first TackleRegen capture
   missed (Away body-chased; equal-stam before first kick). Reload fixed
   TackleRegenProbe/Away (chase ball + short kick then bait).

### Locked (no new capture needed)

- Air planar drag = **0** while airborne
- Vertical bounce **e≈0.23**
- Hang vs charge (~0.28s@0.5 → ~0.61s@full)
- Stamina drain **~34.5s** / regen **~20s**; no snap@0; sprint works to empty
- Sprint-off regen delay = **0** (immediate next tick; Frida 1.0s not live)
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
