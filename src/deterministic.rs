//! Deterministic shot / pass / walk-in detection — the guaranteed-score layer.
//!
//! Pure math, no heuristics, no "funny logic gates". If the math says the ball
//! cannot be stopped from going in, the code executes immediately.
//!
//! What this module does:
//!   - Detects guaranteed goals (shot left / right / center of goal mouth)
//!   - Detects guaranteed passes (ball reaches teammate, no opponent can intercept)
//!   - Detects walk-in goals (carrier can walk ball across goal line unopposed)
//!   - Auto-charges the ball while held (interact = true)
//!   - Releases the shot at the perfect angle when fully charged
//!
//! What this module does NOT do:
//!   - Player movement / positioning (AI handles that)
//!   - Tackling decisions (AI handles that)
//!   - Ball pickup decisions (AI handles that, this module provides a bool)

use bevy::prelude::*;

use crate::api::TeamApi;
use crate::brain::{BrainOutput, TeamId};
use crate::params::SimParams;
use crate::world::FIXED_DT;

/// Tolerance for float comparisons.
const EPS: f32 = 1e-4;

/// Maximum wall-bounce iterations for analytic shot simulation.
const MAX_BOUNCES: usize = 3;

/// Maximum ticks for loose-ball tick-by-tick simulation (with collisions).
#[allow(dead_code)]
const MAX_LOOSE_TICKS: usize = 3;

/// Full-charge shot ball speed (m/s, planar). Computed from SimParams at runtime.
fn full_charge_horiz_speed(params: &SimParams) -> f32 {
    let c = 1.0;
    let horiz = (params.kick_speed_base + params.kick_speed_per_charge * c)
        .min(params.kick_horiz_cap);
    let lift = (params.kick_lift_base + params.kick_lift_per_charge * c).max(0.0);
    let spd = (horiz * horiz + lift * lift).sqrt();
    if spd > params.kick_max_speed && spd > 1e-6 {
        horiz * (params.kick_max_speed / spd)
    } else {
        horiz
    }
}

/// Full-charge shot lift (m/s, vertical).
fn full_charge_lift(params: &SimParams) -> f32 {
    kick_lift_at(1.0, params)
}

/// Horizontal kick speed at a given charge level (0..1).
fn kick_horiz_at(charge: f32, params: &SimParams) -> f32 {
    let horiz = (params.kick_speed_base + params.kick_speed_per_charge * charge)
        .min(params.kick_horiz_cap);
    let lift = (params.kick_lift_base + params.kick_lift_per_charge * charge).max(0.0);
    let spd = (horiz * horiz + lift * lift).sqrt();
    if spd > params.kick_max_speed && spd > 1e-6 {
        horiz * (params.kick_max_speed / spd)
    } else {
        horiz
    }
}

/// Vertical kick lift at a given charge level (0..1).
fn kick_lift_at(charge: f32, params: &SimParams) -> f32 {
    let horiz = (params.kick_speed_base + params.kick_speed_per_charge * charge)
        .min(params.kick_horiz_cap);
    let lift = (params.kick_lift_base + params.kick_lift_per_charge * charge).max(0.0);
    let spd = (horiz * horiz + lift * lift).sqrt();
    if spd > params.kick_max_speed && spd > 1e-6 {
        lift * (params.kick_max_speed / spd)
    } else {
        lift
    }
}

// ─── Ball simulation ─────────────────────────────────────────────────────────
//
// Two modes:
//   1. ANALYTIC shot simulation — pure math trajectory between wall bounces.
//      Simulate ball as if no walls → find where it hits a wall → resolve
//      bounce → repeat. Up to MAX_BOUNCES iterations. Used for goal detection.
//
//   2. TICK-BY-TICK simulation — exact copy of step_free_ball, used only for
//      loose-ball interception (up to MAX_LOOSE_TICKS ticks) because bouncing
//      off walls matters when the ball is already free.
//
// Interception checks use PURE MATH (straight-line, no collision simulation).
// Opponents sorted by distance, early exit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallEndReason {
    None,
    GoalHome,
    GoalAway,
}

fn sim_goal_at(pos: Vec2, params: &SimParams) -> BallEndReason {
    if pos.x >= params.goal_line_x && pos.y.abs() <= params.goal_half_width {
        return BallEndReason::GoalAway;
    }
    if pos.x <= -params.goal_line_x && pos.y.abs() <= params.goal_half_width {
        return BallEndReason::GoalHome;
    }
    BallEndReason::None
}

/// Which goal reason the attacking team wants. Home attacks +X → GoalAway.
fn attacking_goal_reason(is_home: bool) -> BallEndReason {
    if is_home {
        BallEndReason::GoalAway
    } else {
        BallEndReason::GoalHome
    }
}

// ─── Analytic trajectory (no walls) ───────────────────────────────────────────

/// Compute the analytic trajectory of the ball from `pos` with `vel` (planar)
/// and `lift` (vertical), ignoring walls. Returns where the ball ends up:
///   - Goal scored (ball crosses goal line inside mouth)
///   - Ball stops (Coulomb deceleration brings it to rest)
///   - Ball reaches max range
///
/// Also returns the list of (position, time) samples along the path for
/// interception checking.
/// One segment of the ball's trajectory (between wall bounces).
/// The ball always travels in a straight line along `dir = vel.normalize()`.
/// While airborne (t < t_air): constant planar velocity.
/// While grounded (t >= t_air): Coulomb deceleration along the same line.
#[derive(Debug, Clone, Copy)]
pub struct TrajectorySegment {
    /// Start position of this segment.
    origin: Vec2,
    /// Planar velocity at start of segment.
    vel: Vec2,
    /// Time until ball lands (0 = already grounded).
    t_air: f32,
    /// Time offset from trajectory start to this segment's start.
    t_start: f32,
    /// Duration of this segment (until wall hit, goal, or stop).
    duration: f32,
}

impl TrajectorySegment {
    /// Ball position at time `t` (relative to segment start, 0..=duration).
    /// Ball always moves along a straight line; only speed changes at t_air.
    fn pos_at(&self, t: f32, params: &SimParams) -> Vec2 {
        let v0 = self.vel.length();
        if v0 < EPS {
            return self.origin;
        }
        let dir = self.vel / v0;
        if t <= self.t_air || self.t_air < EPS {
            // Airborne (or no air time): constant velocity.
            self.origin + self.vel * t
        } else {
            // Grounded: Coulomb deceleration from landing point.
            let a = params.slide_accel;
            let tg = t - self.t_air; // grounded time
            let landing = self.origin + self.vel * self.t_air;
            landing + dir * (v0 * tg - 0.5 * a * tg * tg)
        }
    }
}

pub struct AnalyticPath {
    /// Trajectory segments (between wall bounces).
    pub segments: Vec<TrajectorySegment>,
    /// Where the ball ends up.
    pub end_pos: Vec2,
    /// End time (seconds).
    pub end_time: f32,
    /// Did it score?
    pub scored: Option<BallEndReason>,
}

