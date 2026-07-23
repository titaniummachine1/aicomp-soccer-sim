# Agent guide — AIComp Soccer Sim

Use this crate as the **offline game copy**. Do **not** ask the user to run
Unity / TimePlots for routine AI evaluation. Self-test with headless matches
and `cargo test`.

## Always prefer

```bat
cargo test --lib
cargo run --release --bin soccer_headless -- --secs 20 --home chase --away idle
```

Viewer only when you need eyes on a bug:

```bat
cargo run --release
```

## Fishtest-style batch

1. Pick brains (`chase`, `idle`, `test1`, `test2`, `perfect`, `aia`, `graph:<path>`).
2. Run many headless jobs with different `--seed` / openings.
3. Parse **stdout JSON** (`ok`, `score_home`, `score_away`, `clock_s`, `ticks`).
4. Aggregate win rates / goal diffs externally.

Example one-shot:

```bat
cargo run --release --bin soccer_headless -- --secs 60 --home chase --away chase --seed 7 --until-goal --quiet
```

Parallel match micropool (`--batch` JSONL, `--jobs` pool size; default jobs =
logical CPUs − 1 so one core stays free for system/UI):

```bat
cargo run --release --bin soccer_headless -- --batch 16 --jobs 8 --secs 20 --home chase --away idle --seed 0 --quiet
```

Seeds are `base_seed + i` (base defaults to 0). Opening follows seed parity
(even=home, odd=away). Each stdout line is one JSON match result.

## Scripted parity probes

```bat
cargo run --release --bin debug_tackle_test -- 45
```

Exit `0` = steal seen; `2` = failed assert.

## Truth files

| File                          | Use                                       |
| ----------------------------- | ----------------------------------------- |
| `docs/AIA_UPSTREAM_QUIRKS.md` | Locked Unity measurements + sim checklist |
| `docs/API_GAPS.md`            | Blank nodes + unsure SoccerGet semantics  |
| `bevy_sim_params_v05.json`    | Live numbers (crate root)                 |
| `data/reference/`             | API / Frida dumps (read-only)             |
| `README.md`                   | Human entry points                        |

## Don't

- Don't block on Unity for “does my brain score?” — use `soccer_headless`.
- Don't invent Survival speeds (4.5/9). Soccer is walk **7** / sprint **8**.
- Don't re-open locked quirks without a new TimePlot path from the user.
- **Never compare Unity to a sim that started from a different state.** Freeze
  the Unity snapshot (ball/players/vels/hold/timers/API-visible state), inject
  it, then measure RMSE. Changing spawn/defaults while “matching” Unity is a
  different simulation, not parity. One independent variable at a time: if you
  calibrate a param from Unity, freeze it and rerun from the **same** initial
  snapshot — never recalibrate and change initial conditions together.
- Don't chase bit-perfect move/rotation while held — Unity held-ball vs walls
  is buggy/noisy (see quirks #27). Practical first-~2s Ball/O1/T1/MoveTo is
  the bar.
