//! Cheap fixed-step ball prediction + earliest intercept.
//! Horizon truncates when someone is guaranteed to reach the ball.
//! Sim Vec2 = Unity (X,Z): X=goals, Y=sidelines.

use bevy::prelude::Vec2;

use crate::ball::{goal_at, step_free_ball, Ball, EndReason};
use crate::params::SimParams;

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
    predict_ball_path_until_intercept(ball, params, dt, max_horizon_s, &[], 0.0).0
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::FIXED_DT;

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
