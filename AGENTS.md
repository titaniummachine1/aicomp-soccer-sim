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

1. Pick brains (`chase`, `idle`, `test1`, `test2`, `aia`, `graph:<path>`).
2. Run many headless jobs with different `--seed` / openings.
3. Parse **stdout JSON** (`ok`, `score_home`, `score_away`, `clock_s`, `ticks`).
4. Aggregate win rates / goal diffs externally.

Example one-shot:

```bat
cargo run --release --bin soccer_headless -- --secs 60 --home chase --away chase --seed 7 --until-goal --quiet
```

## Scripted parity probes

```bat
cargo run --release --bin debug_tackle_test -- 45
```

Exit `0` = steal seen; `2` = failed assert.

## Truth files

| File | Use |
|------|-----|
| `docs/AIA_UPSTREAM_QUIRKS.md` | Locked Unity measurements + sim checklist |
| `bevy_sim_params_v05.json` | Live numbers (crate root) |
| `data/reference/` | API / Frida dumps (read-only) |
| `README.md` | Human entry points |

## Don't

- Don't block on Unity for “does my brain score?” — use `soccer_headless`.
- Don't invent Survival speeds (4.5/9). Soccer is walk **7** / sprint **8**.
- Don't re-open locked quirks without a new TimePlot path from the user.
