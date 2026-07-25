# Player constants — measured, and what's left

Ball physics is done (see commits `8cba432`, `4f46928`, `d7c7f8c`). This is the
player half. Same method: measure in the real game via TimePlot, never infer
from the sim's own values — the sim was wrong on nearly every ball constant it
had not been checked against.

## Confirmed (user measurement, 2026-07-25)

| quantity | value | note |
| --- | --- | --- |
| walk speed | **7.0** | sim previously used `max_speed*0.95` = 7.6 |
| sprint speed | **8.0** | unchanged |
| sprint, stamina empty | **7.65** | sim previously kept full 8.0 |
| turn rate | **2500 deg/s** | effectively instant; any angle immediately |
| stamina drain (full, constant sprint) | **34.5 s** | already matched the sim |

## Still unmeasured

| quantity | sim value | how to get it |
| --- | --- | --- |
| acceleration | 100.0 | ramp from standstill; 100 implies near-instant, easy to falsify |
| stopping distance | 1.25 | release input at speed, measure overshoot past target |
| body radius | 0.762 | walk flush into each wall: `corner - T1.X`, same trick as the ball's 0.25 |
| stamina regen (full) | 20.0 s | **free** — stamina is a node; stand still and time it |
| tackle / steal rules | — | see harness below |

Stamina is published as `Team Player N Stamina`, so drain and regen need no
inference at all — just time the node.

## Tackle harness (user design)

The point is to make the tackle attempt as repeatable as possible, so the
outcome depends on the tackle rules and not on how the approach happened to
go.

Setup:

- Two players only. Carrier starts **higher up** the pitch, tackler **lower
  down**, both near centre.
- The **carrier approaches the tackler** (not the other way round).
- The tackler moves in the **same direction** as the carrier at **half or
  quarter speed**, closing slowly rather than charging.
- Tackler attempts the steal **as early as physically possible**.

Why the same-direction, slower approach: a head-on charge makes closing speed
and contact frame vary wildly between runs, so the tackle fires at a different
relative position every time. Matching direction at a fraction of the speed
keeps closing rate low and roughly constant, so contact happens at a
predictable separation and the tackle rules are what varies, not the geometry.

What to record: both players' positions and stamina, `Hold.Carrying`, ball
position, and interact state, on every tick around the contact.

What it should answer: effective steal range vs the published
`Player Interact Radius` (1.75), whether stamina decides the outcome and at
what margin, and the post-tackle possession/pickup delay.

## Precise-stop idea (user)

Rather than braking at a fixed `stopping_distance`, solve the stop from
measured acceleration: given current speed and decel, compute the distance
needed and begin braking exactly there so the player halts on the target spot.
Needs `acceleration` measured first — it is the input to that solve.

## Method note

Every ball constant that turned out wrong was wrong because it had been
transcribed or guessed, then trusted. Two habits caught them:

1. Measure the quantity **directly** (deceleration per tick) rather than
   fitting it through a reimplementation — the direct route gave friction to
   four decimals and killed a bad 5.40 estimate from curve-fitting.
2. Filter events by **position**, not just by signal shape — velocity sign
   flips alone caught player touches as "wall bounces" and produced physically
   impossible restitutions above 1.
