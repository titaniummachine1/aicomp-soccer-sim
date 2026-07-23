//! Cheap fixed-step ball prediction + earliest intercept.
//! Horizon truncates when someone is guaranteed to reach the ball.
//! Sim Vec2 = Unity (X,Z): X=goals, Y=sidelines.

use bevy::prelude::Vec2;

use crate::ball::{goal_at, kick_launch_speeds, step_free_ball, Ball, EndReason};
use crate::params::SimParams;
use crate::world::FIXED_DT;

#[derive(Debug, Clone, Copy)]
pub struct BallSample {
    pub t: f32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub end: EndReason,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intercept {
    pub t: f32,
    pub pos: Vec2,
    pub arriver_dist: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub pos: Vec2,
    pub speed: f32,
}

pub fn predict_ball_path(
    ball: &Ball,
    params: &SimParams,
    dt: f32,
    max_horizon_s: f32,
) -> Vec<BallSample> {
    predict_ball_path_until_intercept(ball, params, dt, max_horizon_s, &[], 0.0)
        .0
}

/// Advance the free ball until it stops/goals **or** the earliest player among
/// `candidates` can touch it. Empty candidates → full path to rest (viz).
///
/// Returns `(path, Some((candidate_idx, intercept)))` when cut by a touch.
pub fn predict_ball_path_until_intercept(
    ball: &Ball,
    params: &SimParams,
    dt: f32,
    max_horizon_s: f32,
    candidates: &[Candidate],
    reach: f32,
) -> (Vec<BallSample>, Option<(usize, Intercept)>) {
    let mut b = *ball;
    b.held = false;
    let mut out = Vec::new();
    out.push(BallSample {
        t: 0.0,
        pos: b.pos,
        vel: b.vel,
        end: EndReason::None,
    });
    if dt <= 0.0 || max_horizon_s <= 0.0 {
        return (out, None);
    }
    let mut t = 0.0;
    while t + dt <= max_horizon_s + 1e-6 {
        let end = step_free_ball(&mut b, params, dt);
        t += dt;
        let sample = BallSample {
            t,
            pos: b.pos,
            vel: b.vel,
            end,
        };
        out.push(sample);

        if !candidates.is_empty() {
            let mut best: Option<(usize, Intercept)> = None;
            for (i, c) in candidates.iter().enumerate() {
                let dist = c.pos.distance(sample.pos);
                if dist <= c.speed.max(0.0) * sample.t + reach + 1e-5 {
                    let hit = Intercept {
                        t: sample.t,
                        pos: sample.pos,
                        arriver_dist: dist,
                    };
                    best = Some(match best {
                        Some((bi, bh))
                            if bh.t < hit.t
                                || ((bh.t - hit.t).abs() <= 1e-6
                                    && bh.arriver_dist <= hit.arriver_dist) =>
                        {
                            (bi, bh)
                        }
                        _ => (i, hit),
                    });
                }
            }
            if best.is_some() {
                return (out, best);
            }
        }

        if end != EndReason::None || goal_at(b.pos, params) != EndReason::None {
            break;
        }
        if b.vel.length_squared() < 1e-8 && b.grounded(params) {
            break;
        }
    }
    (out, None)
}

pub fn earliest_intercept(
    player_pos: Vec2,
    player_speed: f32,
    path: &[BallSample],
    reach: f32,
) -> Option<Intercept> {
    let speed = player_speed.max(0.0);
    for s in path.iter().skip(1) {
        let dist = player_pos.distance(s.pos);
        if dist <= speed * s.t + reach + 1e-5 {
            return Some(Intercept {
                t: s.t,
                pos: s.pos,
                arriver_dist: dist,
            });
        }
    }
    None
}

pub fn truncate_to_guaranteed_intercept(
    path: &[BallSample],
    candidates: &[Candidate],
    reach: f32,
) -> (Vec<BallSample>, Option<(usize, Intercept)>) {
    let mut best: Option<(usize, Intercept)> = None;
    for (i, c) in candidates.iter().enumerate() {
        if let Some(hit) = earliest_intercept(c.pos, c.speed, path, reach) {
            best = Some(match best {
                Some((bi, bh)) if bh.t <= hit.t => (bi, bh),
                _ => (i, hit),
            });
        }
    }
    let Some((ci, hit)) = best else {
        return (path.to_vec(), None);
    };
    let cut = path
        .iter()
        .copied()
        .filter(|s| s.t <= hit.t + 1e-6)
        .collect();
    (cut, Some((ci, hit)))
}

pub fn guaranteed_intercept_horizon(
    path: &[BallSample],
    candidates: &[Candidate],
    reach: f32,
) -> Vec<BallSample> {
    truncate_to_guaranteed_intercept(path, candidates, reach).0
}

/// Deep fallback line just in front of the goal plane.
pub fn gk_cover_x(own_goal_x: f32, reach: f32) -> f32 {
    let depth_sign = if own_goal_x > 0.0 { -1.0 } else { 1.0 };
    own_goal_x + depth_sign * (reach * 0.85).clamp(0.8, 2.5)
}

#[inline]
fn closer_to_own_goal(a: Vec2, b: Vec2, own_goal_x: f32) -> bool {
    (a.x - own_goal_x).abs() < (b.x - own_goal_x).abs()
}

#[inline]
fn clamp_own_half_x(x: f32, own_goal_x: f32) -> f32 {
    if own_goal_x > 0.0 {
        x.max(0.0)
    } else {
        x.min(0.0)
    }
}

fn clamp_gk_cover(mut g: Vec2, threats: &[Vec2], own_goal_x: f32, reach: f32) -> Vec2 {
    let deep = gk_cover_x(own_goal_x, reach);
    g.x = clamp_own_half_x(g.x, own_goal_x);
    if own_goal_x > 0.0 {
        g.x = g.x.min(deep);
    } else {
        g.x = g.x.max(deep);
    }
    for &t in threats {
        if closer_to_own_goal(t, g, own_goal_x) {
            if own_goal_x > 0.0 {
                g.x = g.x.max(t.x);
            } else {
                g.x = g.x.min(t.x);
            }
        }
    }
    g.x = clamp_own_half_x(g.x, own_goal_x);
    g
}

/// Angle-bisector unit from attacker toward the goal mouth (posts L/R).
fn goal_bisector(attacker: Vec2, own_goal_x: f32, goal_half_width: f32) -> Option<Vec2> {
    let left = Vec2::new(own_goal_x, goal_half_width);
    let right = Vec2::new(own_goal_x, -goal_half_width);
    let to_l = left - attacker;
    let to_r = right - attacker;
    let len_l = to_l.length();
    let len_r = to_r.length();
    if len_l < 1e-4 || len_r < 1e-4 {
        return None;
    }
    let sum = to_l / len_l + to_r / len_r;
    let sum_len = sum.length();
    if sum_len < 1e-4 {
        return None;
    }
    Some(sum / sum_len)
}

/// Max-kick projectile from `origin` toward `aim` using real slide friction.
fn max_kick_path(origin: Vec2, aim: Vec2, params: &SimParams) -> Vec<BallSample> {
    let dir = (aim - origin).normalize_or_zero();
    let (horiz, lift) = kick_launch_speeds(1.0, params);
    let ball = Ball {
        pos: origin,
        vel: dir * horiz,
        height: params.ball_rest_height,
        vel_y: lift,
        held: false,
    };
    predict_ball_path(&ball, params, FIXED_DT, 4.0)
}

/// Shrink post aims toward mouth center until a max-kick can still enter the goal.
fn reachable_post_aims(
    origin: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
) -> (Vec2, Vec2) {
    let mut half = goal_half_width.max(0.5);
    for _ in 0..8 {
        let left = Vec2::new(own_goal_x, half);
        let right = Vec2::new(own_goal_x, -half);
        let path_l = max_kick_path(origin, left, params);
        let path_r = max_kick_path(origin, right, params);
        let ok_l = path_l
            .last()
            .is_some_and(|s| matches!(s.end, EndReason::GoalHome | EndReason::GoalAway));
        let ok_r = path_r
            .last()
            .is_some_and(|s| matches!(s.end, EndReason::GoalHome | EndReason::GoalAway));
        if ok_l && ok_r {
            return (left, right);
        }
        half = (half * 0.75).max(0.75);
    }
    (
        Vec2::new(own_goal_x, half),
        Vec2::new(own_goal_x, -half),
    )
}

/// Half-angle α = θ/2 of the shot cone ∠LAR at the attacker.
fn shot_cone_half_angle(attacker: Vec2, left: Vec2, right: Vec2) -> Option<f32> {
    let to_l = left - attacker;
    let to_r = right - attacker;
    let len_l = to_l.length();
    let len_r = to_r.length();
    if len_l < 1e-4 || len_r < 1e-4 {
        return None;
    }
    let cos = (to_l / len_l).dot(to_r / len_r).clamp(-1.0, 1.0);
    let theta = cos.acos();
    Some((0.5 * theta).max(1e-4))
}

/// Closed-form depth guess along the bisector (constant speeds).
///
/// `d_max = r / (sin(α) − (v_g/v_b) cos(α))`
///
/// Returns `None` when the denominator ≤ 0 (model places no forward limit).
/// This is only a **candidate seed** — callers must verify with real kick paths.
pub fn gk_analytical_cover_depth(alpha: f32, reach: f32, vg: f32, vb: f32) -> Option<f32> {
    let vb = vb.max(1e-3);
    let vg = vg.max(0.0);
    let denom = alpha.sin() - (vg / vb) * alpha.cos();
    if denom <= 1e-5 {
        return None;
    }
    Some((reach.max(0.05) / denom).max(0.0))
}

fn path_scores_goal(path: &[BallSample]) -> bool {
    path.iter()
        .any(|s| matches!(s.end, EndReason::GoalHome | EndReason::GoalAway))
}

fn path_goal_time(path: &[BallSample]) -> Option<f32> {
    path.iter()
        .find(|s| matches!(s.end, EndReason::GoalHome | EndReason::GoalAway))
        .map(|s| s.t)
}

/// True if GK at `gk` can touch the ball on `path` before it goals.
fn gk_can_intercept_scoring_path(
    gk: Vec2,
    path: &[BallSample],
    gk_speed: f32,
    reach: f32,
) -> bool {
    if !path_scores_goal(path) {
        // Not a legal scoring threat — nothing to seal.
        return true;
    }
    let Some(goal_t) = path_goal_time(path) else {
        return true;
    };
    match earliest_intercept(gk, gk_speed, path, reach) {
        Some(hit) => hit.t <= goal_t + 1e-4,
        None => false,
    }
}

/// Legal scoring extremes from `shot_origin` (reachable max-kick aims that
/// still enter the goal under real friction). Empty if nothing scores.
pub fn gk_scoring_extremes(
    shot_origin: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
) -> Vec<(Vec2, Vec<BallSample>)> {
    let (aim_l, aim_r) = reachable_post_aims(shot_origin, own_goal_x, goal_half_width, params);
    let mut out = Vec::with_capacity(2);
    for aim in [aim_l, aim_r] {
        let path = max_kick_path(shot_origin, aim, params);
        if path_scores_goal(&path) {
            out.push((aim, path));
        }
    }
    out
}

/// Can GK at `gk` intercept every legal extreme scoring shot from `shot_origin`?
pub fn gk_stand_seals_scoring_extremes(
    gk: Vec2,
    shot_origin: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
) -> bool {
    let reach = params.interact_radius;
    let threats = gk_scoring_extremes(shot_origin, own_goal_x, goal_half_width, params);
    if threats.is_empty() {
        return true;
    }
    threats
        .iter()
        .all(|(_, path)| gk_can_intercept_scoring_path(gk, path, gk_speed, reach))
}

fn gk_axis_toward_goal(
    shot_origin: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
) -> Option<Vec2> {
    goal_bisector(shot_origin, own_goal_x, goal_half_width).or_else(|| {
        let mouth = Vec2::new(
            own_goal_x,
            shot_origin.y.clamp(-goal_half_width, goal_half_width),
        );
        let d = mouth - shot_origin;
        let len = d.length();
        (len > 1e-4).then(|| d / len)
    })
}

/// Closest stand to the ball along the attacker→goal axis that still seals
/// every legal extreme scoring shot (verified with real kick paths).
///
/// Binary-searches depth: advance toward the ball until the first failure,
/// then sit on the safe side of that boundary. Analytical depth is only a
/// search seed — physics decides.
pub fn gk_closest_safe_stand(
    shot_origin: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
) -> Vec2 {
    let reach = params.interact_radius;
    let deep_x = gk_cover_x(own_goal_x, reach);
    let fallback = Vec2::new(
        deep_x,
        shot_origin.y.clamp(-goal_half_width, goal_half_width),
    );

    let Some(axis) = gk_axis_toward_goal(shot_origin, own_goal_x, goal_half_width) else {
        return fallback;
    };
    let to_goal = (own_goal_x - shot_origin.x).signum();
    if axis.x * to_goal <= 0.0 {
        return fallback;
    }

    let d_deep = if axis.x.abs() > 1e-4 {
        ((deep_x - shot_origin.x) / axis.x).max(0.0)
    } else {
        shot_origin.distance(Vec2::new(deep_x, shot_origin.y))
    };
    // Closest candidate: just outside Interact of the hold point.
    let d_near = (reach * 0.35).clamp(0.05, 1.0).min(d_deep);

    let point = |d: f32| {
        let mut g = shot_origin + axis * d;
        g.x = clamp_own_half_x(g.x, own_goal_x);
        if own_goal_x > 0.0 {
            g.x = g.x.min(deep_x);
        } else {
            g.x = g.x.max(deep_x);
        }
        g
    };

    let seals = |d: f32| {
        gk_stand_seals_scoring_extremes(
            point(d),
            shot_origin,
            own_goal_x,
            goal_half_width,
            params,
            gk_speed,
        )
    };

    if !seals(d_deep) {
        // Even the deep line fails (rare / degenerate) — sit deep anyway.
        return fallback;
    }
    if seals(d_near) {
        return point(d_near);
    }

    // Seed with analytical guess when available, then binary-search the
    // unsafe→safe boundary (smallest safe depth = closest to ball).
    let (aim_l, aim_r) = reachable_post_aims(shot_origin, own_goal_x, goal_half_width, params);
    let (vb, _) = kick_launch_speeds(1.0, params);
    let d_seed = shot_cone_half_angle(shot_origin, aim_l, aim_r)
        .and_then(|a| gk_analytical_cover_depth(a, reach, gk_speed, vb))
        .map(|d| d.clamp(d_near, d_deep))
        .unwrap_or((d_near + d_deep) * 0.5);

    let mut lo = d_near; // often unsafe (too close)
    let mut hi = d_deep; // known safe
    if seals(d_seed) {
        hi = d_seed;
    } else {
        lo = d_seed;
    }
    for _ in 0..14 {
        let mid = 0.5 * (lo + hi);
        if seals(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    point(hi)
}

/// Scenario 1 cover stand: closest verified-safe position to the shot origin.
///
/// Prefer this over pure geometry — it verifies L/R legal scoring paths with
/// real friction + [`earliest_intercept`].
pub fn gk_intercept_cover(
    attacker: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
) -> Vec2 {
    gk_closest_safe_stand(attacker, own_goal_x, goal_half_width, params, gk_speed)
}

/// Whether `x` is on the goal side of (or on) the max-advance cover plane.
#[inline]
pub fn gk_on_or_behind_cover(x: f32, cover_x: f32, own_goal_x: f32) -> bool {
    if own_goal_x > 0.0 {
        x + 1e-3 >= cover_x
    } else {
        x - 1e-3 <= cover_x
    }
}

/// Clamp a desired stand onto / behind the max-advance cover plane.
#[inline]
pub fn gk_clamp_to_cover_plane(mut p: Vec2, cover_x: f32, own_goal_x: f32) -> Vec2 {
    if own_goal_x > 0.0 {
        p.x = p.x.max(cover_x);
    } else {
        p.x = p.x.min(cover_x);
    }
    p.x = clamp_own_half_x(p.x, own_goal_x);
    p
}

/// Held-ball press: stand at closest safe cover; tackle when the ball is in
/// Interact range. MoveTo stays on the safe set — do not gate Interact on the
/// cover-plane check (that blocked steals when slightly past cover.x).
///
/// Returns `(move_to, try_interact)`.
pub fn gk_cover_press_target(
    me: Vec2,
    shot_origin: Vec2,
    tackle_at: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
) -> (Vec2, bool) {
    let reach = params.interact_radius;
    let cover = gk_closest_safe_stand(shot_origin, own_goal_x, goal_half_width, params, gk_speed);
    let try_tackle = me.distance(tackle_at) <= reach * 1.2;
    (cover, try_tackle)
}

/// Classic cone-bisector cover (O(1) geometric fallback).
/// Prefer [`gk_intercept_cover`] for live GK standing.
pub fn gk_cone_bisector_cover(
    attacker: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    r: f32,
) -> Vec2 {
    let Some(b) = goal_bisector(attacker, own_goal_x, goal_half_width) else {
        return Vec2::new(gk_cover_x(own_goal_x, r), 0.0);
    };
    let left = Vec2::new(own_goal_x, goal_half_width);
    let right = Vec2::new(own_goal_x, -goal_half_width);
    let hat_l = (left - attacker).normalize_or_zero();
    let hat_r = (right - attacker).normalize_or_zero();
    let cos = hat_l.dot(hat_r).clamp(-1.0, 1.0);
    let theta = cos.acos();
    let half = (0.5 * theta).max(1e-4);
    let d = (r.max(0.05) / half.tan()).max(0.0);

    let mut g = attacker + b * d;
    let to_goal = (own_goal_x - attacker.x).signum();
    if (g.x - own_goal_x) * to_goal >= 0.0 {
        let deep = gk_cover_x(own_goal_x, r);
        let t = if b.x.abs() > 1e-4 {
            (deep - attacker.x) / b.x
        } else {
            0.0
        };
        g = attacker + b * t.max(0.0);
    }
    g
}

pub fn gk_cover_point(
    threat: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
) -> Vec2 {
    gk_intercept_cover(threat, own_goal_x, goal_half_width, params, gk_speed)
}

/// Multi-threat weighted blend of intercept covers + midfield / behind clamps.
pub fn gk_cover_from_threats(
    ball: Vec2,
    threats: &[Vec2],
    carrier: Option<Vec2>,
    gk_now: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    params: &SimParams,
    gk_speed: f32,
    scale_m: f32,
) -> Vec2 {
    let reach = params.interact_radius;
    let scale = scale_m.max(0.5);
    let mut acc = Vec2::ZERO;
    let mut wsum = 0.0;
    let mut best_w = 0.0;
    let mut best_t = ball;
    for &t in threats {
        let mut w = (-t.distance(ball) / scale).exp();
        if let Some(c) = carrier {
            if t.distance(c) < 0.75 {
                w *= 2.5;
            }
            if closer_to_own_goal(t, gk_now, own_goal_x) {
                w *= 4.0;
            }
        }
        if w > best_w {
            best_w = w;
            best_t = t;
        }
        if w < 1e-4 {
            continue;
        }
        acc += gk_intercept_cover(t, own_goal_x, goal_half_width, params, gk_speed) * w;
        wsum += w;
    }
    {
        let w = 0.35;
        acc += gk_intercept_cover(ball, own_goal_x, goal_half_width, params, gk_speed) * w;
        wsum += w;
    }
    let raw = if wsum < 1e-6 {
        gk_intercept_cover(best_t, own_goal_x, goal_half_width, params, gk_speed)
    } else {
        acc / wsum
    };
    clamp_gk_cover(raw, threats, own_goal_x, reach)
}

pub fn primary_ball_threat(ball: Vec2, threats: &[Vec2], scale_m: f32) -> Option<Vec2> {
    let scale = scale_m.max(0.5);
    let mut best: Option<(f32, Vec2)> = None;
    for &t in threats {
        let w = (-t.distance(ball) / scale).exp();
        best = Some(match best {
            Some((bw, bp)) if bw >= w => (bw, bp),
            _ => (w, t),
        });
    }
    best.map(|(_, p)| p)
}

/// Preemptive cut: if `threat` shoots at posts/centre now, where GK can arrive first.
pub fn gk_cut_possible_shot(
    me: Vec2,
    threat: Vec2,
    own_goal_x: f32,
    goal_half_width: f32,
    sprint: f32,
    params: &SimParams,
) -> Option<Vec2> {
    let (horiz, lift) = kick_launch_speeds(0.85, params);
    let reach = params.interact_radius;
    let targets = [
        Vec2::new(own_goal_x, goal_half_width * 0.92),
        Vec2::new(own_goal_x, -goal_half_width * 0.92),
        Vec2::new(own_goal_x, goal_half_width * 0.40),
        Vec2::new(own_goal_x, -goal_half_width * 0.40),
        Vec2::new(own_goal_x, 0.0),
    ];
    let toward = (own_goal_x - threat.x).signum();
    let mut best: Option<(f32, Vec2)> = None;
    for target in targets {
        let dir = (target - threat).normalize_or_zero();
        if dir.length_squared() < 1e-6 || dir.x * toward <= 0.05 {
            continue;
        }
        let ball = Ball {
            pos: threat + dir * 0.2,
            vel: dir * horiz,
            height: params.ball_rest_height,
            vel_y: lift,
            held: false,
        };
        let path = predict_ball_path(&ball, params, FIXED_DT, 2.2);
        if let Some(hit) = earliest_intercept(me, sprint, &path, reach) {
            let goal_t = path
                .iter()
                .find(|s| (s.pos.x - own_goal_x).abs() <= 0.6)
                .map(|s| s.t)
                .unwrap_or(f32::MAX);
            if hit.t + 1e-3 < goal_t {
                let score = -hit.t;
                best = Some(match best {
                    Some((bs, bp)) if bs >= score => (bs, bp),
                    _ => (score, hit.pos),
                });
            }
        }
    }
    best.map(|(_, p)| p)
}

/// GK buy-time clear: maximize upfield travel (even if later collected).
pub fn best_long_clear_dir(
    origin: Vec2,
    attack_sign: f32,
    opponents: &[Candidate],
    charge: f32,
    params: &SimParams,
    safe_radius: f32,
) -> Vec2 {
    let (horiz, lift) = kick_launch_speeds(charge.clamp(0.7, 1.0), params);
    let candidates = [
        Vec2::new(attack_sign, 0.95).normalize_or_zero(),
        Vec2::new(attack_sign, -0.95).normalize_or_zero(),
        Vec2::new(attack_sign, 0.55).normalize_or_zero(),
        Vec2::new(attack_sign, -0.55).normalize_or_zero(),
        Vec2::new(attack_sign, 0.25).normalize_or_zero(),
        Vec2::new(attack_sign, -0.25).normalize_or_zero(),
        Vec2::new(attack_sign, 0.0),
    ];
    let mut best = candidates[0];
    let mut best_score = f32::NEG_INFINITY;
    for direction in candidates {
        let ball = Ball {
            pos: origin + direction * 0.15,
            vel: direction * horiz,
            height: params.ball_rest_height,
            vel_y: lift,
            held: false,
        };
        let path = predict_ball_path(&ball, params, FIXED_DT, 3.5);
        let unsafe_near =
            path_interceptable_near(&path, opponents, params.interact_radius, origin, safe_radius);
        let mut score = 0.0;
        if let Some(last) = path.last() {
            score += (last.pos.x - origin.x) * attack_sign * 4.0;
            score += last.pos.distance(origin) * 1.5;
            score += last.pos.y.abs() * 0.2;
        }
        if !unsafe_near {
            score += 8.0;
        }
        if score > best_score {
            best_score = score;
            best = direction;
        }
    }
    best
}

/// True if any opponent can intercept the flight before `safe_radius` from origin
/// along the path (near-goal danger zone for clears).
pub fn path_interceptable_near(
    path: &[BallSample],
    opponents: &[Candidate],
    reach: f32,
    origin: Vec2,
    safe_radius: f32,
) -> bool {
    for s in path {
        if origin.distance(s.pos) > safe_radius {
            continue;
        }
        for c in opponents {
            if let Some(hit) = earliest_intercept(c.pos, c.speed, path, reach) {
                if hit.t <= s.t + 1e-4 && origin.distance(hit.pos) <= safe_radius {
                    return true;
                }
            }
        }
    }
    false
}

/// Diagonal AIA-style clear that no nearby opponent can snatch inside `safe_r`.
pub fn best_safe_clear_dir(
    origin: Vec2,
    attack_sign: f32,
    opponents: &[Candidate],
    charge: f32,
    params: &SimParams,
    safe_r: f32,
) -> Vec2 {
    let (horiz, lift) = kick_launch_speeds(charge.clamp(0.35, 1.0), params);
    // Prefer diagonals away from own goal (attack_sign toward opp).
    let cands = [
        Vec2::new(attack_sign, 0.85).normalize_or_zero(),
        Vec2::new(attack_sign, -0.85).normalize_or_zero(),
        Vec2::new(attack_sign, 0.45).normalize_or_zero(),
        Vec2::new(attack_sign, -0.45).normalize_or_zero(),
        Vec2::new(attack_sign, 0.0),
    ];
    let mut best = cands[0];
    let mut best_score = f32::NEG_INFINITY;
    for dir in cands {
        if dir.length_squared() < 1e-8 {
            continue;
        }
        let ball = Ball {
            pos: origin + dir * 0.15,
            vel: dir * horiz,
            height: params.ball_rest_height,
            vel_y: lift,
            held: false,
        };
        let path = predict_ball_path(&ball, params, FIXED_DT, 2.0);
        let unsafe_near = path_interceptable_near(&path, opponents, params.interact_radius, origin, safe_r);
        let mut min_sep = f32::MAX;
        for s in &path {
            for c in opponents {
                min_sep = min_sep.min(s.pos.distance(c.pos));
            }
        }
        let mut score = min_sep;
        if !unsafe_near {
            score += 50.0;
        }
        // Prefer longer carry away from own goal.
        if let Some(last) = path.last() {
            score += (last.pos.x - origin.x) * attack_sign * 0.15;
            score += last.pos.y.abs() * 0.05; // diagonal bias
        }
        if score > best_score {
            best_score = score;
            best = dir;
        }
    }
    best
}

/// Forward pass lane: ball reaches teammate before any opponent.
pub fn best_forward_pass_dir(
    origin: Vec2,
    mate: Vec2,
    opponents: &[Candidate],
    charge: f32,
    params: &SimParams,
) -> Option<Vec2> {
    let mut dir = mate - origin;
    if dir.length_squared() < 1e-4 {
        return None;
    }
    dir = dir.normalize();
    // Lead slightly ahead of mate along same lane.
    let lead = mate + dir * 2.5;
    dir = (lead - origin).normalize_or_zero();
    let (horiz, lift) = kick_launch_speeds(charge.clamp(0.25, 0.85), params);
    let ball = Ball {
        pos: origin + dir * 0.15,
        vel: dir * horiz,
        height: params.ball_rest_height,
        vel_y: lift * 0.35,
        held: false,
    };
    let path = predict_ball_path(&ball, params, FIXED_DT, 2.0);
    let mate_hit = earliest_intercept(mate, 8.0, &path, params.interact_radius)?;
    for c in opponents {
        if let Some(oh) = earliest_intercept(c.pos, c.speed, &path, params.interact_radius) {
            if oh.t <= mate_hit.t + 0.05 {
                return None; // opp can snatch
            }
        }
    }
    Some(dir)
}

pub fn best_shot_dir_evading(
    origin: Vec2,
    attack_sign: f32,
    opponents: &[Vec2],
    charge: f32,
    params: &SimParams,
) -> Vec2 {
    let sign = if attack_sign < 0.0 { -1.0 } else { 1.0 };
    let left = Vec2::new(params.goal_line_x * sign, params.goal_half_width);
    let right = Vec2::new(params.goal_line_x * sign, -params.goal_half_width);
    let a0 = (left - origin).normalize_or_zero();
    let a1 = (right - origin).normalize_or_zero();
    let (horiz, lift) = kick_launch_speeds(charge, params);
    let mut best = Vec2::new(sign, 0.0);
    let mut best_score = f32::NEG_INFINITY;
    for i in 0..9 {
        let u = i as f32 / 8.0;
        let dir = (a0 * (1.0 - u) + a1 * u).normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            continue;
        }
        let mut shot = Ball {
            pos: origin + dir * 0.15,
            vel: dir * horiz,
            height: params.ball_rest_height,
            vel_y: lift,
            held: false,
        };
        let mut min_sep = f32::MAX;
        let mut score = 0.0;
        for tick in 1..=((2.5 / FIXED_DT).ceil() as usize) {
            let reason = step_free_ball(&mut shot, params, FIXED_DT);
            for p in opponents {
                min_sep = min_sep.min((*p - shot.pos).length());
            }
            if (reason == EndReason::GoalAway && sign > 0.0)
                || (reason == EndReason::GoalHome && sign < 0.0)
            {
                score += 1000.0 - tick as f32 * 0.05;
                break;
            }
            if opponents
                .iter()
                .any(|p| (*p - shot.pos).length() <= params.interact_radius)
            {
                break;
            }
        }
        score += if min_sep < f32::MAX {
            min_sep * 4.0
        } else {
            1.0
        };
        if score > best_score {
            best_score = score;
            best = dir;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_intercept_respects_speed() {
        let path = vec![
            BallSample {
                t: 0.0,
                pos: Vec2::new(10.0, 0.0),
                vel: Vec2::ZERO,
                end: EndReason::None,
            },
            BallSample {
                t: 1.0,
                pos: Vec2::new(5.0, 0.0),
                vel: Vec2::ZERO,
                end: EndReason::None,
            },
            BallSample {
                t: 2.0,
                pos: Vec2::ZERO,
                vel: Vec2::ZERO,
                end: EndReason::None,
            },
        ];
        let hit = earliest_intercept(Vec2::ZERO, 2.0, &path, 0.1).unwrap();
        assert_eq!(hit.t, 2.0);
    }

    #[test]
    fn gk_analytical_cover_depth_matches_closed_form() {
        let alpha = 0.35_f32;
        let r = 1.75;
        let vg = 8.0;
        let vb = 30.0;
        let d = gk_analytical_cover_depth(alpha, r, vg, vb).unwrap();
        let expected = r / (alpha.sin() - (vg / vb) * alpha.cos());
        assert!((d - expected).abs() < 1e-4, "d={d} expected={expected}");
    }

    #[test]
    fn gk_analytical_infinite_ball_speed_is_r_over_sin() {
        let alpha = 0.4_f32;
        let r = 1.75;
        let d = gk_analytical_cover_depth(alpha, r, 8.0, 1.0e6).unwrap();
        assert!((d - r / alpha.sin()).abs() < 1e-3);
    }

    #[test]
    fn gk_intercept_cover_seals_midfield_threat() {
        let params = SimParams::default();
        let a = Vec2::new(0.0, 3.0);
        let g = gk_intercept_cover(a, 39.5, 6.0, &params, 8.0);
        assert!(g.x >= 0.0 && g.x < 39.5);
        assert!(
            gk_stand_seals_scoring_extremes(g, a, 39.5, 6.0, &params, 8.0),
            "midfield cover must seal, g={g:?}"
        );
        // Closest-safe may sit near the ball when that still seals.
        assert!(g.distance(a) >= 0.05);
    }

    #[test]
    fn gk_intercept_cover_seals_near_box() {
        let params = SimParams::default();
        let a = Vec2::new(28.0, 2.0);
        let intercept = gk_intercept_cover(a, 39.5, 6.0, &params, 8.0);
        assert!(
            gk_stand_seals_scoring_extremes(intercept, a, 39.5, 6.0, &params, 8.0),
            "box cover must seal, intercept={intercept:?}"
        );
        assert!(intercept.x > a.x - 1e-2);
        assert!(intercept.x < 39.5);
    }

    #[test]
    fn gk_cone_bisector_matches_formula() {
        let a = Vec2::new(20.0, 0.0);
        let r = 1.75;
        let g = gk_cone_bisector_cover(a, 39.5, 6.0, r);
        assert!(g.y.abs() < 1e-3);
        assert!(g.x > a.x && g.x < 39.5);
        let left = Vec2::new(39.5, 6.0);
        let right = Vec2::new(39.5, -6.0);
        let hl = (left - a).normalize();
        let hr = (right - a).normalize();
        let theta = hl.dot(hr).clamp(-1.0, 1.0).acos();
        let d = r / (0.5 * theta).tan();
        assert!((g.distance(a) - d).abs() < 1e-2);
    }

    #[test]
    fn gk_closest_safe_seals_extremes_and_does_not_chase() {
        let params = SimParams::default();
        let shot = Vec2::new(10.0, 2.0);
        let me = Vec2::new(30.0, 0.0);
        let cover = gk_closest_safe_stand(shot, 39.5, 6.0, &params, 8.0);
        assert!(
            gk_stand_seals_scoring_extremes(cover, shot, 39.5, 6.0, &params, 8.0),
            "cover must seal legal extremes, cover={cover:?}"
        );
        // One step closer to the ball along the axis should be unsafe (or equal
        // at the near clamp).
        if let Some(axis) = goal_bisector(shot, 39.5, 6.0) {
            let closer = shot + axis * ((cover - shot).length() - 1.5).max(0.2);
            if (closer - shot).length() + 0.5 < (cover - shot).length() {
                assert!(
                    !gk_stand_seals_scoring_extremes(closer, shot, 39.5, 6.0, &params, 8.0)
                        || (cover - shot).length() < 2.0,
                    "expected a closer stand to open a post, closer={closer:?} cover={cover:?}"
                );
            }
        }
        let (target, try_tackle) =
            gk_cover_press_target(me, shot, shot, 39.5, 6.0, &params, 8.0);
        assert!((target - cover).length() < 1e-3);
        assert!(!try_tackle);
        // Ball past GK → retarget cover, never walk onto the ball.
        let past = Vec2::new(36.0, 2.0);
        let recover = gk_closest_safe_stand(past, 39.5, 6.0, &params, 8.0);
        let (stand, _) =
            gk_cover_press_target(Vec2::new(30.0, 0.0), past, past, 39.5, 6.0, &params, 8.0);
        assert!((stand - recover).length() < 1e-3);
        assert!((stand - past).length() > 0.5);
        let near = cover;
        let (_, tackle_near) = gk_cover_press_target(
            near,
            near + Vec2::new(-1.0, 0.0),
            near + Vec2::new(-1.0, 0.0),
            39.5,
            6.0,
            &params,
            8.0,
        );
        assert!(tackle_near);
    }

    #[test]
    fn predict_stops_at_first_interceptor() {
        let params = SimParams::default();
        let ball = Ball {
            pos: Vec2::new(0.0, 0.0),
            vel: Vec2::new(10.0, 0.0),
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: false,
        };
        let full = predict_ball_path(&ball, &params, FIXED_DT, 4.0);
        let cands = [Candidate {
            pos: Vec2::new(2.0, 0.0),
            speed: 8.0,
        }];
        let (cut, hit) =
            predict_ball_path_until_intercept(&ball, &params, FIXED_DT, 4.0, &cands, 1.75);
        assert!(hit.is_some());
        assert!(cut.len() < full.len(), "must stop before rest");
        assert!(cut.last().unwrap().t <= hit.unwrap().1.t + 1e-4);
    }
}
