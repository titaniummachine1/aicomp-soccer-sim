# Agent guide — AIComp Soccer Sim

Use this crate as the **offline game copy**. Do **not** ask the user to run
Unity / TimePlots for routine AI evaluation. Self-test with headless matches
and `cargo test`.

## Hard requirement — fastest edit → compile → run

**Assume the normal workflow is: edit only files in `src/`, then `cargo run`.
Optimize for that.** Any solution that causes Bevy or other heavy dependencies
to rebuild for tiny code changes is a **regression** unless absolutely
unavoidable — and must be **warned about explicitly before suggesting**.

Expected loop:

- Small changes to our code (`src/`).
- `cargo run` (dev / default features).
- Compile should be **near-instant**, rebuilding **only this crate**.
- **Never** treat a Bevy (or other large-dep) recompile after normal edits as OK.

Full dependency rebuild is only expected after:

- first build after clone,
- `cargo clean` / `scripts\rebuild_deep.bat`,
- changing Bevy version or its Cargo features,
- changing Cargo profiles / rustflags / `.cargo/config.toml` linker flags,
- first use of a different profile (`--release` vs debug),
- switching feature sets that actually affect dependencies (`nn_train`,
  `--no-default-features`, etc.).

Ordinary gameplay / AI / sim edits **must not** trigger a Bevy rebuild. Fast
incremental compilation is a **hard requirement**. Preserve it unless there is
no technically feasible alternative.

Graceful degradation if something looks wrong (do **not** jump to Bevy):

1. `scripts\rebuild_crate.bat` — this package only
2. `scripts\rebuild_deep.bat` — Bevy + deps (last resort; multi-minute)

Optional: `scripts\cargo_hot.bat …` for HIGH priority + max `-j`.

## Always prefer

```bat
cargo test --lib
cargo run --release --bin soccer_headless -- --secs 20 --home chase --away idle
```

Viewer (incremental — **dev**, not release):

```bat
cargo run
```

`fast_link` is **default** — Bevy stays prepared in **`target/debug`** for
`cargo run`. Release is a **separate** cache; preparing `--release` does **not**
warm the next `cargo run`.

**NN train harness** (`src/train`, brain `trained`) is **gated off by default**
(`nn_train` feature) so slow compiles are not mistaken for training failure.
Code is intact. Re-enable: `--features nn_train`.

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

## Physics constants live in TWO places

`SimParams::fallback()` in `src/params.rs` **and** `bevy_sim_params_v05.json`.
The JSON **overrides** the Rust defaults at load, so editing only `params.rs`
changes nothing at runtime. This has caused silent no-op "fixes" three times
(`bounce_e`, `body_radius`, the field AABB).

`params::measured_constants_tests` guards it: it loads params the way the sim
does and asserts the values measured from real-game TimePlots. If it fails,
**re-measure — do not edit the expected value to match.** Those numbers come
from recordings, and several sim defaults that looked authoritative (prefab
colliders, Unity's 9.81) were simply wrong.

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
