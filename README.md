# AIComp Soccer Sim (Rust / Bevy 2D)

Fully **2D** top-down close copy of AIComp Soccer physics + controls.
Unity’s 3D meshes are visual-only — this repo does not simulate Y.

## Run

```bat
cargo run
```

Fast rebuild loop (optional):

```bat
cargo install cargo-watch
cargo watch -x run
```

Hot-reload params from `bevy_sim_params_v05.json`: press **R**.

## Layout

| Path | Role |
|------|------|
| `src/` | Sim + top-down view |
| `bevy_sim_params_v05.json` | Numbers pack |
| `SOCCER_GAME_MODEL.md` | Human rules / confidence tags |
| `soccer_ball_sim_params.json` | Full ball-sim pack (reference) |

Python mining / graphs stay in `../worldcupteams` — not mixed in here.