/// Simulate ball trajectory without walls — pure math.
///
/// Airborne phase: constant planar velocity, vertical parabola.
/// Grounded phase: Coulomb deceleration until stop.
///
/// Returns segments (not samples) for exact analytic interception checking.
pub fn analytic_trajectory(
    pos: Vec2,
    vel: Vec2,
    lift: f32,
    params: &SimParams,
) -> AnalyticPath {
    let v0 = vel.length();
    if v0 < EPS {
        return AnalyticPath {
            segments: vec![TrajectorySegment {
                origin: pos,
                vel: Vec2::ZERO,
                t_air: 0.0,
                t_start: 0.0,
                duration: 0.0,
            }],
            end_pos: pos,
            end_time: 0.0,
            scored: Some(sim_goal_at(pos, params)),
        };
    }

    let a = params.slide_accel;
    let dir = vel / v0;

    // Air time: gravity pulls ball down, it lands when height hits 0.
    let t_air = if lift > 0.0 && params.gravity > 0.0 {
        2.0 * lift / params.gravity
    } else {
        0.0
    };

    // Grounded phase starts at t_air. Coulomb deceleration: v(t) = v0 - a*(t-t_air).
    let t_stop_grounded = v0 / a; // grounded time to stop
    let t_total = t_air + t_stop_grounded;

    // Landing position (where grounded phase starts).
    let landing = pos + vel * t_air;

    // Check if ball crosses goal line before stopping.
    // Ball position at time t: pos + vel*t (airborne) or landing + dir*(v0*tg - 0.5*a*tg²) (grounded).
    let goal_x = if vel.x > 0.0 {
        params.goal_line_x
    } else {
        -params.goal_line_x
    };

    if vel.x.abs() > EPS {
        // Try airborne phase first: pos.x + vel.x * t = goal_x
        let mut t_goal = None;
        if t_air > 0.0 {
            let t = (goal_x - pos.x) / vel.x;
            if t > 0.0 && t <= t_air {
                t_goal = Some(t);
            }
        }
        // Try grounded phase: landing.x + dir.x*(v0*tg - 0.5*a*tg²) = goal_x
        if t_goal.is_none() {
            let dx = goal_x - landing.x;
            let decel_x = a * dir.x;
            if decel_x.abs() > EPS {
                let disc = vel.x * vel.x - 2.0 * decel_x * dx;
                if disc >= 0.0 {
                    let tg = (vel.x - disc.sqrt()) / decel_x;
                    if tg > 0.0 && tg <= t_stop_grounded {
                        t_goal = Some(t_air + tg);
                    }
                }
            } else if vel.x.abs() > EPS {
                let tg = dx / vel.x;
                if tg > 0.0 && tg <= t_stop_grounded {
                    t_goal = Some(t_air + tg);
                }
            }
        }

        if let Some(tg) = t_goal {
            let goal_pos = if tg <= t_air {
                pos + vel * tg
            } else {
                let tg_g = tg - t_air;
                landing + dir * (v0 * tg_g - 0.5 * a * tg_g * tg_g)
            };
            if goal_pos.y.abs() <= params.goal_half_width {
                return AnalyticPath {
                    segments: vec![TrajectorySegment {
                        origin: pos,
                        vel,
                        t_air,
                        t_start: 0.0,
                        duration: tg,
                    }],
                    end_pos: goal_pos,
                    end_time: tg,
                    scored: Some(if vel.x > 0.0 {
                        BallEndReason::GoalAway
                    } else {
                        BallEndReason::GoalHome
                    }),
                };
            }
        }
    }

    // Ball doesn't score — it stops after Coulomb deceleration.
    let stop_pos = landing + dir * (v0 * v0 / (2.0 * a));

    AnalyticPath {
        segments: vec![TrajectorySegment {
            origin: pos,
            vel,
            t_air,
            t_start: 0.0,
            duration: t_total,
        }],
        end_pos: stop_pos,
        end_time: t_total,
        scored: None,
    }
}

/// Find where the trajectory hits a wall (if any).
/// Returns (hit_pos, hit_time, wall_normal_axis) or None.
///
/// Handles both airborne (t < t_air, linear) and grounded (t >= t_air, Coulomb) phases.
fn find_wall_hit(
    pos: Vec2,
    vel: Vec2,
    t_air: f32,
    t_max: f32,
    params: &SimParams,
) -> Option<(Vec2, f32, usize)> {
    let r = params.ball_radius;
    let (x_lo, x_hi) = (params.x_min + r, params.x_max - r);
    let (z_lo, z_hi) = (params.z_min + r, params.z_max - r);

    let v0 = vel.length();
    if v0 < EPS {
        return None;
    }
    let dir = vel / v0;
    let a = params.slide_accel;
    let landing = pos + vel * t_air;

    // Position at time t (handles airborne→grounded transition).
    let pos_at = |t: f32| -> Vec2 {
        if t <= t_air {
            pos + vel * t
        } else {
            let tg = t - t_air;
            landing + dir * (v0 * tg - 0.5 * a * tg * tg)
        }
    };

    // Solve for t when a position component reaches a boundary.
    // Airborne (t <= t_air): linear. pos_comp + vel_comp * t = boundary.
    // Grounded (t > t_air): landing_comp + dir_comp*(v0*tg - 0.5*a*tg²) = boundary.
    let solve = |vel_comp: f32, dir_comp: f32, boundary: f32, p_comp: f32, landing_comp: f32| -> Option<f32> {
        if vel_comp.abs() < EPS {
            return None;
        }
        // Try airborne phase.
        if t_air > 0.0 {
            let t = (boundary - p_comp) / vel_comp;
            if t > 0.0 && t <= t_air {
                return Some(t);
            }
        }
        // Try grounded phase: landing_comp + dir_comp*(v0*tg - 0.5*a*tg²) = boundary
        // => 0.5*a*dir_comp*tg² - v0*dir_comp*tg + (boundary - landing_comp) = 0
        let decel_comp = a * dir_comp;
        let dx = boundary - landing_comp;
        if decel_comp.abs() < EPS {
            // No deceleration component: v0*dir_comp*tg = dx => tg = dx / (v0*dir_comp)
            let denom = v0 * dir_comp;
            if denom.abs() < EPS {
                return None;
            }
            let tg = dx / denom;
            if tg > 0.0 && tg <= t_max - t_air {
                return Some(t_air + tg);
            }
            return None;
        }
        let disc = (v0 * dir_comp) * (v0 * dir_comp) - 2.0 * decel_comp * dx;
        if disc < 0.0 {
            return None;
        }
        let sq = disc.sqrt();
        let tg1 = (v0 * dir_comp - sq) / decel_comp;
        let tg2 = (v0 * dir_comp + sq) / decel_comp;
        let tg_max = t_max - t_air;
        for &tg in &[tg1, tg2] {
            if tg > 0.0 && tg <= tg_max {
                return Some(t_air + tg);
            }
        }
        None
    };

    let mut best_t = t_max + 1.0;
    let mut best_axis = 0;

    // X walls (skip if in goal mouth at hit point).
    if vel.x > EPS {
        if let Some(t) = solve(vel.x, dir.x, x_hi, pos.x, landing.x) {
            if t < best_t {
                let p = pos_at(t);
                if p.y.abs() > params.goal_half_width {
                    best_t = t;
                    best_axis = 0;
                }
            }
        }
    } else if vel.x < -EPS {
        if let Some(t) = solve(vel.x, dir.x, x_lo, pos.x, landing.x) {
            if t < best_t {
                let p = pos_at(t);
                if p.y.abs() > params.goal_half_width {
                    best_t = t;
                    best_axis = 0;
                }
            }
        }
    }

    // Z walls.
    if vel.y > EPS {
        if let Some(t) = solve(vel.y, dir.y, z_hi, pos.y, landing.y) {
            if t < best_t {
                best_t = t;
                best_axis = 1;
            }
        }
    } else if vel.y < -EPS {
        if let Some(t) = solve(vel.y, dir.y, z_lo, pos.y, landing.y) {
            if t < best_t {
                best_t = t;
                best_axis = 1;
            }
        }
    }

    if best_t <= t_max {
        let hit_pos = pos_at(best_t);
        Some((hit_pos, best_t, best_axis))
    } else {
        None
    }
}

