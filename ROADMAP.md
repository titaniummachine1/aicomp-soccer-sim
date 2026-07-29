# Roadmap

**Generated — do not edit by hand.** `python scripts/gen_roadmap.py`

This file is derived from the code and the issue tracker, because a
hand-written roadmap is stale the day after it is written. Fidelity
grades come from `cargo run --bin fidelity_report`; the work list comes
from GitHub issues. To change the roadmap, change what it describes.

## What the simulator is verified to get right

Every API getter carries a confidence grade. The default is UNCERTAIN:
a getter earns CONFIRMED only by being compared against a real-game
recording, and the note must cite which one — a test enforces that.
This matters because "returns a plausible number" is not a testable
property, and the bugs that have actually cost us were all of that shape.

**Confirmed** 21 · **Approximated** 1 · **Uncertain** 18 · **Wrong/Absent** 0

### Confirmed against a real-game reading

- `Ball Carrier Shot Charge` — 0..1 in game, matching the sim's range (MatchProbe 2026-07-26)
- `Current Simulation Time` — advances 1.0 per real second in game â€” the on-screen 20x is display only
- `Delta Time` — 0.019 in game, constant across the recording (MatchProbe 2026-07-26)
- `Field Depth` — 80.0 in game and sim (MatchProbe 2026-07-26)
- `Field Width` — 50.0 in game and sim (MatchProbe 2026-07-26)
- `Fixed Delta Time` — 0.019 in game; equals sim FIXED_DT and the TimePlot sample spacing
- `Goal Height` — 4.000 in game, constant (MatchProbe 2026-07-26). The sim had 2.44, a placeholder that outlived the TimePlot it was waiting for.
- `Goal Width` — 11.4 in game and sim; half-width 5.7 (MatchProbe 2026-07-26)
- `Kickoff Circle Radius` — 7.250 in game and sim (MatchProbe 2026-07-26); also explains Titanium's measured 7.75 standoff: circle plus 0.5 clearance.
- `Max Simulation Time` — 180.0 default in game; it is a SETTING and can be changed per match
- `Opponent Attacking %` — same 0..1 fraction and 0 baseline as Team Attacking %; read 0.000 in game throughout (MatchProbe 2026-07-26)
- `Opponent Player 1 Stamina` — 1.000 at full in game, matching the sim's 0..1 (MatchProbe 2026-07-26)
- `Opponent Possession %` — same 0..1 fraction as Team Possession %; read 0.000 in game throughout while the other side held the ball (MatchProbe 2026-07-26)
- `Pi` — 3.14159 in game â€” the canary that proves index alignment
- `Player Interact Radius` — 1.750 in game and sim (MatchProbe 2026-07-26)
- `Simulation Time Remaining` — equals Max - Current throughout (MatchProbe 2026-07-26)
- `Stamina of last defending opponent` — 1.000 at full in game (MatchProbe 2026-07-26)
- `Team Attacking %` — 0..1 fraction with a 0 baseline; read 0.000 for both sides across the reference recording (MatchProbe 2026-07-26). Sim corrected from 50.0-at-kickoff.
- `Team Player 1 Stamina` — 1.000 at full in game, matching the sim's 0..1 (MatchProbe 2026-07-26)
- `Team Possession %` — 0..1 FRACTION, not a percentage despite the label, and 0 before anyone touches it. A probe holding the ball all match read 1.000 while its opponent read 0.000 (MatchProbe 2026-07-26). Sim corrected from 0..100-starting-at-50.
- `Teammate 1 Shot Charge` — 0..1 in game, matching the sim's range (MatchProbe 2026-07-26)

### Approximated — documented model, not the engine's own

- `Ball Carrier Stamina` — 0..1 in both; game reached 1.000 while carrying. The DRAIN model is the sim's own and has never been compared tick-by-tick.

### Uncertain — implemented, never measured

These are the backlog. Each needs one real-game reading to
settle, and several feed the tackle economy directly.

- `Ball Speed`
- `Opponent Nearest Teammate Player 1 Stamina`
- `Opponent Nearest Teammate Player 2 Stamina`
- `Opponent Nearest Teammate Player 3 Stamina`
- `Opponent Nearest Teammate Player 4 Stamina`
- `Opponent Player 2 Stamina`
- `Opponent Player 3 Stamina`
- `Opponent Player 4 Stamina`
- `Opponent Score`
- `Opponent Shots`
- `Team Player 2 Stamina`
- `Team Player 3 Stamina`
- `Team Player 4 Stamina`
- `Team Score`
- `Team Shots`
- `Teammate 2 Shot Charge`
- `Teammate 3 Shot Charge`
- `Teammate 4 Shot Charge`

## Open issues

None open.

## Closed (6)

- ~~[#7](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/7) ur ball stealing system doesnt work like in game~~
- ~~[#6](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/6) ur ball stealing system doesnt work like in game~~
- ~~[#5](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/5) players when its not their kickoff should be able to go into that middle circle even if its not their kickoff and even if the team whose kickoff it is hasnt touched the ball yet...~~
- ~~[#4](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/4) Incorrect node is able to crush the sim~~
- ~~[#3](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/3) random side first kickoff~~
- ~~[#1](https://github.com/titaniummachine1/aicomp-soccer-sim/issues/1) my controller doesnt work~~
