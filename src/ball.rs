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
    if ball.held || dt <= 0.0 {
        return EndReason::None;
    }

    let speed = ball.vel.length();
    if speed <= params.stop_speed_eps {
        ball.vel = Vec2::ZERO;
        return check_goal(ball.pos, params);
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
    check_goal(ball.pos, params)
}

fn check_goal(pos: Vec2, params: &SimParams) -> EndReason {
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
}