/// Bounce velocity off a wall axis (mirrors sim_bounce_axis).
fn bounce_axis(vel: &mut Vec2, normal_axis: usize, e: f32, mu: f32) {
    let mut v = *vel;
    let into_speed = if normal_axis == 0 {
        if v.x > 0.0 { v.x } else { -v.x }
    } else if v.y > 0.0 {
        v.y
    } else {
        -v.y
    };
    v[normal_axis] *= -e;
    let mut t = v;
    t[normal_axis] = 0.0;
    let t_speed = t.length();
    let friction_budget = mu * (1.0 + e) * into_speed;
    if t_speed > 0.0 {
        let kill = friction_budget.min(t_speed);
        t *= (t_speed - kill) / t_speed;
        v[1 - normal_axis] = t[1 - normal_axis];
    }
    *vel = v;
}

/// Analytic shot simulation: simulate without walls, find wall hit, bounce,
/// repeat. Up to MAX_BOUNCES iterations.
///
/// Returns the full path (all segments) and whether the ball scores.
fn simulate_shot_analytic(
    origin: Vec2,
    direction: Vec2,
    params: &SimParams,
) -> AnalyticPath {
    let v0 = full_charge_horiz_speed(params);
    let lift = full_charge_lift(params);
    let vel = direction.normalize_or_zero() * v0;
    let t_air_initial = if lift > 0.0 && params.gravity > 0.0 {
        2.0 * lift / params.gravity
    } else {
        0.0
    };

    let mut all_segments = Vec::with_capacity(8);
    let mut cur_pos = origin;
    let mut cur_vel = vel;
    let mut cur_t_air = t_air_initial;
    let mut t_offset = 0.0;

    for _bounce in 0..=MAX_BOUNCES {
        // Simulate this segment without walls.
        let path = analytic_trajectory(cur_pos, cur_vel, lift_from_t_air(cur_t_air, params), params);

        // Collect segments with time offset.
        for seg in &path.segments {
            all_segments.push(TrajectorySegment {
                origin: seg.origin,
                vel: seg.vel,
                t_air: seg.t_air,
                t_start: seg.t_start + t_offset,
                duration: seg.duration,
            });
        }

        if path.scored.is_some() {
            return AnalyticPath {
                segments: all_segments,
                end_pos: path.end_pos,
                end_time: path.end_time + t_offset,
                scored: path.scored,
            };
        }

        // Check if this segment hits a wall before the ball stops.
        let t_max = path.end_time;
        let wall_hit = find_wall_hit(cur_pos, cur_vel, cur_t_air, t_max, params);

        if let Some((hit_pos, hit_t, axis)) = wall_hit {
            // Truncate segments to before the wall hit.
            let hit_abs_t = t_offset + hit_t;
            for seg in all_segments.iter_mut() {
                let seg_end = seg.t_start + seg.duration;
                if seg_end > hit_abs_t + EPS {
                    seg.duration = (hit_abs_t - seg.t_start).max(0.0);
                }
            }
            all_segments.retain(|s| s.duration > EPS);

            // Compute velocity at hit time.
            let speed_before = cur_vel.length();
            let speed_at_hit = if hit_t <= cur_t_air {
                // Airborne: constant planar velocity.
                speed_before
            } else {
                // Grounded: Coulomb deceleration. v = v0 - a*(t - t_air).
                let a = params.slide_accel;
                (speed_before - a * (hit_t - cur_t_air)).max(0.0)
            };
            let mut new_vel = if speed_at_hit > 0.0 {
                cur_vel.normalize_or_zero() * speed_at_hit
            } else {
                Vec2::ZERO
            };
            bounce_axis(&mut new_vel, axis, params.wall_e, params.wall_mu);

            cur_pos = hit_pos;
            cur_vel = new_vel;
            cur_t_air = 0.0; // Ball is grounded after wall bounce
            t_offset = hit_abs_t;

            if cur_vel.length() <= params.stop_speed_eps {
                break;
            }
        } else {
            break;
        }
    }

    let end_pos = all_segments.last()
        .map(|s| s.pos_at(s.duration, params))
        .unwrap_or(origin);
    let end_time = all_segments.last()
        .map(|s| s.t_start + s.duration)
        .unwrap_or(0.0);

    AnalyticPath {
        segments: all_segments,
        end_pos,
        end_time,
        scored: None,
    }
}

/// Convert t_air back to lift for analytic_trajectory call.
fn lift_from_t_air(t_air: f32, params: &SimParams) -> f32 {
    if t_air <= 0.0 || params.gravity <= 0.0 {
        0.0
    } else {
        t_air * params.gravity * 0.5
    }
}

// ─── Tick-by-tick simulation (for loose ball only) ───────────────────────────

/// Simulated ball state (mirrors crate::ball::Ball).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct SimBall {
    pos: Vec2,
    vel: Vec2,
    height: f32,
    vel_y: f32,
}

impl SimBall {
    #[allow(dead_code)]
    fn new(pos: Vec2, vel: Vec2, lift: f32) -> Self {
        Self {
            pos,
            vel,
            height: 0.33,
            vel_y: lift,
        }
    }

    #[allow(dead_code)]
    fn grounded(&self, params: &SimParams) -> bool {
        self.height <= params.ball_rest_height + 1e-4 && self.vel_y <= 0.05
    }
}

#[allow(dead_code)]
fn sim_resolve_walls(ball: &mut SimBall, params: &SimParams) {
    let e = params.wall_e;
    let mu = params.wall_mu;
    let r = params.ball_radius;
    let (x_lo, x_hi) = (params.x_min + r, params.x_max - r);
    let (z_lo, z_hi) = (params.z_min + r, params.z_max - r);

    if ball.pos.y < z_lo {
        ball.pos.y = z_lo;
        if ball.vel.y < 0.0 {
            bounce_axis(&mut ball.vel, 1, e, mu);
        }
    } else if ball.pos.y > z_hi {
        ball.pos.y = z_hi;
        if ball.vel.y > 0.0 {
            bounce_axis(&mut ball.vel, 1, e, mu);
        }
    }

    let in_mouth = ball.pos.y.abs() <= params.goal_half_width;
    if ball.pos.x < x_lo && !in_mouth {
        ball.pos.x = x_lo;
        if ball.vel.x < 0.0 {
            bounce_axis(&mut ball.vel, 0, e, mu);
        }
    } else if ball.pos.x > x_hi && !in_mouth {
        ball.pos.x = x_hi;
        if ball.vel.x > 0.0 {
            bounce_axis(&mut ball.vel, 0, e, mu);
        }
    }
}

#[allow(dead_code)]
fn sim_resolve_posts(ball: &mut SimBall, params: &SimParams) {
    if sim_goal_at(ball.pos, params) != BallEndReason::None {
        return;
    }
    let contact_r = params.post_contact_radius;
    let e = params.wall_e;
    let mu = params.wall_mu;
    for &(sx, sz) in &[
        (params.posts_x, params.goal_half_width),
        (params.posts_x, -params.goal_half_width),
        (-params.posts_x, params.goal_half_width),
        (-params.posts_x, -params.goal_half_width),
    ] {
        let c = Vec2::new(sx, sz);
        let delta = ball.pos - c;
        let dist = delta.length();
        if dist >= contact_r || dist < 1e-8 {
            continue;
        }
        let n = delta / dist;
        ball.pos = c + n * contact_r;
        // Circle bounce for posts.
        let vn = ball.vel.dot(n);
        if vn < 0.0 {
            let into = -vn;
            let mut t = ball.vel - n * vn;
            let t_speed = t.length();
            let friction_budget = mu * (1.0 + e) * into;
            if t_speed > 0.0 {
                let kill = friction_budget.min(t_speed);
                t *= (t_speed - kill) / t_speed;
            }
            ball.vel = t + n * (into * e);
        }
    }
}

