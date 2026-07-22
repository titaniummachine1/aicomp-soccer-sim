//! Free-ball XZ (2D) kinematics — Coulomb slide + walls + open goal mouths.

use bevy::prelude::*;

use crate::params::SimParams;

#[derive(Component, Debug, Clone, Copy)]
pub struct Ball {
    pub pos: Vec2,
    pub vel: Vec2,
    pub held: bool,
}

impl Default for Ball {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            held: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    None,
    GoalHome,
    GoalAway,
}

pub fn in_goal_mouth(z: f32, half_width: f32) -> bool {
    z.abs() <= half_width
}

pub fn step_free_ball(ball: &mut Ball, params: &SimParams, dt: f32) -> EndReason {
    // Held or parked: still score if the center is already in the net (carry-in).
    if ball.held || dt <= 0.0 {
        return goal_at(ball.pos, params);
    }

    let speed = ball.vel.length();
    if speed <= params.stop_speed_eps {
        ball.vel = Vec2::ZERO;
        return goal_at(ball.pos, params);
    }

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

    resolve_walls(ball, params);

    // Score as soon as the ball-center crosses the goal line in the mouth.
    // Must run BEFORE post bounce — posts sit past the line (±40.2) and were
    // shoving near-post shots back onto the pitch without terminating.
    let scored = goal_at(ball.pos, params);
    if scored != EndReason::None {
        return scored;
    }

    resolve_posts(ball, params);
    goal_at(ball.pos, params)
}

/// Loose-ball circle vs player bodies (body_r + ball_r). Soft restitution so
/// contested midfield doesn't need a fat phantom radius.
/// Hot balls (>8 m/s) pass through — real kicks hang ~1s airborne; ground-only
/// bounce was dumping speed under the pickup gate and killing loose %.
pub fn resolve_player_bodies(ball: &mut Ball, players: &[crate::player::Player], params: &SimParams) {
    if ball.held {
        return;
    }
    if ball.vel.length() > 8.0 {
        return;
    }
    let contact_r = params.body_radius + params.ball_radius;
    let e = params.wall_e;
    let mu = params.wall_mu;
    for p in players {
        let delta = ball.pos - p.pos;
        let dist = delta.length();
        if dist >= contact_r || dist < 1e-8 {
            continue;
        }
        let n = delta / dist;
        ball.pos = p.pos + n * contact_r;
        let v_rel = ball.vel - p.vel;
        let vn = v_rel.dot(n);
        if vn >= 0.0 {
            continue;
        }
        let mut tangent = ball.vel - n * ball.vel.dot(n);
        let into = -vn;
        let reflected_n = n * (into * e);
        let push = n * (p.vel.dot(n).max(0.0));
        let t_speed = tangent.length();
        let friction_budget = mu * (1.0 + e) * into;
        if t_speed > 0.0 {
            let kill = friction_budget.min(t_speed);
            tangent *= (t_speed - kill) / t_speed;
        }
        ball.vel = tangent + reflected_n + push;
    }
}

fn resolve_posts(ball: &mut Ball, params: &SimParams) {
    // Past the goal line inside the mouth = scoring volume; ignore posts/net.
    if goal_at(ball.pos, params) != EndReason::None {
        return;
    }
    let contact_r = params.post_contact_radius;
    let e = params.wall_e;
    let mu = params.wall_mu;
    for sx in [-1.0_f32, 1.0] {
        for sz in [-1.0_f32, 1.0] {
            let post = Vec2::new(sx * params.posts_x, sz * params.goal_half_width);
            let delta = ball.pos - post;
            let dist = delta.length();
            if dist >= contact_r || dist < 1e-8 {
                continue;
            }
            let n = delta / dist;
            // Push ball center out to contact surface
            ball.pos = post + n * contact_r;
            let vn = ball.vel.dot(n);
            if vn >= 0.0 {
                continue; // separating
            }
            // Reflect normal * e, Coulomb tangent friction
            let mut v = ball.vel - n * vn; // tangent part
            let into = -vn;
            let reflected_n = n * (into * e);
            let t_speed = v.length();
            let friction_budget = mu * (1.0 + e) * into;
            if t_speed > 0.0 {
                let kill = friction_budget.min(t_speed);
                v *= (t_speed - kill) / t_speed;
            }
            ball.vel = v + reflected_n;
        }
    }
}

