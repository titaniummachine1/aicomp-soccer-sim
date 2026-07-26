//! SNAKE / STAR: a team that keeps the ball by making it unreachable.
//!
//! The chain idea, with Titanium's anti-tackle model deciding where the ball
//! goes. Three pieces:
//!
//! 1. FREEDOM SCORE (Titanium's blocked-arc math, reused). A held ball traces a
//!    circle of radius `hold_offset` around its carrier, so an opponent at
//!    distance `d` denies an ARC of that circle, half-width
//!        acos((hold^2 + d^2 - r_int^2) / (2 * hold * d))
//!    An opponent further than `hold + r_int` denies nothing; one closer than
//!    `r_int - hold` denies everything. Sum the arcs and you have exactly how
//!    much of a player's ball-circle is unusable. A player with zero blocked
//!    arc CANNOT be tackled, whatever it does. That number, not distance, is
//!    what "safe" means.
//!
//! 2. STAR FORMATION. Non-carriers stand one chain-spacing from the carrier, so
//!    every one of them can take the ball off it instantly. They pick their
//!    direction to trade forward progress against their own blocked arc — which
//!    is what makes the shape zigzag around pressure instead of walking into
//!    it, and collapse to a defensive star when the whole team is marked.
//!
//! 3. RELAY. The ball moves to the freest player in reach, every tick. An
//!    opponent closing on the ball is what pushes the ball away from it: their
//!    approach widens the arc they block, which drops that player's score below
//!    a spoke they are not covering.
//!
//!     cargo run --release --bin snake -- --away graph:<path> --secs 180

use std::env;

use bevy::prelude::Vec2;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn brain(input: &BrainInput, cache: &ProgramCache) -> Box<dyn TeamBrain> {
    match input {
        BrainInput::Graph(p) => Box::new(RuntimeBrain::from_cached(
            cache.get_or_compile(p).expect("compile"),
        )),
        _ => Box::new(aicomp_soccer_sim::brain::ChaseBallBrain::default()),
    }
}

/// An opponent that matters: predicted position after this tick, and stamina.
#[derive(Clone, Copy)]
struct Threat {
    pos: Vec2,
    stamina: f32,
}

/// Fraction of a player's ball-circle that opponents deny, in [0, 1].
///
/// 0.0 means every direction is safe — this player cannot be tackled no matter
/// which way it turns. 1.0 means it is dead whatever it does.
///
/// Arcs are summed, not merged, so overlapping opponents over-count. That is
/// deliberate: over-counting is conservative, it only ever makes a covered
/// player look worse, and merging is not needed to pick the best of four.
fn blocked_fraction(me: Vec2, my_stam: f32, threats: &[Threat], hold: f32, r_int: f32) -> f32 {
    let mut blocked = 0.0f32;
    for t in threats {
        // A weaker opponent loses the duel, so it is not a threat at all.
        if t.stamina < my_stam {
            continue;
        }
        let d = (t.pos - me).length();
        if d > hold + r_int {
            continue; // cannot reach any point on the circle
        }
        if d + hold <= r_int {
            return 1.0; // covers the whole circle: nowhere to turn
        }
        let denom = (2.0 * hold * d).max(1e-4);
        let cos_half = ((hold * hold + d * d - r_int * r_int) / denom).clamp(-1.0, 1.0);
        blocked += 2.0 * cos_half.acos();
    }
    (blocked / std::f32::consts::TAU).min(1.0)
}

/// Where a player ENDS this tick, and which way it will be facing.
///
/// Interaction is resolved after movement, so every reach test and every hold
/// point has to be evaluated here, not at the position the player is standing
/// on while the decision is being made. Mirrors `step_mover` exactly: rotate to
/// the move direction instantly, then brake-to-rest or accelerate-and-step.
fn predict(pos: Vec2, vel: Vec2, move_to: Vec2, facing: Vec2, p: &SimParams, dt: f32) -> (Vec2, Vec2) {
    let to = move_to - pos;
    let dist = to.length();
    let speed = vel.length();
    let decel = p.player_decel.max(1e-6);
    let brake = if speed > 1e-6 { speed * speed / (2.0 * decel) } else { 0.0 };
    let new_facing = if dist > 1e-6 { to.normalize() } else { facing };

    if dist <= 1e-3 || dist <= brake {
        if speed > 1e-6 {
            let dir = vel.normalize();
            let dv = decel * dt;
            let step = if speed <= dv {
                speed * speed / (2.0 * decel)
            } else {
                speed * dt - 0.5 * decel * dt * dt
            };
            return (pos + dir * step, new_facing);
        }
        return (pos, new_facing);
    }
    let max_speed = p.player_walk_speed;
    let desired = new_facing * max_speed;
    let delta = desired - vel;
    let max_delta = p.player_accel * dt;
    let mut v = if delta.length() <= max_delta { desired } else { vel + delta.normalize() * max_delta };
    if v.length() > max_speed {
        v *= max_speed / v.length();
    }
    (pos + v * dt, new_facing)
}