/// One tick of ball physics — exact copy of step_free_ball logic.
#[allow(dead_code)]
fn sim_ball_step(ball: &mut SimBall, params: &SimParams, dt: f32) -> BallEndReason {
    ball.height += ball.vel_y * dt - 0.5 * params.gravity * dt * dt;
    ball.vel_y -= params.gravity * dt;
    if ball.height <= params.ball_rest_height {
        ball.height = params.ball_rest_height;
        if ball.vel_y < 0.0 {
            let bounce = -ball.vel_y * params.ball_bounce_e;
            ball.vel_y = if bounce < params.ball_bounce_settle { 0.0 } else { bounce };
        }
    }

    let grounded = ball.grounded(params);
    let speed = ball.vel.length();
    if speed <= params.stop_speed_eps {
        ball.vel = Vec2::ZERO;
        sim_resolve_walls(ball, params);
        let scored = sim_goal_at(ball.pos, params);
        if scored != BallEndReason::None { return scored; }
        sim_resolve_posts(ball, params);
        return sim_goal_at(ball.pos, params);
    }

    if grounded {
        let a = params.slide_accel;
        let t_stop = speed / a;
        let dt_use = dt.min(t_stop);
        let dir = ball.vel / speed;
        let accel = -dir * a;
        ball.pos += ball.vel * dt_use + 0.5 * accel * dt_use * dt_use;
        ball.vel += accel * dt_use;
        if ball.vel.length() <= params.stop_speed_eps || dt_use >= t_stop {
            ball.vel = Vec2::ZERO;
        }
    } else {
        ball.pos += ball.vel * dt;
    }

    sim_resolve_walls(ball, params);
    let scored = sim_goal_at(ball.pos, params);
    if scored != BallEndReason::None { return scored; }
    sim_resolve_posts(ball, params);
    sim_goal_at(ball.pos, params)
}

/// Simulate a loose ball tick-by-tick for up to MAX_LOOSE_TICKS ticks.
/// Returns positions at each tick (for interception checks with wall bounces).
#[allow(dead_code)]
fn simulate_loose_ball(pos: Vec2, vel: Vec2, params: &SimParams) -> Vec<(Vec2, f32)> {
    let mut ball = SimBall::new(pos, vel, 0.0);
    let dt = FIXED_DT;
    let mut positions = Vec::with_capacity(MAX_LOOSE_TICKS + 1);
    positions.push((pos, 0.0));

    for tick in 0..MAX_LOOSE_TICKS {
        let reason = sim_ball_step(&mut ball, params, dt);
        let t = (tick + 1) as f32 * dt;
        positions.push((ball.pos, t));
        if reason != BallEndReason::None || ball.vel.length() <= params.stop_speed_eps {
            break;
        }
    }

    positions
}

// ─── Analytic interception (exact, no sampling) ──────────────────────────────

/// Solve for the earliest time `t` in [0, t_end] where `player_pos` can intercept
/// the ball on this segment. Returns t (relative to segment start) or None.
///
/// Ball always moves in a straight line (airborne = constant speed, grounded = Coulomb
/// deceleration along same line). f(t) = distance(ball(t), player) - (sprint*t + r_int)
/// is continuous and convex (distance to a line is convex, minus linear reach = convex).
/// So if both endpoints are positive, there's no crossing — no interception on this segment.
/// Otherwise, binary search for the first crossing.
fn intercept_segment(
    seg: &TrajectorySegment,
    player_pos: Vec2,
    params: &SimParams,
) -> Option<f32> {
    intercept_segment_with_headstart(seg, player_pos, 0.0, params)
}

/// Same as intercept_segment but gives the player a time head start:
/// they've already been sprinting for `head_start` seconds before the ball
/// is kicked, so their effective reach at time t is sprint*(t + head_start) + r_int.
fn intercept_segment_with_headstart(
    seg: &TrajectorySegment,
    player_pos: Vec2,
    head_start: f32,
    params: &SimParams,
) -> Option<f32> {
    let sprint = params.player_max_speed;
    let r_int = params.interact_radius;
    let t_end = seg.duration;
    if t_end < EPS {
        return None;
    }

    let f = |t: f32| -> f32 {
        let ball = seg.pos_at(t, params);
        (ball - player_pos).length() - (sprint * (t + head_start) + r_int)
    };

    let f0 = f(0.0);
    if f0 <= EPS {
        return Some(0.0);
    }
    let f_end = f(t_end);
    if f_end > EPS {
        return None;
    }

    let mut lo = 0.0f32;
    let mut hi = t_end;
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if f(mid) <= EPS {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    Some(hi)
}

/// Can `player_pos` intercept the ball along any segment of the trajectory?
/// Returns true at the first segment where interception is possible.
fn can_intercept_segments(
    segments: &[TrajectorySegment],
    player_pos: Vec2,
    params: &SimParams,
) -> bool {
    segments.iter().any(|seg| intercept_segment(seg, player_pos, params).is_some())
}

/// Can `player_pos` intercept with a time head start?
fn can_intercept_segments_headstart(
    segments: &[TrajectorySegment],
    player_pos: Vec2,
    head_start: f32,
    params: &SimParams,
) -> bool {
    segments.iter().any(|seg| intercept_segment_with_headstart(seg, player_pos, head_start, params).is_some())
}

/// Can any opponent intercept? Sorts by distance to ball origin, early exit.
pub fn any_can_intercept_pure_math(
    segments: &[TrajectorySegment],
    origin: Vec2,
    opponents: &[Vec2],
    params: &SimParams,
) -> bool {
    any_can_intercept_pure_math_headstart(segments, origin, opponents, 0.0, params)
}

/// Same as any_can_intercept_pure_math but gives all opponents a time head start.
pub fn any_can_intercept_pure_math_headstart(
    segments: &[TrajectorySegment],
    origin: Vec2,
    opponents: &[Vec2],
    head_start: f32,
    params: &SimParams,
) -> bool {
    let mut sorted: Vec<(f32, Vec2)> = opponents
        .iter()
        .map(|&p| ((p - origin).length(), p))
        .collect();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    sorted.iter().any(|&(_, opp)| {
        can_intercept_segments_headstart(segments, opp, head_start, params)
    })
}

/// Earliest intercept point + time for a player along the full trajectory.
/// Iterates segments in order, returns the first segment where interception
/// is possible, with the exact time within that segment.
pub fn earliest_intercept(
    segments: &[TrajectorySegment],
    player_pos: Vec2,
    params: &SimParams,
) -> Option<(Vec2, f32)> {
    for seg in segments {
        if let Some(t_local) = intercept_segment(seg, player_pos, params) {
            let abs_t = seg.t_start + t_local;
            let pos = seg.pos_at(t_local, params);
            return Some((pos, abs_t));
        }
    }
    None
}

/// Post-tangent direction: the aim direction that clears the post's physical body.
/// Returns the direction from `origin` that grazes the post on the inward side.
fn post_tangent_dir(origin: Vec2, post: Vec2, contact_r: f32) -> Vec2 {
    let offset = post - origin;
    let dist = offset.length();
    if dist < contact_r + EPS {
        // Origin is too close to the post — just aim at the post.
        return offset.normalize_or_zero();
    }
    let toward = offset / dist;
    let alpha = (contact_r / dist).asin();
    // Two candidate tangent directions: rotate toward by +/- alpha.
    let cos_a = alpha.cos();
    let sin_a = alpha.sin();
    let cand_a = Vec2::new(
        toward.x * cos_a - toward.y * sin_a,
        toward.x * sin_a + toward.y * cos_a,
    );
    let cand_b = Vec2::new(
        toward.x * cos_a + toward.y * sin_a,
        -toward.x * sin_a + toward.y * cos_a,
    );
    // Pick the tangent that lands closer to the goal center (z=0).
    let l_a = (dist * dist - contact_r * contact_r).max(0.0).sqrt();
    let pt_a = origin + cand_a * l_a;
    let pt_b = origin + cand_b * l_a;
    if pt_a.y.abs() <= pt_b.y.abs() {
        cand_a
    } else {
        cand_b
    }
}

/// Shot candidate: an aim point and the direction to it.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct ShotCandidate {
    aim_point: Vec2,
    direction: Vec2,
    label: &'static str,
}