/// True when ball center is past a goal line inside the mouth (held or free).
pub fn goal_at(pos: Vec2, params: &SimParams) -> EndReason {
    if pos.x <= -params.goal_line_x && in_goal_mouth(pos.y, params.goal_half_width) {
        return EndReason::GoalHome;
    }
    if pos.x >= params.goal_line_x && in_goal_mouth(pos.y, params.goal_half_width) {
        return EndReason::GoalAway;
    }
    EndReason::None
}

fn resolve_walls(ball: &mut Ball, params: &SimParams) {
    let e = params.wall_e;
    let mu = params.wall_mu;
    let mouth = params.goal_half_width;

    if ball.pos.y < params.z_min {
        ball.pos.y = params.z_min;
        bounce_axis(&mut ball.vel, 1, e, mu);
    } else if ball.pos.y > params.z_max {
        ball.pos.y = params.z_max;
        bounce_axis(&mut ball.vel, 1, e, mu);
    }

    if ball.pos.x < params.x_min {
        if in_goal_mouth(ball.pos.y, mouth) {
            return;
        }
        ball.pos.x = params.x_min;
        bounce_axis(&mut ball.vel, 0, e, mu);
    } else if ball.pos.x > params.x_max {
        if in_goal_mouth(ball.pos.y, mouth) {
            return;
        }
        ball.pos.x = params.x_max;
        bounce_axis(&mut ball.vel, 0, e, mu);
    }
}

fn bounce_axis(vel: &mut Vec2, normal_axis: usize, e: f32, mu: f32) {
    let mut v = *vel;
    let into_speed = v[normal_axis].abs();
    if into_speed <= 1e-8 {
        return;
    }
    v[normal_axis] = -v[normal_axis].signum() * into_speed * e;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coast_stops_under_coulomb() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::ZERO,
            vel: Vec2::new(10.0, 0.0),
            held: false,
        };
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            step_free_ball(&mut ball, &params, dt);
        }
        assert!(ball.vel.length() < 0.01);
        assert!((ball.pos.x - 8.40).abs() < 0.3, "pos.x={}", ball.pos.x);
    }

    #[test]
    fn goal_mouth_is_terminal() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::new(39.0, 0.0),
            vel: Vec2::new(20.0, 0.0),
            held: false,
        };
        let mut reason = EndReason::None;
        for _ in 0..30 {
            reason = step_free_ball(&mut ball, &params, 1.0 / 60.0);
            if reason != EndReason::None {
                break;
            }
        }
        assert_eq!(reason, EndReason::GoalAway);
    }

    #[test]
    fn near_post_shot_still_scores_past_goal_line() {
        // Posts sit at x=40.2; a shot that crosses x=39.5 near the post used to
        // get shoved back by post collision before check_goal ran.
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::new(39.2, 5.5),
            vel: Vec2::new(25.0, 0.0),
            held: false,
        };
        let mut reason = EndReason::None;
        for _ in 0..40 {
            reason = step_free_ball(&mut ball, &params, 1.0 / 60.0);
            if reason != EndReason::None {
                break;
            }
        }
        assert_eq!(reason, EndReason::GoalAway, "pos={:?} vel={:?}", ball.pos, ball.vel);
    }

    #[test]
    fn held_ball_in_mouth_still_scores() {
        let params = SimParams::default();
        let ball = Ball {
            pos: Vec2::new(params.goal_line_x + 0.5, 0.0),
            vel: Vec2::ZERO,
            held: true,
        };
        assert_eq!(goal_at(ball.pos, &params), EndReason::GoalAway);
        let mut ball = ball;
        assert_eq!(step_free_ball(&mut ball, &params, 0.019), EndReason::GoalAway);
    }
}
