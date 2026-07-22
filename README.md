# AIComp Soccer Sim (Rust / Bevy 2D)

Cheap **2D** close copy of AIComp Soccer for offline testing and mass batch sims.

## Rules of the cheap model

- **No navmesh / agents** — pitch is empty; walk limits are the ball AABB.
- **Ball** is the only sliding physics body (Coulomb a=5.95, wall e≈0.2, open mouths score).
- **Players** do not collide with the ball; pickup/tackle uses **interact radius 1.75**.
- **No goal entry for players** (for now). Later: if `moveTo` is inside a goal, sweep a collision circle vs posts+walls — go straight if clear, else pathfind. Deferred to keep batch sims cheap.
- Fixed dt **0.019** (from ApiProbe; independent of render FPS).

## Run viewer

```bat
cargo run
```

## Headless batch (lib)

```rust
use aicomp_soccer_sim::{ChaseBallBrain /* via brain */, MatchWorld, SimParams, FIXED_DT};
// MatchWorld::new_kickoff(params).step_brains(&mut home, &mut away, FIXED_DT);
```

Parallelize **across matches**, not with 2 threads per match.