/// Generate shot candidates: goal center, left post tangent, right post tangent.
fn shot_candidates(
    origin: Vec2,
    goal_center: Vec2,
    left_post: Vec2,
    right_post: Vec2,
    contact_r: f32,
) -> Vec<ShotCandidate> {
    let mut cands = Vec::with_capacity(5);

    // 1. Goal center
    let dir_center = (goal_center - origin).normalize_or_zero();
    cands.push(ShotCandidate {
        aim_point: goal_center,
        direction: dir_center,
        label: "center",
    });

    // 2. Left post tangent (inward side)
    let lt_dir = post_tangent_dir(origin, left_post, contact_r);
    cands.push(ShotCandidate {
        aim_point: left_post,
        direction: lt_dir,
        label: "left",
    });

    // 3. Right post tangent (inward side)
    let rt_dir = post_tangent_dir(origin, right_post, contact_r);
    cands.push(ShotCandidate {
        aim_point: right_post,
        direction: rt_dir,
        label: "right",
    });

    cands
}

/// Extracted world state from the TeamApi for deterministic calculations.
pub struct DeterministicState {
    pub ball: Vec2,
    pub ball_vel: Vec2,
    pub ball_held: bool,
    pub ball_loose: bool,
    pub carrier_slot: Option<u8>,
    pub team_players: [Vec2; 4],
    pub team_staminas: [f32; 4],
    pub team_shot_charges: [f32; 4],
    pub opp_players: [Vec2; 4],
    pub opp_staminas: [f32; 4],
    pub opp_goal_center: Vec2,
    pub opp_goal_left: Vec2,
    pub opp_goal_right: Vec2,
    pub team_goal_center: Vec2,
    pub interact_radius: f32,
    pub shot_charge: f32,
    pub is_home: bool,
    /// Carrier's movement direction from last tick (where they were walking).
    /// Used for "what if I shot now" simulation. None if no carrier or no movement.
    pub carrier_move_dir: Option<Vec2>,
}

impl DeterministicState {
    /// Extract all needed state from the TeamApi sensor snapshot.
    pub fn from_api(api: &TeamApi, params: &SimParams) -> Self {
        let team = api.team;
        let is_home = team == TeamId::Home;

        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let ball_vel = api
            .get_vector3("Ball Velocity")
            .flatten()
            .unwrap_or(Vec2::ZERO);

        let ball_held = api.get_bool("Team Has Ball").unwrap_or(false)
            || api.get_bool("Opponent Has Ball").unwrap_or(false);
        let ball_loose = api.get_bool("Is Ball Loose").unwrap_or(false);

        let carrier_slot = (1..=4u8).find(|&i| {
            api.get_bool(&format!("Team Player {i} Has Ball")).unwrap_or(false)
        });

        let mut team_players = [Vec2::ZERO; 4];
        let mut team_staminas = [0.0f32; 4];
        let mut team_shot_charges = [0.0f32; 4];
        for i in 0..4 {
            let slot = i + 1;
            team_players[i] = api
                .get_transform(&format!("Team Player {slot}"))
                .unwrap_or(Vec2::ZERO);
            team_staminas[i] = api
                .get_float(&format!("Team Player {slot} Stamina"))
                .unwrap_or(0.0);
            team_shot_charges[i] = api
                .get_float(&format!("Teammate {slot} Shot Charge"))
                .unwrap_or(0.0);
        }

        let mut opp_players = [Vec2::ZERO; 4];
        let mut opp_staminas = [0.0f32; 4];
        for i in 0..4 {
            let slot = i + 1;
            opp_players[i] = api
                .get_transform(&format!("Opponent Player {slot}"))
                .unwrap_or(Vec2::ZERO);
            opp_staminas[i] = api
                .get_float(&format!("Opponent Player {slot} Stamina"))
                .unwrap_or(0.0);
        }

        let opp_goal = api
            .get_transform("Opponent Goal Center")
            .unwrap_or_else(|| {
                let x = if is_home { params.goal_line_x } else { -params.goal_line_x };
                Vec2::new(x, 0.0)
            });
        let opp_goal_left = api
            .get_transform("Opponent Goal Left Post")
            .unwrap_or_else(|| {
                let x = if is_home { params.goal_line_x } else { -params.goal_line_x };
                Vec2::new(x, params.goal_half_width)
            });
        let opp_goal_right = api
            .get_transform("Opponent Goal Right Post")
            .unwrap_or_else(|| {
                let x = if is_home { params.goal_line_x } else { -params.goal_line_x };
                Vec2::new(x, -params.goal_half_width)
            });
        let team_goal = api
            .get_transform("Team Goal Center")
            .unwrap_or_else(|| {
                let x = if is_home { -params.goal_line_x } else { params.goal_line_x };
                Vec2::new(x, 0.0)
            });

        let shot_charge = api.get_float("Ball Carrier Shot Charge").unwrap_or(0.0);
        let interact_radius = api.get_float("Player Interact Radius").unwrap_or(params.interact_radius);

        // Override shot charge from the carrier's own charge if we know who has the ball.
        if let Some(slot) = carrier_slot {
            let idx = (slot as usize).saturating_sub(1).min(3);
            team_shot_charges[idx] = shot_charge;
        }

        Self {
            ball,
            ball_vel,
            ball_held,
            ball_loose,
            carrier_slot,
            team_players,
            team_staminas,
            team_shot_charges,
            opp_players,
            opp_staminas,
            opp_goal_center: opp_goal,
            opp_goal_left,
            opp_goal_right,
            team_goal_center: team_goal,
            interact_radius,
            shot_charge,
            is_home,
            carrier_move_dir: None,
        }
    }

    /// Opponent positions as a slice.
    fn opponents(&self) -> &[Vec2] {
        &self.opp_players
    }

    /// Is the ball already free and heading toward the opponent goal unopposed?
    /// Uses analytic trajectory (pure math, no tick-by-tick) + pure-math interception.
    fn ball_heading_to_goal_unopposed(&self, params: &SimParams) -> bool {
        if self.ball_held {
            return false;
        }
        let vel = self.ball_vel;
        if vel.length() < 1.0 {
            return false;
        }
        let dir = vel.normalize();
        let toward = if self.is_home { dir.x > 0.0 } else { dir.x < 0.0 };
        if !toward {
            return false;
        }
        // Analytic trajectory (no walls, pure math).
        let path = analytic_trajectory(self.ball, vel, 0.0, params);
        let want = attacking_goal_reason(self.is_home);
        if path.scored != Some(want) {
            return false;
        }
        !any_can_intercept_pure_math(&path.segments, self.ball, self.opponents(), params)
    }
}

/// Result of the deterministic evaluation for one player.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicCommand {
    /// If true, override the AI's move_to with this target.
    pub override_move: bool,
    pub move_to: Vec2,
    /// If true, override the AI's interact flag.
    pub override_interact: bool,
    pub interact: bool,
    /// If true, override the AI's sprint flag.
    pub override_sprint: bool,
    pub sprint: bool,
}

impl DeterministicCommand {
    fn face_and_release(aim_dir: Vec2, carrier_pos: Vec2) -> Self {
        Self {
            override_move: true,
            move_to: carrier_pos + aim_dir * 100.0,
            override_interact: true,
            interact: false, // Release to kick
            override_sprint: true,
            sprint: false,
        }
    }

    fn charge() -> Self {
        Self {
            override_interact: true,
            interact: true, // Hold to charge
            ..Self::default()
        }
    }
}

