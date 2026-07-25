# Handoff — Unity AIComp Soccer **v0.63** → sim

**Date:** 2026-07-25  
**Unity build:** `C:\gitProjects\worldcup\Worldcupv0.63\` (`GameAssembly.dll` 126 784 000 vs v0.61 126 776 320)  
**PyLibrary:** `worldcupteams/AIGamePyLibrary` @ `f900f96` (`fixed outdated debug node`), pulled from `theaia/AIGamePyLibrary` main  
**Sim target:** `aicomp-soccer-sim` (this repo)

Upstream changelog (author):

```
--v0.63--
- now allow nested functions
- fixed an issue where "load" would clear
- fixed some minor cosmetic issues
```

---

## 0. Status before coding

| Piece | State |
| --- | --- |
| PyLibrary pull | Done — `main` @ `f900f96` |
| Local PyLibrary patches kept | Uncommitted: `HitInfo.Tag` (`String1`) + TimePlot optional `Float2`/`Float3` min/max in `data.py` / `nodes.py` |
| Sim nested-Function Null guard | **Still active** — must change |
| Quirk #1b (`AIA_UPSTREAM_QUIRKS.md`) | Still says nest → Null — mark superseded when sim lands |
| Load-clear / cosmetics | Editor/UI only — **no sim physics work** unless a TimePlot proves otherwise |

Do **not** start from Titanium engine work in this handoff; that is a separate thread (`titanim-socker-engine/docs/HANDOFF_2026-07-25.md`).

---

## 1. Must change in sim — nested custom functions

### What Unity does now

Custom `Function` calls **inside** another `CreateFunction` body are legal and must return real values (not Null). Pre-v0.63 (Discord AIA / Maia): nest → Null / identical wrong distances. Stock `AIA.txt` still has **0** nested calls, so AIA parity alone will not catch a miss.

### Where the sim still rejects nesting

Hard early-return when `call_stack` is non-empty:

1. `src/graph/eval.rs` — `GraphBrain::eval_function_call` (~604–609)  
2. `src/graph_vm/lower.rs` — `Lowerer::lower_function_call` (~825–829)

Comments still cite the old Discord limit — delete those with the guards.

### Implementation sketch

1. Remove both `if !self.call_stack.is_empty() { return Null }` guards.
2. Keep push/pop `CallFrame` as today (args + per-call cache / port regs). Nesting = deeper stack, not a new IR shape.
3. Watch for:
   - **CreateFunction param resolution** while multiple frames are live — must bind to the **innermost** matching `create_sid` (already `call_stack.last()` in both paths; verify with a 2-deep test).
   - **SetVariable / GetVariable** visibility across nest levels — match Unity (probe if unsure; default = shared graph variables, same as today for non-nested).
   - **DebugDraw** body sinks on each nested call (already looped per call).
   - **Lowering depth** — `MAX_LOWER_DEPTH` (5000) already exists for cycles; deep nesting inflates dependency depth. Leave the cap; do not raise casually.
4. GraphBrain ≡ RuntimeBrain acceptance must still hold for nested graphs (extend existing compile identity tests).

### Tests to add (minimal)

In `src/graph/eval.rs` (and mirror under `graph_vm` if there is a lowerer unit test):

- **Depth 1 (existing):** root `Function("Scale2", …)` still works.
- **Depth 2:** CreateFunction `Outer` body contains `Function("Scale2", Param1)` → root call `Outer(3)` → expect scaled result, **not** Null.
- **Depth 2 with two args / side-effect:** nested call that feeds `DebugDrawLine` or a controller path still runs body draws.
- Optional: mutual / self recursion should hit depth limit or stack behavior without process crash (Unity may soft-fail; do not invent bit-perfect recurse policy without a probe).

Smoke after change:

```bat
cargo test --lib
```

Optional nest probe graph via PyLibrary → Unity Saves, then:

```bat
cargo run --release --bin soccer_headless -- --secs 5 --home graph:%USERPROFILE%\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\<NestProbe>.txt --away idle --quiet
```

---

## 2. No sim work expected

| Upstream item | Why ignore (unless new evidence) |
| --- | --- |
| **"load" would clear** | Editor Load wiping canvas / selection / wrong clear — not match physics or graph eval. Sim `load_team_graph` does not clear match state. |
| **Minor cosmetic issues** | Scoreboard / UI / editor chrome — viewer may ignore. |

If someone reports sim “load clears variables”, that is a **different** bug: ask for repro (which Load, which graph). Do not invent a variable wipe on graph load.

---

## 3. PyLibrary note (already pulled)

Incoming upstream commit only:

- `f900f96` — `Debug(...)` no longer forces the first port’s rect scale to `(0,0)` (“outdated debug node”). Cosmetic / editor layout for generated graphs.

Local uncommitted deltas (keep unless upstream absorbs them):

- `HitInfo` → `Tag` (`String1`, output index 3)
- TimePlot ports `Float2` / `Float3` (optional min/max)

Rebuild Titanium / probes as usual from `titanim-socker-engine` or `worldcupteams` — no forced rebuild for this handoff.

---

## 4. Docs to update when nested support lands

1. `docs/AIA_UPSTREAM_QUIRKS.md` § **1b** — mark **SUPERSEDED (Unity v0.63)**; sim now evaluates nested `Function`; leave historical Maia note for old builds.
2. `docs/API_GAPS.md` — Custom Function row: note nesting supported.
3. This file — tick §1 done + link the PR/commit.

---

## 5. Out of scope / do not mix in

- Bevy feature / profile / `.cargo` changes (incremental compile hard requirement).
- Dropdown `labels.rs` reorder (append-only).
- Titanium behaviour / wall-bounce / 180s gate work — separate handoff.
- Treating v0.63 cosmetics as physics parity work.

---

## 6. Suggested first commit message (when implementing)

```
Allow nested CreateFunction/Function calls (Unity v0.63)

Remove the call_stack Null guards in GraphBrain and O0 lowerer so nested
custom functions return real values. Add depth-2 unit coverage.
```
