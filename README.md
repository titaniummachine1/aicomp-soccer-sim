# AIComp Soccer Sim

A reverse-engineered 2D simulation of [AIComp Soccer](https://github.com/UnicornOne/AIComp) for offline AI development and testing. Built with [Bevy](https://bevyengine.org) in Rust.

## Quick Start

**Easiest way:** double-click `run_simulation.bat` in the repository root.

**From terminal:**

```bat
cargo run --release
```

This opens the Bevy viewer with a top-down pitch view. Press **Space** to pause, **R** to reload params.

## Features

- **2D physics simulation** matching the real game's ball, player movement, and interaction rules
- **Graph VM** that loads and executes the same AI graph format the game uses
- **Headless mode** for batch testing — outputs JSON results per match
- **TimePlot recording** for position/velocity/charge data collection
- **Scripted test brains** for reproducible parity checks

## Commands

| Task | Command |
|------|---------|
| Viewer (windowed) | `cargo run --release` or `run_simulation.bat` |
| Headless match | `cargo run --release --bin soccer_headless -- --secs 30 --home chase --away chase` |
| Unit tests | `cargo test --lib` |
| Tackle test | `cargo run --release --bin debug_tackle_test -- 45` |

### Available brains

`chase` | `idle` | `test1` | `test2` | `perfect` | `aia` | `graph:<path>`

### Headless example

```bat
cargo run --release --bin soccer_headless -- --secs 60 --home chase --away chase --seed 7 --until-goal
```

Output (one JSON object per match):

```json
{
  "ok": true,
  "score_home": 2,
  "score_away": 1,
  "clock_s": 60.0,
  "ticks": 3158
}
```

## Project Structure

```
src/
  main.rs            Bevy viewer entry point
  world.rs           Core simulation (fixed-dt tick, player/ball/interaction)
  possession.rs      Tackle/pickup/charge logic
  ball.rs            Ball physics (Coulomb friction, wall bounces, goals)
  player.rs          Player movement, stamina, hold-point
  params.rs          Sim constants (measured from real game)
  brain.rs           Brain trait + command types
  api/               SoccerGet* API layer (what graphs see)
  graph/             Graph loader (JSON → node tree)
  graph_vm/          Graph VM (interpreter, optimizer, runtime brain)
  probe_brains.rs    Scripted test brains (Test1, Test2, PerfectController)
  timeplot.rs        TimePlot recorder (JSON output)
  bin/               Additional binaries (headless, tests, drills)
scripts/             Helper scripts (build, deploy, analyze)
docs/                Documentation and measured quirks
data/reference/      API dumps and measured constants from the real game
```

## Physics Model

| Parameter | Value | Source |
|-----------|-------|--------|
| Fixed dt | 0.019s (~52.6 Hz) | Measured |
| Player walk speed | 7 m/s | Measured |
| Player sprint speed | 8 m/s | Measured |
| Interact/tackle radius | 1.75 m | Measured |
| Body collision radius | 0.655 m | Measured |
| Ball friction (Coulomb) | 5.95 m/s² | Measured |
| Wall restitution | 0.2 | Measured |
| Kick speed formula | (10 + 290c) / 9, capped at 30 | Measured |
| Gravity (ball height) | 17.0 m/s² | Measured |

All constants are measured from real-game TimePlot recordings. See `docs/AIA_UPSTREAM_QUIRKS.md` for details.

## Documentation

- [`docs/SOCCER_GAME_MODEL.md`](docs/SOCCER_GAME_MODEL.md) — Game model reference
- [`docs/AIA_UPSTREAM_QUIRKS.md`](docs/AIA_UPSTREAM_QUIRKS.md) — Measured quirks and parity checklist
- [`docs/API_GAPS.md`](docs/API_GAPS.md) — Unimplemented or uncertain API getters
- [`AGENTS.md`](AGENTS.md) — Guide for AI agents working on this codebase

## License

MIT