/// What the deterministic layer wants to do this tick.
#[derive(Debug, Clone, Default)]
pub struct DeterministicOutput {
    /// Per-player deterministic commands. Index = player slot - 1.
    pub commands: [DeterministicCommand; 4],
    /// Bools for the AI to read.
    pub can_score_left: bool,
    pub can_score_right: bool,
    pub can_score_center: bool,
    pub can_score: bool,
    /// Per-teammate: can we pass to them without interception?
    pub can_pass_to: [bool; 4],
    /// Per-teammate: if they receive the pass, can THEY score?
    pub can_mate_score: [bool; 4],
    /// Can the carrier walk the ball into the goal?
    pub can_walk_in: bool,
    /// Is the ball already heading to goal unopposed?
    pub ball_going_in: bool,
    /// Which slot has the ball (if any).
    pub carrier_slot: Option<u8>,
    /// The best shot direction if can_score.
    pub shot_direction: Option<Vec2>,
    /// The best pass target if a guaranteed pass exists.
    pub best_pass_target: Option<(u8, Vec2)>,

    // ── Loose ball analysis (pure math, no collision simulation) ──
    /// Where the loose ball will stop rolling (None if held or going in goal).
    pub ball_stop_pos: Option<Vec2>,
    /// Per-teammate: earliest point where they can intercept the loose ball.
    pub team_intercept_pos: [Option<Vec2>; 4],
    /// Per-teammate: time (seconds) to reach the intercept point.
    pub team_intercept_time: [Option<f32>; 4],
    /// Which teammate can intercept the loose ball earliest?
    pub best_intercept_slot: Option<u8>,
    /// Per-opponent: can they intercept the loose ball before any teammate?
    pub opp_can_intercept: [bool; 4],

    // ── "What if I shot right now" analysis (current charge, not full) ──
    /// Where the ball would stop if released right now with current charge.
    pub shot_now_stop_pos: Option<Vec2>,
    /// Per-teammate: earliest intercept point if ball released now.
    pub shot_now_team_intercept: [Option<Vec2>; 4],
    /// Per-opponent: earliest intercept point if ball released now.
    pub shot_now_opp_intercept: [Option<Vec2>; 4],
    /// Who wins the race? true = our team, false = opponent, None = no one.
    pub shot_now_we_win: Option<bool>,
}

impl DeterministicOutput {
    /// Apply deterministic overrides to a BrainOutput (mutates commands).
    pub fn apply_overrides(&self, out: &mut BrainOutput) {
        for i in 0..4 {
            let dc = &self.commands[i];
            if dc.override_move {
                out.commands[i].move_to = dc.move_to;
            }
            if dc.override_interact {
                out.commands[i].interact = dc.interact;
            }
            if dc.override_sprint {
                out.commands[i].sprint = dc.sprint;
            }
        }
    }
}

/// Check if a shot from `origin` in `direction` is a guaranteed goal.
///
/// Uses analytic trajectory simulation (pure math, wall bounces up to 3 iterations)
/// and pure-math interception (no collision simulation for interception check).
///
/// Returns the aim direction if guaranteed, None otherwise.
fn check_shot(
    origin: Vec2,
    direction: Vec2,
    is_home: bool,
    opponents: &[Vec2],
    params: &SimParams,
) -> Option<Vec2> {
    let dir = direction.normalize_or_zero();
    if dir.length() < EPS {
        return None;
    }

    let path = simulate_shot_analytic(origin, dir, params);

    let want = attacking_goal_reason(is_home);
    if path.scored != Some(want) {
        return None;
    }

    if any_can_intercept_pure_math(&path.segments, origin, opponents, params) {
        return None;
    }

    Some(dir)
}

/// Same as check_shot but gives opponents a time head start.
/// Used when checking if a teammate can score after receiving a pass:
/// opponents get pass_time + charge_time of free running before the shot is fired.
fn check_shot_with_headstart(
    origin: Vec2,
    direction: Vec2,
    is_home: bool,
    opponents: &[Vec2],
    head_start: f32,
    params: &SimParams,
) -> Option<Vec2> {
    let dir = direction.normalize_or_zero();
    if dir.length() < EPS {
        return None;
    }

    let path = simulate_shot_analytic(origin, dir, params);

    let want = attacking_goal_reason(is_home);
    if path.scored != Some(want) {
        return None;
    }

    if any_can_intercept_pure_math_headstart(&path.segments, origin, opponents, head_start, params) {
        return None;
    }

    Some(dir)
}

/// Analytic pass trajectory from `origin` toward `target` with moderate charge.
/// Returns segments for interception checking. No collision simulation.
pub fn pass_trajectory(origin: Vec2, target: Vec2, params: &SimParams) -> Vec<TrajectorySegment> {
    let dir = (target - origin).normalize_or_zero();
    let dist = (target - origin).length();
    let pass_charge = 0.4;
    let horiz = (params.kick_speed_base + params.kick_speed_per_charge * pass_charge)
        .min(params.kick_horiz_cap);
    let lift = (params.kick_lift_base + params.kick_lift_per_charge * pass_charge).max(0.0);
    let spd = (horiz * horiz + lift * lift).sqrt();
    let v0 = if spd > params.kick_max_speed && spd > 1e-6 {
        horiz * (params.kick_max_speed / spd)
    } else {
        horiz
    };
    let v0 = v0.max(dist * 1.5);
    let vel = dir * v0;
    let path = analytic_trajectory(origin, vel, lift, params);
    path.segments
}

/// Lead-pass: predict where teammate will be when ball arrives, aim there.
/// Iterates 3 times to converge on travel time.
/// Returns the aim direction.
pub fn lead_pass_direction(
    origin: Vec2,
    mate_pos: Vec2,
    mate_vel: Vec2,
    params: &SimParams,
) -> Vec2 {
    let pass_charge = 0.4;
    let horiz = (params.kick_speed_base + params.kick_speed_per_charge * pass_charge)
        .min(params.kick_horiz_cap);
    let lift = (params.kick_lift_base + params.kick_lift_per_charge * pass_charge).max(0.0);
    let spd = (horiz * horiz + lift * lift).sqrt();
    let base_v0 = if spd > params.kick_max_speed && spd > 1e-6 {
        horiz * (params.kick_max_speed / spd)
    } else {
        horiz
    };

    let mut target = mate_pos;
    for _ in 0..3 {
        let dist = (target - origin).length();
        let v0 = base_v0.max(dist * 1.5);
        let travel_time = if v0 > EPS { dist / v0 } else { 0.0 };
        target = mate_pos + mate_vel * travel_time;
    }
    (target - origin).normalize_or_zero()
}

/// Action the NN chose for the ball carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistAction {
    None,
    ShootLeft,
    ShootCenter,
    ShootRight,
    PassTo1,
    PassTo2,
    PassTo3,
    PassTo4,
}

impl AssistAction {
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => Self::ShootLeft,
            2 => Self::ShootCenter,
            3 => Self::ShootRight,
            4 => Self::PassTo1,
            5 => Self::PassTo2,
            6 => Self::PassTo3,
            7 => Self::PassTo4,
            _ => Self::None,
        }
    }

    pub fn is_shoot(&self) -> bool {
        matches!(self, Self::ShootLeft | Self::ShootCenter | Self::ShootRight)
    }

    pub fn pass_target(&self) -> Option<usize> {
        match self {
            Self::PassTo1 => Some(0),
            Self::PassTo2 => Some(1),
            Self::PassTo3 => Some(2),
            Self::PassTo4 => Some(3),
            _ => None,
        }
    }
}

