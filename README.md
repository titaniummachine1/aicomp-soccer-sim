# AIComp Soccer Sim

**Reverse-engineered 2D close copy of AIComp Soccer** for offline AI work.
Agents should **self-test here** (headless / unit tests) instead of asking a
human to click around in Unity.

Measured quirks live in [`docs/AIA_UPSTREAM_QUIRKS.md`](docs/AIA_UPSTREAM_QUIRKS.md).
Agent-oriented commands: [`AGENTS.md`](AGENTS.md).
Reference dumps: [`data/reference/`](data/reference/).

| Need | Command |
|------|---------|
| **Viewer** (window) | `cargo run --release` |
| **Headless match** (JSON) | `cargo run --release --bin soccer_headless -- --help` |
| **Unit / parity tests** | `cargo test --lib` |
| **Scripted tackle assert** | `cargo run --release --bin debug_tackle_test -- 45` |

First Bevy compile is slow; after that edit→run is fine. Use `--release` for
batch / speed.

---

## Quick start

```bat
rustup update
cd aicomp-soccer-sim
cargo build --release
cargo run --release
cargo run --release --bin soccer_headless -- --secs 20 --home chase --away idle
```

Or double-click / run:

- [`scripts/run_viewer.bat`](scripts/run_viewer.bat)
- [`scripts/run_headless.bat`](scripts/run_headless.bat)

---

## Headless (fishtest-style)

No window. Fixed dt `0.019`. One **JSON result on stdout** per run — aggregate
these for strength tests / CI.

```bat
cargo run --release --bin soccer_headless -- --secs 30 --home chase --away chase
cargo run --release --bin soccer_headless -- --secs 40 --home test1 --away test2
cargo run --release --bin soccer_headless -- --home aia --away aia --until-goal --secs 90
cargo run --release --bin soccer_headless -- --home graph:C:\path\Team.txt --away idle --json out.json
```

**Brains:** `chase` | `idle` | `test1` | `test2` | `aia` | `graph:<path>`

**Exit:** `0` ok · `1` bad args / load fail

Stdout example:

```json
{"ok":true,"fixed_dt":0.019,"secs_requested":30.0,"clock_s":30.0,"ticks":1579,"opening":"home","seed":null,"home":"chase","away":"idle","score_home":0,"score_away":0,"phase":"Playing","until_goal":false,"goal_stopped":false}
```

Parallelize **across matches** (many processes / jobs), not with 2 threads per match.

### Lib loop (custom brains)

```rust
use aicomp_soccer_sim::{ChaseBallBrain, IdleBrain, MatchWorld, TeamId, FIXED_DT};

let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
let mut home = ChaseBallBrain;
let mut away = IdleBrain;
while world.match_state.clock_s < 30.0 {
    world.step_brains(&mut home, &mut away, FIXED_DT);
}
// read world.match_state.score_home / score_away
```

---

## Viewer

```bat
cargo run --release
```

- **Space** — pause / resume  
- **R** — reload params  
- Loads `AIA.txt` from AIComp Saves when present; otherwise chase fallback  

---

## Other bins

| Bin | Purpose |
|-----|---------|
| `soccer_sim` | Bevy top-down viewer (default) |
| `soccer_headless` | Batch matches → JSON |
| `debug_tackle_test` | Test1 vs Test2 steal; exit `2` if no steal |
| `timeplot_until_goal` | AIA vs AIA TimePlot JSON under Saves |

---

## Model rules (cheap core)

- **No navmesh** — pitch AABB clamp only  
- Ball: Coulomb **5.95**, wall **e≈0.2**, open mouths score  
- Players: walk **7** / sprint **8**; stamina does **not** throttle speed  
- Interact / tackle radius **1.75**; equal stam keeps carrier; higher stam steals (no mutual dump)  
- Fixed dt **0.019** (~52.6 Hz), independent of render FPS  

Params: `bevy_sim_params_v05.json` (next to exe or crate root).
