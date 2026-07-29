# Agent Guide — AIComp Soccer Sim

This crate is an **offline game copy** for AI development. Self-test with headless matches and `cargo test` instead of asking users to run Unity.

## Build & Run

```bat
cargo run                              # Viewer (dev, fast incremental)
cargo run --release                    # Viewer (release)
cargo run --release --bin soccer_headless -- --secs 30 --home chase --away chase
cargo test --lib                       # Unit / parity tests
```

**Fast edit → compile loop:** `cargo run` (dev) rebuilds only this crate. Bevy stays cached in `target/debug`. Do not switch features or profiles between small edits — that triggers a full Bevy rebuild.

**If the build is stale:**
1. `scripts\rebuild_crate.bat` — this crate only
2. `scripts\rebuild_deep.bat` — full rebuild (last resort)

## Key Rules

- **Physics constants live in two places:** `src/params.rs` (Rust defaults) and `bevy_sim_params_v05.json` (runtime override). The JSON **overrides** Rust at load time. Edit both or only the JSON for runtime changes.
- **Measured constants are locked.** `params::measured_constants_tests` asserts values from real-game recordings. If a test fails, re-measure — do not change the expected value.
- **Do not compare Unity to a sim that started from a different state.** Freeze the Unity snapshot, inject it, then measure RMSE.
- **NN train harness** is gated behind `--features nn_train` to keep default compiles fast.

## Truth Files

| File | Purpose |
|------|---------|
| `docs/AIA_UPSTREAM_QUIRKS.md` | Locked Unity measurements |
| `docs/API_GAPS.md` | Unimplemented API getters |
| `bevy_sim_params_v05.json` | Runtime physics constants |
| `data/reference/` | API dumps and measured data (read-only) |

## Brain Types

`chase` | `idle` | `test1` | `test2` | `perfect` | `aia` | `graph:<path>`

Use `soccer_headless` for batch testing. Parse stdout JSON for results.