/// Execute NN-chosen action with perfect mechanics.
/// Returns the move_to override and interact override for the carrier.
pub fn assist_action(
    action: AssistAction,
    carrier_pos: Vec2,
    carrier_idx: usize,
    team_players: &[Vec2; 4],
    team_vels: &[Vec2; 4],
    opp_goal_left: Vec2,
    opp_goal_center: Vec2,
    opp_goal_right: Vec2,
    params: &SimParams,
) -> Option<(Vec2, bool)> {
    match action {
        AssistAction::ShootLeft => {
            let dir = (opp_goal_left - carrier_pos).normalize_or_zero();
            Some((carrier_pos + dir * 100.0, false))
        }
        AssistAction::ShootCenter => {
            let dir = (opp_goal_center - carrier_pos).normalize_or_zero();
            Some((carrier_pos + dir * 100.0, false))
        }
        AssistAction::ShootRight => {
            let dir = (opp_goal_right - carrier_pos).normalize_or_zero();
            Some((carrier_pos + dir * 100.0, false))
        }
        AssistAction::PassTo1
        | AssistAction::PassTo2
        | AssistAction::PassTo3
        | AssistAction::PassTo4 => {
            let mate_i = action.pass_target()?;
            if mate_i == carrier_idx {
                return None;
            }
            let mate_pos = team_players[mate_i];
            let mate_vel = team_vels[mate_i];
            let aim = lead_pass_direction(carrier_pos, mate_pos, mate_vel, params);
            Some((carrier_pos + aim * 100.0, false))
        }
        AssistAction::None => None,
    }
}

