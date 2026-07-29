//! Why did we lose the ball? Two answers, and only one is worth fixing.
//!
//!   UNAVOIDABLE  no heading existed that kept the ball out of tackle range
//!   MISPREDICTED at least one safe heading existed, and we lost it anyway
//!
//! A scoreline cannot separate these. Losing the ball with no way out is the
//! situation being hard; losing it when an escape existed is the model being
//! wrong, and only the second is evidence that anti-tackle maths needs work.
//!
//! Measured entirely from the simulator's own state — no graph instrumentation,
//! no debug-draw plumbing. At the tick a steal happens we know every position,
//! stamina and radius, so "was there a safe heading" is plain geometry.
//!
//!   cargo run --release --bin tackle_reason -- --home graph:<g> --away graph:<o> --secs 180
use std::env;
use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use bevy::prelude::Vec2;

fn brain(input: &BrainInput, cache: &ProgramCache) -> Box<dyn TeamBrain> {
    match input {
        BrainInput::Graph(p) => Box::new(RuntimeBrain::from_cached(
            cache.get_or_compile(p).expect("compile"),
        )),
        _ => Box::new(aicomp_soccer_sim::brain::ChaseBallBrain::default()),
    }
}

/// Did ANY heading keep the held ball out of every capable tackler's reach?
///
/// Order matters and is the whole point: a tick resolves MOVEMENT first and
/// INTERACTIONS second, so a tackle is judged against where everyone actually
/// ended up, not where they started. Opponent positions here are therefore the
/// RESOLVED ones from after the step — their real movement, not a guess that
/// they walked straight at the carrier. Only the carrier's own end position is
/// predicted, because that is the thing being chosen: each candidate heading
/// implies a different one.
///
/// Samples the full circle; this is a measuring tool, not the engine, so
/// sampling is the honest simple thing rather than a closed form that could
/// drift from what the engine actually does.
/// Is this ONE heading safe, by the same post-movement geometry?
fn heading_is_safe(
    params: &SimParams,
    carrier_start: Vec2,
    heading: Vec2,
    my_stam: f32,
    resolved_opponents: &[(Vec2, Vec2, f32)],
) -> bool {
    let me_end = carrier_start + heading * params.player_walk_speed * FIXED_DT;
    let ball_end = me_end + heading * params.hold_offset;
    !resolved_opponents.iter().any(|(opp_end, opp_facing, stam)| {
        // A weaker opponent loses the duel and is a ghost — INDEPENDENTLY.
        //
        // The residual mispredictions all live here. A failed tackle still
        // drains the carrier by min(both), and phase 2 applies that drain
        // immediately, so opponent A can fail, drop the carrier's stamina, and
        // leave opponent B — previously too weak — winning the duel on the same
        // tick. Judging each opponent against the carrier's PRE-drain stamina
        // (as here, and as Titanium does) cannot see that chain.
        //
        // NOT graded: whether the real game applies drain mid-tick or at tick
        // end is unmeasured. If it is tick-end, this chain is a sim artifact
        // and the mispredictions are not Titanium's fault at all.
        if *stam < my_stam {
            return false;
        }
        // Interact distance from the tackler's POSITION to the CARRIER's
        // center, matching apply_interact exactly. The ball sits at a hold
        // offset from the carrier; the real game checks center-to-center.
        let _ = opp_facing;
        (*opp_end - me_end).length() <= params.interact_radius
    })
}