/// Where the ball will be after `t` seconds.
///
/// Two cases, one formula. A LOOSE ball is a Coulomb slide: constant
/// deceleration `decel` along its heading, stopping dead at zero. A HELD ball
/// simply rides its carrier, so it keeps that carrier's velocity and `decel` is
/// zero. Both are "the ball has a velocity and I should aim where it ends up".
fn ball_at(pos: Vec2, vel: Vec2, t: f32, decel: f32) -> Vec2 {
    let speed = vel.length();
    if speed < 1e-4 {
        return pos;
    }
    if decel <= 1e-4 {
        return pos + vel * t; // held: travels with the carrier
    }
    let stop_t = speed / decel;
    let tt = t.min(stop_t);
    pos + vel.normalize() * (speed * tt - 0.5 * decel * tt * tt)
}

/// Earliest point on the ball's path this player can actually get to.
///
/// Running at where the ball IS is running at where it has already left —
/// a tail chase that never closes on anything moving away. This walks forward
/// in time and takes the first moment the player can be within interact range,
/// which is a cut-off rather than a pursuit.
fn intercept_point(me: Vec2, ball: Vec2, bvel: Vec2, decel: f32, p: &SimParams, dt: f32) -> Vec2 {
    for k in 0..160 {
        let t = k as f32 * dt;
        let bp = ball_at(ball, bvel, t, decel);
        // Where we can have got to by then, plus the reach we arrive with.
        if (bp - me).length() <= p.player_walk_speed * t + p.interact_radius {
            return Vec2::new(bp.x.clamp(-p.x_max, p.x_max), bp.y.clamp(-p.z_max, p.z_max));
        }
    }
    let bp = ball_at(ball, bvel, 160.0 * dt, decel);
    Vec2::new(bp.x.clamp(-p.x_max, p.x_max), bp.y.clamp(-p.z_max, p.z_max))
}

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let get = |k: &str| -> Option<String> {
        argv.iter()
            .position(|a| a == k)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let away_in =
        BrainInput::parse(&get("--away").unwrap_or_else(|| "chase".into())).expect("away brain");
    let secs: f32 = get("--secs").unwrap_or_else(|| "180".into()).parse().unwrap();
    // Walk at the CLOSEST point on the goal line rather than the centre spot.
    // OFF by default because it MEASURED WORSE: 45-19 against 44-16 on its own,
    // and it costs 3 goals on top of interception (72-11 against 75-11).
    // The centre spot is not a detour, it is the one place the chain can be
    // fed from every angle; drifting to a corner of the mouth trades a shorter
    // walk for spokes that no longer have room behind them.
    let nearest_goal = argv.iter().any(|a| a == "--nearest");
    // Chase where a loose ball WILL be, not where it is.
    let intercept = !argv.iter().any(|a| a == "--no-intercept");

    let cache = ProgramCache::default();
    let mut away = brain(&away_in, &cache);
    let params = SimParams::default();
    let hold = params.hold_offset;
    let r_int = params.interact_radius;
    // 1 cm inside maximum reach: at exactly hold + r_int the f32 gap lands on
    // 1.7500001 and the engine's `dist <= interact_radius` refuses.
    let spacing = hold + r_int - 0.01;
    let goal_c = Vec2::new(params.goal_line_x, 0.0);
    // Aim 2 m PAST the line: the player is clamped to the pitch at x_max, but
    // its held ball sits hold_offset further on and is not clamped inside the
    // mouth, so walking into the line carries the ball over it.
    let mouth = params.goal_half_width - 0.7;
    let goal_for = |from: Vec2| -> Vec2 {
        if nearest_goal {
            Vec2::new(params.goal_line_x + 2.0, from.y.clamp(-mouth, mouth))
        } else {
            goal_c
        }
    };

    let mut w = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);

    let (mut held, mut ticks, mut lost) = (0u32, 0u32, 0u32);
    let (mut max_x, mut gf, mut ga) = (-100.0f32, 0u32, 0u32);
    let (mut blocked_sum, mut blocked_n) = (0.0f64, 0u32);
    let mut pressed = [false; 4];

    // Candidate spoke directions for the star.
    let dirs: Vec<Vec2> = (0..16)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            Vec2::new(a.cos(), a.sin())
        })
        .collect();

    for _ in 0..((secs / FIXED_DT) as u32) {
        let (_, away_api) = w.build_apis();
        let away_out = away.think(&away_api);

        let ball = w.ball.pos;
        let ball_free = !w.ball.held;
        let carrier = w.possession.carrier;

        // Opponents, stepped one tick toward the ball at walk speed — the same
        // prediction Titanium makes, and the right one because interaction is
        // resolved after movement.
        let threats: Vec<Threat> = w
            .players
            .iter()
            .filter(|p| p.team == TeamId::Away)
            .map(|p| {
                let to = ball - p.pos;
                let step = if to.length() > 1e-6 {
                    to.normalize() * params.player_walk_speed * FIXED_DT
                } else {
                    Vec2::ZERO
                };
                Threat { pos: p.pos + step, stamina: p.stamina }
            })
            .collect();

        let mine: Vec<usize> = (0..w.players.len())
            .filter(|&i| w.players[i].team == TeamId::Home)
            .collect();

        // Anchor: whoever holds the ball, else the ball itself.
        let anchor = match carrier {
            Some((TeamId::Home, id)) => w
                .players
                .iter()
                .find(|p| p.team == TeamId::Home && p.id.0 == id)
                .map(|p| p.pos)
                .unwrap_or(ball),
            _ => ball,
        };

        // Freedom of the current ball holder, for the relay comparison.
        let ball_blocked = match carrier {
            Some((TeamId::Home, id)) => {
                let p = w
                    .players
                    .iter()
                    .find(|q| q.team == TeamId::Home && q.id.0 == id)
                    .unwrap();
                blocked_fraction(p.pos, p.stamina, &threats, hold, r_int)
            }
            _ => 1.0, /* placeholder, recomputed below at predicted state */ // loose or enemy-held: anything of ours is an improvement
        };
        if carrier.is_some_and(|(t, _)| t == TeamId::Home) {
            blocked_sum += ball_blocked as f64;
            blocked_n += 1;
        }

        // ---- STAR: pick a spoke for each non-carrier --------------------
        // Greedy, one direction each, scoring forward progress against how
        // exposed that spot is. Simple and good enough to choose among 16.
        let mut taken: Vec<usize> = Vec::new();
        let mut slot_for = [Vec2::ZERO; 4];
        // If we do not hold the ball, nothing else matters: go and get it.
        // Loose -> intercept the sliding ball. Enemy-held -> intercept the ball
        // travelling with its carrier, which is what a tackle has to reach.
        let we_hold = matches!(carrier, Some((TeamId::Home, _)));
        let chase = if !we_hold && intercept {
            let (bvel, decel) = match carrier {
                Some((TeamId::Away, id)) => {
                    // A held ball rides the carrier: same velocity, no slide.
                    let c = w
                        .players
                        .iter()
                        .find(|q| q.team == TeamId::Away && q.id.0 == id)
                        .unwrap();
                    (c.vel, 0.0)
                }
                _ => (w.ball.vel, params.slide_accel),
            };
            Some((bvel, decel))
        } else {
            None
        };
        let to_goal = (goal_for(anchor) - anchor).normalize_or_zero();
        for &i in &mine {
            let p = &w.players[i];
            if matches!(carrier, Some((TeamId::Home, id)) if id == p.id.0) {
                // The carrier drives the whole formation at the goal.
                slot_for[p.id.0 as usize - 1] = goal_for(p.pos);
                continue;
            }
            if let Some((bvel, decel)) = chase {
                slot_for[p.id.0 as usize - 1] =
                    intercept_point(p.pos, ball, bvel, decel, &params, FIXED_DT);
                continue;
            }
            let mut best = (f32::NEG_INFINITY, Vec2::ZERO, usize::MAX);
            for (k, d) in dirs.iter().enumerate() {
                if taken.contains(&k) {
                    continue;
                }
                let spot = anchor + *d * spacing;
                if spot.x.abs() > params.x_max - 1.0 || spot.y.abs() > params.z_max - 1.0 {
                    continue; // keep spokes on the pitch
                }
                let exposure = blocked_fraction(spot, p.stamina, &threats, hold, r_int);
                // Forward progress, minus exposure, minus how far this player
                // has to run to get there (so spokes do not swap every tick).
                let score = 1.5 * d.dot(to_goal) - 2.0 * exposure
                    - 0.05 * (spot - p.pos).length();
                if score > best.0 {
                    best = (score, spot, k);
                }
            }
            if best.2 != usize::MAX {
                taken.push(best.2);
                slot_for[p.id.0 as usize - 1] = best.1;
            } else {
                slot_for[p.id.0 as usize - 1] = anchor;
            }
        }

        // ---- PREDICT: where everyone ENDS this tick ----------------------
        // The chain must be valid AFTER movement, because that is when the
        // engine resolves interaction. Testing reach against current positions
        // plans a pass between two players who will not be there.
        let mut end_pos = [Vec2::ZERO; 4];
        let mut end_face = [Vec2::X; 4];
        for &i in &mine {
            let p = &w.players[i];
            let s = p.id.0 as usize - 1;
            let (np, nf) = predict(p.pos, p.vel, slot_for[s], p.facing, &params, FIXED_DT);
            end_pos[s] = np;
            end_face[s] = nf;
        }
        // The ball ends at the carrier's PREDICTED hold point — it rotates with
        // the carrier's walk direction and moves with it, before any tackle.
        let ball_end = match carrier {
            Some((TeamId::Home, id)) => {
                let s = id as usize - 1;
                end_pos[s] + end_face[s] * hold
            }
            _ => ball,
        };

        // ---- RELAY: ball goes to the freest player in reach --------------
        // Reach and freedom are both measured at the predicted end state.
        let mut best_taker: Option<(f32, u8)> = None;
        for &i in &mine {
            let p = &w.players[i];
            let s = p.id.0 as usize - 1;
            if matches!(carrier, Some((TeamId::Home, id)) if id == p.id.0) {
                continue;
            }
            if (end_pos[s] - ball_end).length() > r_int {
                continue; // chain would be broken by the time it mattered
            }
            let f = blocked_fraction(end_pos[s], p.stamina, &threats, hold, r_int);
            // Take it if clearly freer, or as free and further forward.
            let better = f < ball_blocked - 0.02
                || (f <= ball_blocked + 0.02 && end_pos[s].x + hold > ball_end.x + 0.10);
            if better && best_taker.is_none_or(|(bf, _)| f < bf) {
                best_taker = Some((f, p.id.0));
            }
        }

        let mut out = BrainOutput::default();
        for &i in &mine {
            let p = &w.players[i];
            let slot = p.id.0 as usize - 1;
            let carrying = matches!(carrier, Some((TeamId::Home, id)) if id == p.id.0);
            let want = if carrying {
                // RETAIN: a carrier kicks on the FALLING edge, so never release.
                true
            } else if ball_free {
                (end_pos[slot] - ball).length() <= r_int
            } else {
                best_taker.is_some_and(|(_, id)| id == p.id.0)
            };
            // Pulse everything except the retain-hold: Interact is an impulse,
            // and a pinned press is one attempt for the whole match.
            let interact = if carrying { true } else { want && !pressed[slot] };
            pressed[slot] = interact;
            out.commands[slot] = BrainCommand {
                move_to: slot_for[slot],
                sprint: false,
                interact,
            };
        }

        let before = w.possession.carrier;
        let (sh, sa) = (w.match_state.score_home, w.match_state.score_away);
        w.step_with_commands(&out, &away_out, FIXED_DT);
        gf += w.match_state.score_home - sh;
        ga += w.match_state.score_away - sa;

        ticks += 1;
        match w.possession.carrier {
            Some((TeamId::Home, _)) => {
                held += 1;
                max_x = max_x.max(w.ball.pos.x);
            }
            Some((TeamId::Away, _)) => {
                if matches!(before, Some((TeamId::Home, _))) {
                    lost += 1;
                }
            }
            None => {}
        }
    }

    let name = match &away_in {
        BrainInput::Graph(p) => p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
        other => format!("{other:?}"),
    };
    println!("SNAKE vs {name}");
    println!(
        "  possession     {:.1}%   balls lost {lost}   goals {gf} for / {ga} against",
        100.0 * held as f32 / ticks as f32
    );
    println!(
        "  deepest ball x {max_x:.1} (goal line {:.0})   mean blocked arc while carrying {:.3}",
        params.goal_line_x,
        if blocked_n == 0 { 0.0 } else { blocked_sum / blocked_n as f64 }
    );
}
