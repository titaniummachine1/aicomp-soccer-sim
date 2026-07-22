//! Free-ball XZ kinematics + hidden Y (lift / hang).
//!
//! Ground Coulomb slide (`slide_accel`) applies only while grounded. Airborne
//! kicks keep planar speed until landing — stronger charge ⇒ more lift ⇒ longer
//! hang ⇒ farther carry than always-on slide.

use bevy::prelude::*;

use crate::params::SimParams;

#[derive(Component, Debug, Clone, Copy)]
pub struct Ball {
    /// Pitch plane (sim X = Unity depth, sim Y = Unity width/Z).
    pub pos: Vec2,
    pub vel: Vec2,
    /// Hidden height (Unity Y). Not drawn; drives airborne vs ground slide.
    pub height: f32,
    pub vel_y: f32,
    pub held: bool,
}

impl Default for Ball {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            height: 0.33,
            vel_y: 0.0,
            held: false,
        }
    }
}

impl Ball {
    pub fn grounded(&self, params: &SimParams) -> bool {
        self.height <= params.ball_rest_height + 1e-4 && self.vel_y <= 0.05
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    None,
    GoalHome,
    GoalAway,
}

/// TimePlot 2026-07-22 charge sweep: planar launch + lift.
/// `horiz = min((10 + 290 c)/9, horiz_cap)` then soft-cap `|v|` to `kick_max_speed`.
pub fn kick_launch_speeds(charge: f32, params: &SimParams) -> (f32, f32) {
    let c = charge.clamp(0.0, 1.0);
    let mut horiz = (params.kick_speed_base + params.kick_speed_per_charge * c)
        .min(params.kick_horiz_cap);
    let mut lift = params.kick_lift_base + params.kick_lift_per_charge * c;
    if lift < 0.0 {
        lift = 0.0;
    }
    let spd = (horiz * horiz + lift * lift).sqrt();
    if spd > params.kick_max_speed && spd > 1e-6 {
        let s = params.kick_max_speed / spd;
        horiz *= s;
        lift *= s;
    }
    (horiz, lift)
}

pub fn in_goal_mouth(z: f32, half_width: f32) -> bool {
    z.abs() <= half_width
}

pub fn step_free_ball(ball: &mut Ball, params: &SimParams, dt: f32) -> EndReason {
    if ball.held || dt <= 0.0 {
        return goal_at(ball.pos, params);
    }

    // --- Vertical (hidden) ---
    ball.height += ball.vel_y * dt - 0.5 * params.gravity * dt * dt;
    ball.vel_y -= params.gravity * dt;
    if ball.height <= params.ball_rest_height {
        ball.height = params.ball_rest_height;
        if ball.vel_y < 0.0 {
            let bounce = -ball.vel_y * params.ball_bounce_e;
            ball.vel_y = if bounce < params.ball_bounce_settle {
                0.0
            } else {
                bounce
            };
        }
    }

    let grounded = ball.grounded(params);

    // --- Planar ---
    let speed = ball.vel.length();
    if speed <= params.stop_speed_eps {
        ball.vel = Vec2::ZERO;
        resolve_walls(ball, params);
        let scored = goal_at(ball.pos, params);
        if scored != EndReason::None {
            return scored;
        }
        resolve_posts(ball, params);
        return goal_at(ball.pos, params);
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
        // Airborne: no ground Coulomb slide (Y is why strong kicks carry farther).
        ball.pos += ball.vel * dt;
    }

    resolve_walls(ball, params);

    let scored = goal_at(ball.pos, params);
    if scored != EndReason::None {
        return scored;
    }

    resolve_posts(ball, params);
    goal_at(ball.pos, params)
}

/// Loose-ball circle vs player bodies. Hot / airborne balls pass through.
pub fn resolve_player_bodies(ball: &mut Ball, players: &[crate::player::Player], params: &SimParams) {
    if ball.held {
        return;
    }
    if !ball.grounded(params) || ball.vel.length() > 8.0 {
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
    if goal_at(ball.pos, params) != EndReason::None {
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
        bounce_circle(&mut ball.vel, n, e, mu);
    }
}

fn resolve_walls(ball: &mut Ball, params: &SimParams) {
    // `x_min`/`z_*` are already the playable *ball-center* AABB — do not
    // inset by radius again.
    let e = params.wall_e;
    let mu = params.wall_mu;

    if ball.pos.y < params.z_min {
        ball.pos.y = params.z_min;
        if ball.vel.y < 0.0 {
            bounce_axis(&mut ball.vel, 1, e, mu);
        }
    } else if ball.pos.y > params.z_max {
        ball.pos.y = params.z_max;
        if ball.vel.y > 0.0 {
            bounce_axis(&mut ball.vel, 1, e, mu);
        }
    }

    // Open goal mouths — no wall when |z| <= goal_half_width.
    if ball.pos.x < params.x_min {
        if !in_goal_mouth(ball.pos.y, params.goal_half_width) {
            ball.pos.x = params.x_min;
            if ball.vel.x < 0.0 {
                bounce_axis(&mut ball.vel, 0, e, mu);
            }
        }
    } else if ball.pos.x > params.x_max {
        if !in_goal_mouth(ball.pos.y, params.goal_half_width) {
            ball.pos.x = params.x_max;
            if ball.vel.x > 0.0 {
                bounce_axis(&mut ball.vel, 0, e, mu);
            }
        }
    }
}

fn bounce_circle(vel: &mut Vec2, n: Vec2, e: f32, mu: f32) {
    let vn = vel.dot(n);
    if vn >= 0.0 {
        return;
    }
    let into = -vn;
    let mut t = *vel - n * vn;
    let t_speed = t.length();
    let friction_budget = mu * (1.0 + e) * into;
    if t_speed > 0.0 {
        let kill = friction_budget.min(t_speed);
        t *= (t_speed - kill) / t_speed;
    }
    *vel = t + n * (into * e);
}

fn bounce_axis(vel: &mut Vec2, normal_axis: usize, e: f32, mu: f32) {
    let mut v = *vel;
    let into_speed = if normal_axis == 0 {
        if v.x > 0.0 {
            v.x
        } else {
            -v.x
        }
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

/// True when ball center is past a goal line inside the mouth (held or free).
pub fn goal_at(pos: Vec2, params: &SimParams) -> EndReason {
    if pos.x >= params.goal_line_x && in_goal_mouth(pos.y, params.goal_half_width) {
        return EndReason::GoalAway;
    }
    if pos.x <= -params.goal_line_x && in_goal_mouth(pos.y, params.goal_half_width) {
        return EndReason::GoalHome;
    }
    EndReason::None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_ball(vel: Vec2) -> Ball {
        Ball {
            pos: Vec2::ZERO,
            vel,
            height: 0.33,
            vel_y: 0.0,
            held: false,
        }
    }

    #[test]
    fn kick_curve_matches_timeplot_grid() {
        let p = SimParams::default();
        let (h05, _) = kick_launch_speeds(0.05, &p);
        let (h50, _) = kick_launch_speeds(0.50, &p);
        let (h85, _) = kick_launch_speeds(0.85, &p);
        let (h95, l95) = kick_launch_speeds(0.95, &p);
        assert!((h05 - 2.722222).abs() < 1e-3, "h05={h05}");
        assert!((h50 - 17.222222).abs() < 1e-3, "h50={h50}");
        assert!((h85 - 28.5).abs() < 1e-3, "h85={h85}");
        assert!(h95 <= 29.42 + 0.05, "h95={h95}");
        let spd = (h95 * h95 + l95 * l95).sqrt();
        assert!(spd <= p.kick_max_speed + 1e-3, "spd={spd}");
    }

    #[test]
    fn coast_stops_under_coulomb() {
        let params = SimParams::default();
        let mut ball = ground_ball(Vec2::new(10.0, 0.0));
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            step_free_ball(&mut ball, &params, dt);
        }
        assert!(ball.vel.length() < 0.01);
        assert!((ball.pos.x - 8.40).abs() < 0.3, "pos.x={}", ball.pos.x);
    }

    #[test]
    fn airborne_skips_ground_slide() {
        let params = SimParams::default();
        let mut air = ground_ball(Vec2::new(20.0, 0.0));
        air.height = 0.33;
        air.vel_y = 5.55; // full-kick lift class
        let mut ground = ground_ball(Vec2::new(20.0, 0.0));
        let dt = 0.019;
        for _ in 0..30 {
            step_free_ball(&mut air, &params, dt);
            step_free_ball(&mut ground, &params, dt);
        }
        assert!(
            air.pos.x > ground.pos.x + 0.5,
            "air={} ground={} (air should carry farther)",
            air.pos.x,
            ground.pos.x
        );
    }

    #[test]
    fn goal_mouth_is_terminal() {
        let params = SimParams::default();
        let mut ball = ground_ball(Vec2::new(20.0, 0.0));
        ball.pos = Vec2::new(39.0, 0.0);
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
        let params = SimParams::default();
        let mut ball = ground_ball(Vec2::new(25.0, 0.0));
        ball.pos = Vec2::new(39.2, 5.5);
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
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: true,
        };
        assert_eq!(goal_at(ball.pos, &params), EndReason::GoalAway);
        let mut ball = ball;
        assert_eq!(step_free_ball(&mut ball, &params, 0.019), EndReason::GoalAway);
    }
}