fn any_safe_heading(
    params: &SimParams,
    carrier_start: Vec2,
    my_stam: f32,
    resolved_opponents: &[(Vec2, Vec2, f32)],
) -> bool {
    for i in 0..72 {
        let a = (i as f32) * std::f32::consts::TAU / 72.0;
        let heading = Vec2::new(a.cos(), a.sin());
        let me_end = carrier_start + heading * params.player_walk_speed * FIXED_DT;
        let ball_end = me_end + heading * params.hold_offset;
        let mut tackled = false;
        for (opp_end, opp_facing, stam) in resolved_opponents {
            if *stam < my_stam {
                continue; // cannot win the duel - a ghost
            }
            let _ = opp_facing;
            if (*opp_end - ball_end).length() <= params.interact_radius {
                tackled = true;
                break;
            }
        }
        if !tackled {
            return true;
        }
    }
    false
}

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let get = |k: &str| -> Option<String> {
        argv.iter().position(|a| a == k).and_then(|i| argv.get(i + 1).cloned())
    };
    let home = BrainInput::parse(&get("--home").expect("--home")).expect("home");
    let away = BrainInput::parse(&get("--away").expect("--away")).expect("away");
    let secs: f32 = get("--secs").unwrap_or_else(|| "180".into()).parse().unwrap();

    let cache = ProgramCache::default();
    let (mut h, mut a) = (brain(&home, &cache), brain(&away, &cache));
    let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);

    let (mut unavoidable, mut mispredicted, mut avoidable_but_not_taken) = (0u32, 0u32, 0u32);
    let (mut model_err_sum, mut samples) = (0.0f32, 0u32);
    let mut prev: Option<(TeamId, u8)> = None;
    for _ in 0..((secs / FIXED_DT) as u32) {
        // Snapshot BEFORE stepping: this is the state the decision was made in.
        let before = world.possession.carrier;
        let snapshot = before.and_then(|(t, id)| {
            world.players.iter().find(|p| p.team == t && p.id.0 == id)
                .map(|p| (t, p.pos, p.stamina))
        });
        // Step by hand rather than step_brains, so the carrier's CHOSEN heading
        // is visible. "Did a safe heading exist somewhere in 360 degrees" is
        // almost always yes - you can nearly always turn away from a tackler -
        // so it separates nothing. What matters is whether the direction the
        // engine actually picked was safe.
        let ball_before = world.ball.pos;
        let pre_positions: Vec<(TeamId, u8, Vec2)> =
            world.players.iter().map(|q| (q.team, q.id.0, q.pos)).collect();
        // Stamina BEFORE the step. A tackle drains min(tackler, carrier) from
        // both, so reading it afterwards makes the tackler look too exhausted to
        // have tackled - the ghost filter then excuses the very player who took
        // the ball. The duel is decided on pre-drain values, so those are what
        // the model has to be judged against.
        let pre_stamina: Vec<(TeamId, u8, f32)> =
            world.players.iter().map(|q| (q.team, q.id.0, q.stamina)).collect();
        let (home_api, away_api) = world.build_apis();
        let home_out = h.think(&home_api);
        let away_out = a.think(&away_api);
        let chosen = before.and_then(|(t, id)| {
            if t != TeamId::Home { return None; }
            home_out.commands.get(id as usize - 1).map(|c| c.move_to)
        });
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        let after = world.possession.carrier;

        // A steal: home held it, now the away side does.
        if let (Some((TeamId::Home, _)), Some((TeamId::Away, _))) = (before, after) {
            if let Some((team, pos, stam)) = snapshot {
                // Opponents at their RESOLVED positions - movement has already
                // been applied this tick, which is the state the interaction
                // was actually judged against.
                // Resolved POSITIONS (post-movement) with PRE-drain stamina.
                let resolved: Vec<(Vec2, Vec2, f32)> = world
                    .players
                    .iter()
                    .filter(|q| q.team != team)
                    .map(|q| {
                        let stam = pre_stamina
                            .iter()
                            .find(|(tm, id, _)| *tm == q.team && *id == q.id.0)
                            .map(|(_, _, s)| *s)
                            .unwrap_or(q.stamina);
                        (q.pos, q.facing, stam)
                    })
                    .collect();
                // Was the direction it actually walked a safe one?
                let picked = chosen
                    .map(|m| (m - pos).normalize_or_zero())
                    .unwrap_or(Vec2::ZERO);
                let picked_safe = picked.length_squared() > 1e-6
                    && heading_is_safe(&world.params, pos, picked, stam, &resolved);
                if picked_safe {
                    mispredicted += 1;
                    // Diagnose WHY: Titanium models every defender as walking
                    // straight at the ball at walk speed. Compare that model
                    // against where the tackler actually ended up.
                    if let Some((_, tid)) = after {
                        if let Some(t) = world.players.iter()
                            .find(|q| q.team == TeamId::Away && q.id.0 == tid)
                        {
                            let start = pre_positions.iter()
                                .find(|(tm, id, _)| *tm == TeamId::Away && *id == tid)
                                .map(|(_, _, p)| *p)
                                .unwrap_or(t.pos);
                            let modelled = start
                                + (ball_before - start).normalize_or_zero()
                                    * world.params.player_walk_speed * FIXED_DT;
                            let err = (modelled - t.pos).length();
                            let moved = (t.pos - start).length();
                            let me_end = pos + picked * world.params.player_walk_speed * FIXED_DT;
                            let ball_end = me_end + picked * world.params.hold_offset;
                            model_err_sum += err;
                            samples += 1;
                            // Titanium assumes the ball lands at
                            //     me_end + chosen_heading * hold_offset
                            // Compare that against where the ball ACTUALLY was
                            // when the tackle was judged: the losing carrier's
                            // own wall-projected hold point.
                            //
                            // NOT `world.ball.pos` — by now the tackler owns the
                            // ball, so that reads the tackler's hold point and
                            // reports a ~2 m "model error" that is pure artifact.
                            let real_ball = world
                                .players
                                .iter()
                                .find(|q| Some((q.team, q.id.0)) == before)
                                .map(|q| q.hold_pos_playable(&world.params))
                                .unwrap_or(world.ball.pos);
                            let ball_model_err = (real_ball - ball_end).length();
                            println!("  miss: A{tid} movement err {err:.3}m | ball assumed {:.2}m from tackler, REAL {:.2}m (r {:.2}) | ball position error {ball_model_err:.3}m",
                                (t.pos - ball_end).length(),
                                (t.pos - real_ball).length(),
                                world.params.interact_radius);
                            let _ = moved;
                        }
                    }
                } else if any_safe_heading(&world.params, pos, stam, &resolved) {
                    avoidable_but_not_taken += 1;
                } else {
                    unavoidable += 1;
                }
            }
        }
        prev = after;
    }
    let _ = prev;

    if samples > 0 {
        println!("
  mean tackler position-model error: {:.3} m over {samples} misses",
                 model_err_sum / samples as f32);
    }
    let total = unavoidable + mispredicted + avoidable_but_not_taken;
    println!("balls lost to a tackle: {total}");
    if total > 0 {
        let pct = |n: u32| 100.0 * n as f32 / total as f32;
        println!("  MISPREDICTED  walked a heading it judged SAFE, lost it anyway {mispredicted:5}  ({:.1}%)", pct(mispredicted));
        println!("  WRONG CHOICE  a safe heading existed, it walked another one   {avoidable_but_not_taken:5}  ({:.1}%)", pct(avoidable_but_not_taken));
        println!("  UNAVOIDABLE   no heading anywhere was safe                    {unavoidable:5}  ({:.1}%)", pct(unavoidable));
    }
}