/// Check if the carrier can walk the ball into the goal unopposed.
///
/// The carrier walks at `walk_speed` toward the goal mouth. For each tick,
/// check if any opponent can reach the ball position before the carrier gets there.
fn check_walk_in(
    carrier_pos: Vec2,
    _goal_center: Vec2,
    left_post: Vec2,
    right_post: Vec2,
    opponents: &[Vec2],
    params: &SimParams,
) -> bool {
    // Closest point on the goal mouth to the carrier.
    let mouth_target = {
        let ab = right_post - left_post;
        let t = ((carrier_pos - left_post).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
        left_post + ab * t
    };

    let to_goal = mouth_target - carrier_pos;
    let dist = to_goal.length();
    if dist < EPS {
        return true; // Already at the goal mouth
    }

    let dir = to_goal / dist;
    let walk_speed = params.player_walk_speed;

    // For each opponent, check if they can reach any point on the carrier's
    // path before the carrier gets there.
    for &opp in opponents {
        let s = (opp - carrier_pos).dot(dir);
        if s < 0.0 || s > dist {
            continue; // Opponent is behind or beyond the path
        }
        let closest_pt = carrier_pos + dir * s;
        let perp_dist = (opp - closest_pt).length();
        let t_carrier = s / walk_speed;
        // Opponent can sprint to intercept.
        let opp_reach = params.interact_radius + params.player_max_speed * t_carrier;
        if perp_dist <= opp_reach + EPS {
            return false;
        }
    }

    true
}

/// Main entry point: evaluate the deterministic layer for this tick.
///
/// Returns the deterministic output with guaranteed-score detection and
/// auto-charge commands.
pub fn evaluate(state: &DeterministicState, params: &SimParams) -> DeterministicOutput {
    let mut out = DeterministicOutput::default();
    out.carrier_slot = state.carrier_slot;

    let contact_r = params.post_contact_radius;

    // --- Ball already heading to goal unopposed ---
    out.ball_going_in = state.ball_heading_to_goal_unopposed(params);

    // ── Loose ball analysis: stop position + intercept points ──
    if !state.ball_held && state.ball_vel.length() > 0.5 {
        let loose_path = analytic_trajectory(state.ball, state.ball_vel, 0.0, params);

        if loose_path.scored.is_none() {
            out.ball_stop_pos = Some(loose_path.end_pos);
        }

        // Sort teammates by distance to ball origin — closest first.
        let mut team_order: [(f32, usize); 4] =
            [(0.0, 0), (0.0, 1), (0.0, 2), (0.0, 3)];
        for i in 0..4 {
            team_order[i] = ((state.team_players[i] - state.ball).length_squared(), i);
        }
        team_order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Find first teammate who can intercept — they're the closest, most likely.
        let mut best_time = f32::MAX;
        for &(_, i) in &team_order {
            if let Some((pos, t)) = earliest_intercept(&loose_path.segments, state.team_players[i], params) {
                out.team_intercept_pos[i] = Some(pos);
                out.team_intercept_time[i] = Some(t);
                if t < best_time {
                    best_time = t;
                    out.best_intercept_slot = Some((i + 1) as u8);
                }
                break; // First found is closest → earliest. Done.
            }
        }

        // Opponents: sorted by distance, check if any beats our best teammate.
        let mut opp_order: [(f32, usize); 4] =
            [(0.0, 0), (0.0, 1), (0.0, 2), (0.0, 3)];
        for i in 0..4 {
            opp_order[i] = ((state.opp_players[i] - state.ball).length_squared(), i);
        }
        opp_order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        for &(_, i) in &opp_order {
            if let Some((_, t)) = earliest_intercept(&loose_path.segments, state.opp_players[i], params) {
                if t < best_time {
                    out.opp_can_intercept[i] = true;
                    break; // Closest opponent who beats us — that's all we need to know.
                }
            }
        }
    }

    // --- Check shot candidates for the carrier ---
    if let Some(slot) = state.carrier_slot {
        let idx = (slot as usize).saturating_sub(1).min(3);
        let carrier_pos = state.team_players[idx];
        let charge = state.team_shot_charges[idx];
        let opponents = state.opponents();

        // ── "What if I shot right now" analysis at current charge ──
        // Simulate a shot in the direction the carrier was walking last tick.
        // This tells the AI where the ball would end up if released right now.
        if let Some(move_dir) = state.carrier_move_dir {
            if move_dir.length() > EPS && charge > 0.01 {
                let v0 = kick_horiz_at(charge, params);
                let lift = kick_lift_at(charge, params);
                let vel = move_dir * v0;
                let shot_now_path = analytic_trajectory(carrier_pos, vel, lift, params);

                if shot_now_path.scored.is_none() {
                    out.shot_now_stop_pos = Some(shot_now_path.end_pos);
                }

                // Find first teammate who can intercept (sorted by distance).
                let mut team_order: [(f32, usize); 4] =
                    [(0.0, 0), (0.0, 1), (0.0, 2), (0.0, 3)];
                for i in 0..4 {
                    team_order[i] = ((state.team_players[i] - carrier_pos).length_squared(), i);
                }
                team_order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                let mut team_best_t = f32::MAX;
                for &(_, i) in &team_order {
                    if let Some((pos, t)) = earliest_intercept(&shot_now_path.segments, state.team_players[i], params) {
                        out.shot_now_team_intercept[i] = Some(pos);
                        team_best_t = t;
                        break;
                    }
                }

                // Find first opponent who can intercept (sorted by distance).
                let mut opp_order: [(f32, usize); 4] =
                    [(0.0, 0), (0.0, 1), (0.0, 2), (0.0, 3)];
                for i in 0..4 {
                    opp_order[i] = ((state.opp_players[i] - carrier_pos).length_squared(), i);
                }
                opp_order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                let mut opp_best_t = f32::MAX;
                for &(_, i) in &opp_order {
                    if let Some((pos, t)) = earliest_intercept(&shot_now_path.segments, state.opp_players[i], params) {
                        out.shot_now_opp_intercept[i] = Some(pos);
                        opp_best_t = t;
                        break;
                    }
                }

                // Who wins the race?
                if team_best_t < f32::MAX || opp_best_t < f32::MAX {
                    out.shot_now_we_win = Some(team_best_t <= opp_best_t);
                }
            }
        }

        // Generate shot candidates.
        let cands = shot_candidates(
            carrier_pos,
            state.opp_goal_center,
            state.opp_goal_left,
            state.opp_goal_right,
            contact_r,
        );

        // Check each candidate for a guaranteed goal.
        // Order: center, left, right. Take first match (center preferred).
        let mut best_shot: Option<(Vec2, &'static str)> = None;
        for cand in &cands {
            if let Some(dir) = check_shot(
                carrier_pos,
                cand.direction,
                state.is_home,
                opponents,
                params,
            ) {
                match cand.label {
                    "center" => out.can_score_center = true,
                    "left" => out.can_score_left = true,
                    "right" => out.can_score_right = true,
                    _ => {}
                }
                best_shot = Some((dir, cand.label));
                break; // First match wins, center preferred
            }
        }

        out.can_score = out.can_score_left || out.can_score_right || out.can_score_center;
        out.shot_direction = best_shot.map(|(dir, _)| dir);

        // Check walk-in.
        out.can_walk_in = check_walk_in(
            carrier_pos,
            state.opp_goal_center,
            state.opp_goal_left,
            state.opp_goal_right,
            opponents,
            params,
        );

        // --- Auto-charge + release logic ---
        let fully_charged = charge >= 0.99;

        if out.can_score && fully_charged {
            if let Some(aim_dir) = out.shot_direction {
                out.commands[idx] = DeterministicCommand::face_and_release(aim_dir, carrier_pos);
            }
        } else if out.can_score {
            out.commands[idx] = DeterministicCommand::charge();
        } else if out.can_walk_in {
            out.commands[idx] = DeterministicCommand::charge();
        } else {
            out.commands[idx] = DeterministicCommand::charge();
        }

        // --- Check passes to teammates (skip if we can already score) ---
        if !out.can_score {
            for mate_i in 0..4 {
                if mate_i == idx {
                    continue;
                }
                let mate_pos = state.team_players[mate_i];
                let pass_dir = mate_pos - carrier_pos;
                let pass_dist = pass_dir.length();
                if pass_dist < EPS {
                    continue;
                }

                // Pure-math pass interception (no collision simulation).
                let pass_samples = pass_trajectory(carrier_pos, mate_pos, params);
                let pass_interceptable = any_can_intercept_pure_math(&pass_samples, carrier_pos, opponents, params);
                out.can_pass_to[mate_i] = !pass_interceptable;

                if out.can_pass_to[mate_i] {
                    // Compute pass travel time from the trajectory.
                    let pass_time: f32 = pass_samples
                        .iter()
                        .map(|s| s.duration)
                        .sum();
                    // Time for teammate to fully charge after receiving:
                    // warmup + charge time. Teammate starts at charge=0 on pickup.
                    let charge_time = params.shot_charge_warmup_s + params.shot_charge_time_s;
                    let head_start = pass_time + charge_time;

                    // Check shot from mate's current position against opponents
                    // at their CURRENT positions, but with a time head start:
                    // opponents get pass_time + charge_time of free running
                    // toward the shot trajectory before the shot is fired.
                    let mate_cands = shot_candidates(
                        mate_pos,
                        state.opp_goal_center,
                        state.opp_goal_left,
                        state.opp_goal_right,
                        contact_r,
                    );
                    let mate_can_score = mate_cands.iter().any(|c| {
                        check_shot_with_headstart(mate_pos, c.direction, state.is_home, opponents, head_start, params).is_some()
                    });
                    out.can_mate_score[mate_i] = mate_can_score;

                    if mate_can_score && fully_charged {
                        let pass_aim = pass_dir.normalize();
                        out.best_pass_target = Some(((mate_i + 1) as u8, mate_pos));
                        out.commands[idx] =
                            DeterministicCommand::face_and_release(pass_aim, carrier_pos);
                    }
                }
            }
        }
    }

    out
}

/// Persistent cache across ticks for tracking player movement directions.
/// Store this in the brain struct and pass to evaluate_and_apply each tick.
#[derive(Debug, Clone, Default)]
pub struct DeterministicCache {
    /// Previous tick player positions (team + opponent), 8 slots.
    prev_positions: [Vec2; 8],
    /// Previous tick carrier slot, to track movement.
    prev_carrier_slot: Option<u8>,
}

impl DeterministicCache {
    /// Compute the carrier's movement direction from previous tick.
    fn carrier_move_dir(&self, carrier_slot: Option<u8>, team_players: &[Vec2; 4]) -> Option<Vec2> {
        let slot = carrier_slot?;
        let idx = (slot as usize).saturating_sub(1).min(3);
        let prev = self.prev_positions[idx];
        let cur = team_players[idx];
        let delta = cur - prev;
        let len = delta.length();
        if len > 0.01 {
            Some(delta / len)
        } else {
            None
        }
    }

    /// Compute per-player velocities (units/sec) from previous tick positions.
    pub fn team_vels(&self, team_players: &[Vec2; 4], dt: f32) -> [Vec2; 4] {
        let mut vels = [Vec2::ZERO; 4];
        if dt < EPS {
            return vels;
        }
        for i in 0..4 {
            vels[i] = (team_players[i] - self.prev_positions[i]) / dt;
        }
        vels
    }

    /// Update cache with current positions for next tick.
    fn update(&mut self, state: &DeterministicState) {
        for i in 0..4 {
            self.prev_positions[i] = state.team_players[i];
            self.prev_positions[i + 4] = state.opp_players[i];
        }
        self.prev_carrier_slot = state.carrier_slot;
    }
}

/// Convenience: evaluate deterministic layer from API and apply to BrainOutput.
/// Uses cache to track carrier movement direction across ticks.
pub fn evaluate_and_apply(
    api: &TeamApi,
    params: &SimParams,
    out: &mut BrainOutput,
    cache: &mut DeterministicCache,
) -> DeterministicOutput {
    let mut state = DeterministicState::from_api(api, params);
    state.carrier_move_dir = cache.carrier_move_dir(state.carrier_slot, &state.team_players);
    cache.update(&state);
    let det = evaluate(&state, params);
    det.apply_overrides(out);
    det
}

/// Evaluate deterministic layer AND apply NN-chosen actions with aimbot assists.
///
/// Two layers:
/// 1. Guaranteed score detection (always on) — if uninterceptable shot or
///    pass-to-score chain detected, forces it. Overrides everything.
/// 2. NN-invoked aimbot — if no guaranteed score, and NN chose shoot/pass,
///    aimbot aims perfectly at goal corner or teammate (predicting motion).
///    NN decides WHEN and WHO to pass to or WHERE to shoot.
///
/// Movement (move_to, sprint) is NEVER touched by the aimbot — only interact
/// and facing direction when releasing a shot/pass.
pub fn evaluate_and_apply_with_action(
    api: &TeamApi,
    params: &SimParams,
    out: &mut BrainOutput,
    cache: &mut DeterministicCache,
    actions: [AssistAction; 4],
) -> DeterministicOutput {
    let mut state = DeterministicState::from_api(api, params);
    state.carrier_move_dir = cache.carrier_move_dir(state.carrier_slot, &state.team_players);
    let team_vels = cache.team_vels(&state.team_players, FIXED_DT);
    cache.update(&state);

    // Run guaranteed-score detection first.
    let det = evaluate(&state, params);

    // If guaranteed score detected, it takes priority. Apply and return.
    if det.can_score || det.best_pass_target.is_some() || det.can_walk_in || det.ball_going_in {
        det.apply_overrides(out);
        return det;
    }

    // No guaranteed score. Apply NN-chosen aimbot assist for the carrier.
    if let Some(slot) = state.carrier_slot {
        let idx = (slot as usize).saturating_sub(1).min(3);
        let action = actions[idx];
        if action != AssistAction::None {
            let carrier_pos = state.team_players[idx];
            if let Some((aim_target, release)) = assist_action(
                action,
                carrier_pos,
                idx,
                &state.team_players,
                &team_vels,
                state.opp_goal_left,
                state.opp_goal_center,
                state.opp_goal_right,
                params,
            ) {
                // Override facing direction + interact for the carrier only.
                // Movement (move_to) is set to aim direction so the player faces
                // the target. Sprint stays NN-controlled.
                out.commands[idx].move_to = aim_target;
                out.commands[idx].interact = release;
            }
        }
    }

    det
}
