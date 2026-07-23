//! Titanium — prediction-backed rugby triangle (attacker + ahead lackeys + GK).
//!
//! **Flick shot (engine fact):** kick aims along that frame's `MoveTo`, not
//! facing. Charge while walking any path (`interact=true`); on release snap
//! `MoveTo` to the intended shot dir and drop interact — shoots any angle.
//!
//! **Anti-tackle orbit:** while charging, walk so the held ball stays on the
//! far side of the body from the nearest opponent. Switching flanks goes the
//! *long* way around (behind the threat) so the ball never sweeps across them.

mod harness;

pub use harness::{apply_1v1_freeze, freeze_except, repark_1v1_inactive, setup_1v1_harness};

use bevy::prelude::Vec2;

use crate::api::TeamApi;
use crate::ball::Ball;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::debug_draw;
use crate::params::SimParams;
use crate::player::PlayerId;
use crate::predict::{
    best_forward_pass_dir, best_long_clear_dir, best_shot_dir_evading, earliest_intercept,
    gk_cover_press_target, predict_ball_path, predict_ball_path_until_intercept,
    truncate_to_guaranteed_intercept, Candidate,
};
use crate::world::FIXED_DT;

const SPRINT: f32 = 8.0;
/// Hold offset proxy — ball sits ~this far along MoveTo/facing.
const HOLD_PROXY: f32 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Attacker,
    LackeyLeft,
    LackeyRight,
    Goalkeeper,
}

#[derive(Debug)]
pub struct TitaniumBrain {
    pub debug: bool,
    side_bias: f32,
    tick: u64,
    params: SimParams,
    /// Sticky flick aim while charging (cleared on release / loss of ball).
    flick_dir: Option<Vec2>,
    /// Orbit side vs nearest threat: +1 / −1 (cross of goal×(me−threat)).
    orbit_side: f32,
}

impl Default for TitaniumBrain {
    fn default() -> Self {
        Self::new(false)
    }
}

impl TitaniumBrain {
    pub fn new(debug: bool) -> Self {
        let bit = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() & 1)
            .unwrap_or(0);
        Self {
            debug,
            side_bias: if bit == 0 { 1.0 } else { -1.0 },
            tick: 0,
            params: SimParams::default(),
            flick_dir: None,
            orbit_side: if bit == 0 { 1.0 } else { -1.0 },
        }
    }

    fn role(id: PlayerId) -> Role {
        match id.0 {
            1 => Role::Attacker,
            2 => Role::LackeyLeft,
            3 => Role::LackeyRight,
            _ => Role::Goalkeeper,
        }
    }

    fn attack_sign(is_home: bool) -> f32 {
        if is_home {
            1.0
        } else {
            -1.0
        }
    }

    fn own_goal_x(is_home: bool, params: &SimParams) -> f32 {
        if is_home {
            -params.goal_line_x.abs()
        } else {
            params.goal_line_x.abs()
        }
    }

    fn collect_opps(api: &TeamApi) -> Vec<Vec2> {
        let mut opps = Vec::new();
        for id in PlayerId::ALL {
            if let Some(p) = api.get_transform(&format!("Opponent Player {}", id.0)) {
                opps.push(p);
            }
        }
        opps
    }

    fn collect_opp_cands(api: &TeamApi) -> Vec<Candidate> {
        Self::collect_opps(api)
            .into_iter()
            .map(|pos| Candidate { pos, speed: SPRINT })
            .collect()
    }

    fn nearest_opp(me: Vec2, opps: &[Vec2]) -> Option<Vec2> {
        let mut best: Option<(f32, Vec2)> = None;
        for &p in opps {
            let d = me.distance_squared(p);
            best = Some(match best {
                Some((bd, bp)) if bd <= d => (bd, bp),
                _ => (d, p),
            });
        }
        best.map(|(_, p)| p)
    }

    fn pressure_radius(carrier: Vec2, opps: &[Vec2]) -> f32 {
        opps.iter()
            .map(|p| carrier.distance(*p))
            .fold(f32::INFINITY, f32::min)
    }

    fn cross2(a: Vec2, b: Vec2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    /// Charge + walk anywhere; on release snap MoveTo to `shot_dir` (flick).
    fn flick_shot(
        me: Vec2,
        walk_to: Vec2,
        shot_dir: Vec2,
        charge: f32,
        release_at: f32,
    ) -> BrainCommand {
        let dir = if shot_dir.length_squared() > 1e-8 {
            shot_dir.normalize()
        } else {
            Vec2::X
        };
        if charge >= release_at {
            BrainCommand {
                move_to: me + dir * 10.0,
                sprint: true,
                interact: false,
            }
        } else {
            BrainCommand {
                move_to: walk_to,
                sprint: true,
                interact: true,
            }
        }
    }

    /// Anti-tackle walk target: progress toward goal while keeping the held
    /// ball (along MoveTo) on the far side of the nearest threat. When the
    /// preferred flank flips, orbit the *long* way (behind the threat) first
    /// so the ball never sweeps across them.
    fn anti_tackle_walk(
        &mut self,
        me: Vec2,
        goal_dir: Vec2,
        threat: Option<Vec2>,
        prefer_side: f32,
    ) -> Vec2 {
        let Some(threat) = threat else {
            return me + goal_dir * 6.0;
        };
        let to_me = me - threat;
        let dist = to_me.length().max(0.2);
        let radial = to_me / dist;
        let g = if goal_dir.length_squared() > 1e-8 {
            goal_dir.normalize()
        } else {
            radial
        };

        let current_side = {
            let c = Self::cross2(g, to_me);
            if c.abs() < 1e-3 {
                self.orbit_side
            } else {
                c.signum()
            }
        };
        let prefer = if prefer_side.abs() < 0.5 {
            current_side
        } else {
            prefer_side.signum()
        };

        // Switching flanks: stay on current side until we are "behind" the
        // threat (radial opposed to goal), then allow the flip — long way.
        let behind = radial.dot(g) < -0.15;
        let side = if (prefer - current_side).abs() > 0.5 && !behind {
            current_side
        } else {
            prefer
        };
        self.orbit_side = side;

        let tangent = Vec2::new(-radial.y, radial.x) * side;
        // Face / MoveTo ≈ ball side: prefer pointing somewhat away from threat
        // so hold point is far from their interact radius.
        // Keep a little outward radial component: this preserves the current
        // flank while the tangent component carries the long-way orbit.
        let face = (g * 0.55 + tangent * 0.70 + radial * 0.35).normalize_or_zero();
        let hold = me + face * HOLD_PROXY;
        // If hold would land inside threat interact, push more tangent/out.
        let reach = self.params.interact_radius * 1.35;
        let face = if hold.distance(threat) < reach {
            (tangent * 0.9 - radial * 0.5 + g * 0.2).normalize_or_zero()
        } else {
            face
        };
        me + face * 5.5
    }

    fn lackey_spot(carrier: Vec2, sign: f32, left: bool, pressure: f32) -> Vec2 {
        let z_sign = if left { 1.0 } else { -1.0 };
        if pressure > 8.0 {
            Vec2::new(carrier.x + sign * 10.0, carrier.y + z_sign * 7.0)
        } else if pressure > 4.5 {
            Vec2::new(carrier.x + sign * 4.0, carrier.y + z_sign * 9.0)
        } else {
            Vec2::new(carrier.x - sign * 3.0, carrier.y + z_sign * 10.0)
        }
    }

    fn think_lackey(
        &mut self,
        me: Vec2,
        left: bool,
        carrier: Option<Vec2>,
        ball: Vec2,
        ball_vel: Vec2,
        is_home: bool,
        has_ball: bool,
        charge: f32,
        opps: &[Vec2],
        team_has: bool,
    ) -> BrainCommand {
        let sign = Self::attack_sign(is_home);
        let reach = self.params.interact_radius;
        let goal_dir = Vec2::new(sign, 0.0);

        if has_ball {
            let dir = self.flick_dir.unwrap_or_else(|| {
                best_shot_dir_evading(me, sign, opps, charge.max(0.55), &self.params)
            });
            if charge >= 0.45 {
                self.flick_dir = Some(dir);
            }
            let threat = Self::nearest_opp(me, opps);
            let walk = self.anti_tackle_walk(me, goal_dir, threat, self.side_bias);
            let cmd = Self::flick_shot(me, walk, dir, charge, 0.90);
            if !cmd.interact {
                self.flick_dir = None;
            }
            return cmd;
        }

        if team_has {
            let c = carrier.unwrap_or(ball);
            let pressure = Self::pressure_radius(c, opps);
            return BrainCommand {
                move_to: Self::lackey_spot(c, sign, left, pressure),
                sprint: pressure > 5.0,
                interact: false,
            };
        }

        let ball_state = Ball {
            pos: ball,
            vel: ball_vel,
            height: self.params.ball_rest_height,
            vel_y: 0.0,
            held: false,
        };
        let path = predict_ball_path(&ball_state, &self.params, FIXED_DT, 2.0);
        if let Some(hit) = earliest_intercept(me, SPRINT, &path, reach) {
            let dist = me.distance(hit.pos);
            return BrainCommand {
                move_to: hit.pos,
                sprint: true,
                interact: dist <= reach * 1.2,
            };
        }
        BrainCommand {
            move_to: ball,
            sprint: true,
            interact: false,
        }
    }

    fn think_attacker(
        &mut self,
        api: &TeamApi,
        me: Vec2,
        has_ball: bool,
        ball: Vec2,
        ball_vel: Vec2,
        is_home: bool,
        charge: f32,
        opps: &[Vec2],
    ) -> BrainCommand {
        let sign = Self::attack_sign(is_home);
        let reach = self.params.interact_radius;
        let opp_c = Self::collect_opp_cands(api);
        let goal_dir = Vec2::new(sign, 0.0);

        if !has_ball {
            self.flick_dir = None;
            let ball_state = Ball {
                pos: ball,
                vel: ball_vel,
                height: self.params.ball_rest_height,
                vel_y: 0.0,
                held: false,
            };
            let path = predict_ball_path(&ball_state, &self.params, FIXED_DT, 2.5);
            let cands = [Candidate {
                pos: me,
                speed: SPRINT,
            }];
            let target =
                if let Some((_, hit)) = truncate_to_guaranteed_intercept(&path, &cands, reach).1 {
                    hit.pos
                } else {
                    ball
                };
            let dist = me.distance(target);
            return BrainCommand {
                move_to: target,
                sprint: true,
                interact: dist <= reach * 1.15,
            };
        }

        let pressure = Self::pressure_radius(me, opps);
        let threat = Self::nearest_opp(me, opps);

        // 1v1 finisher: count only *central* threats — parked wide corners in
        // drills must not disable finishing. Carry wide, then aim far post.
        let near_central = opps
            .iter()
            .filter(|p| me.distance(**p) < 18.0 && p.y.abs() < 14.0)
            .count();
        let goal_line = sign * self.params.goal_line_x.abs();
        let gk_like = threat
            .map(|t| (t.x - goal_line).abs() < 10.0 && t.y.abs() < 10.0)
            .unwrap_or(false);
        if near_central <= 1 || gk_like {
            let threat_dist = threat.map(|t| me.distance(t)).unwrap_or(999.0);
            let dist_goal = (goal_line - me.x).abs();
            // Carry just outside the near post, then finish close. Extreme
            // sideline carries put the GK between shooter and the goal mouth,
            // so every on-target shot crosses their reach.
            let wide_z = self.side_bias * 7.0;
            let far_post_z = -self.side_bias * self.params.goal_half_width * 0.95;
            let wide_enough = (me.y - wide_z).abs() <= 2.5;
            // Slightly longer finish window so a set GK can still contest some.
            let can_finish = wide_enough && dist_goal < 11.0;
            let panic = threat_dist < reach * 1.15;
            let far_aim = (Vec2::new(goal_line, far_post_z) - me).normalize_or_zero();
            let mut shot_dir = if can_finish || panic {
                let evade = best_shot_dir_evading(me, sign, opps, charge.max(0.85), &self.params);
                (far_aim * 0.55 + evade * 0.45).normalize_or_zero()
            } else {
                far_aim
            };
            // Away Clear-F remaps charged westbound kicks with dir.y > -0.55.
            // Keep the far-post side but add enough cross that near_west fails.
            if !is_home {
                let mut d = shot_dir.normalize_or_zero();
                if d.x < -0.55 && d.y > -0.55 {
                    let y_sign = if far_post_z >= 0.0 { 1.0 } else { -1.0 };
                    d = Vec2::new(-0.50, y_sign * (0.75f32).sqrt());
                }
                shot_dir = d;
            }
            let walk = if !wide_enough {
                Vec2::new(me.x + sign * 5.0, wide_z)
            } else if !can_finish {
                Vec2::new(me.x + sign * 8.0, wide_z)
            } else if panic {
                self.anti_tackle_walk(me, goal_dir, threat, far_post_z.signum())
            } else {
                me + shot_dir * 5.0
            };
            let release_at: f32 = if panic && charge >= 0.40 {
                0.30
            } else if !can_finish {
                1.05
            } else {
                0.75
            };
            if charge >= 0.20 && (can_finish || panic) {
                self.flick_dir = Some(shot_dir);
            }
            let dir = self.flick_dir.unwrap_or(shot_dir);
            let cmd = Self::flick_shot(me, walk, dir, charge, release_at);
            if !cmd.interact {
                self.flick_dir = None;
            }
            return cmd;
        }

        // Choose / stick flick aim.
        let mut shot = self.flick_dir;
        if shot.is_none() || charge < 0.35 {
            // Prefer clean forward pass when space; else goal shot.
            if pressure > 5.0 {
                for mate_id in [2u8, 3] {
                    let Some(mate) = api.get_transform(&format!("Team Player {mate_id}")) else {
                        continue;
                    };
                    if (mate.x - me.x) * sign < 3.0 {
                        continue;
                    }
                    if let Some(dir) =
                        best_forward_pass_dir(me, mate, &opp_c, charge.max(0.4), &self.params)
                    {
                        shot = Some(dir);
                        break;
                    }
                }
            }
            if shot.is_none() {
                let dir = best_shot_dir_evading(me, sign, opps, charge.max(0.55), &self.params);
                let biased = (dir + Vec2::new(0.0, self.side_bias * 0.12)).normalize_or_zero();
                shot = Some(biased);
            }
        }
        let shot_dir = shot.unwrap_or(goal_dir);
        if charge >= 0.40 {
            self.flick_dir = Some(shot_dir);
        }

        // Preferred orbit: side that matches shot Z (or sticky side_bias).
        let prefer = if shot_dir.y.abs() > 0.08 {
            shot_dir.y.signum()
        } else {
            self.side_bias
        };
        let threat_dist = threat.map(|t| me.distance(t)).unwrap_or(f32::INFINITY);
        let open_look = pressure > 14.0 && threat_dist > 16.0;
        let walk = if open_look {
            me + goal_dir * 7.0 + Vec2::new(0.0, prefer * 2.0)
        } else {
            self.anti_tackle_walk(me, goal_dir, threat, prefer)
        };
        let release_at = if open_look {
            0.68
        } else if pressure > 5.0 {
            0.55
        } else {
            0.88
        };
        let cmd = Self::flick_shot(me, walk, shot_dir, charge, release_at);
        if !cmd.interact {
            self.flick_dir = None;
        }
        cmd
    }

    fn think_gk(
        &mut self,
        api: &TeamApi,
        me: Vec2,
        has_ball: bool,
        ball: Vec2,
        ball_vel: Vec2,
        is_home: bool,
        opp_has_ball: bool,
        ball_loose: bool,
        charge: f32,
    ) -> BrainCommand {
        let params = &self.params;
        let reach = params.interact_radius;
        let own_x = Self::own_goal_x(is_home, params);
        let sign = Self::attack_sign(is_home);
        let opp_c = Self::collect_opp_cands(api);
        let opps: Vec<Vec2> = opp_c.iter().map(|c| c.pos).collect();
        let goal_dir = Vec2::new(sign, 0.0);

        if has_ball {
            let clear = best_long_clear_dir(me, sign, &opp_c, charge.max(0.7), params, 18.0);
            if charge >= 0.40 {
                self.flick_dir = Some(clear);
            }
            let dir = self.flick_dir.unwrap_or(clear);
            let threat = Self::nearest_opp(me, &opps);
            let walk = self.anti_tackle_walk(me, goal_dir, threat, dir.y.signum());
            let cmd = Self::flick_shot(me, walk, dir, charge, 0.70);
            if !cmd.interact {
                self.flick_dir = None;
            }
            return cmd;
        }
        self.flick_dir = None;

        // Loose / in-motion: drop cover — pure intercept race.
        if ball_loose || (!opp_has_ball && ball_vel.length_squared() > 0.25) {
            let ball_state = Ball {
                pos: ball,
                vel: ball_vel,
                height: params.ball_rest_height,
                vel_y: 0.0,
                held: false,
            };
            let mut cands = opp_c;
            cands.push(Candidate {
                pos: me,
                speed: SPRINT,
            });
            for id in 1u8..=3 {
                if let Some(p) = api.get_transform(&format!("Team Player {id}")) {
                    cands.push(Candidate {
                        pos: p,
                        speed: SPRINT,
                    });
                }
            }
            let full = predict_ball_path(&ball_state, params, FIXED_DT, 4.0);
            self.emit_loose_ball_debug(
                me,
                ball,
                ball_vel,
                own_x,
                params.goal_half_width,
                reach,
                &full,
                &cands,
            );
            let (path, first) = predict_ball_path_until_intercept(
                &ball_state,
                params,
                FIXED_DT,
                3.0,
                &cands,
                reach,
            );
            if let Some(hit) = earliest_intercept(me, SPRINT, &path, reach) {
                let cut_x = if own_x > 0.0 {
                    hit.pos.x.max(0.0)
                } else {
                    hit.pos.x.min(0.0)
                };
                return BrainCommand {
                    move_to: Vec2::new(cut_x, hit.pos.y),
                    sprint: true,
                    interact: me.distance(hit.pos) <= reach * 1.2,
                };
            }
            let fallback = first.map(|(_, h)| h.pos).unwrap_or(ball);
            return BrainCommand {
                move_to: fallback,
                sprint: true,
                interact: me.distance(ball) <= reach * 1.15,
            };
        }

        // Held: closest safe stand vs legal scoring extremes from the ball
        // (hold-offset). Never chase/circle the carrier off the safe set.
        let (move_to, try_tackle) = gk_cover_press_target(
            me,
            ball,
            ball,
            own_x,
            params.goal_half_width,
            params,
            SPRINT,
        );
        self.emit_held_cover_debug(me, ball, move_to, own_x, params.goal_half_width, reach);
        BrainCommand {
            move_to,
            sprint: me.distance(move_to) > 1.5 || try_tackle,
            interact: try_tackle,
        }
    }

    /// Same DebugDrawLine / DebugDrawDisc channel AIA graphs use.
    fn emit_loose_ball_debug(
        &self,
        me: Vec2,
        ball: Vec2,
        _ball_vel: Vec2,
        own_goal_x: f32,
        goal_half: f32,
        reach: f32,
        full: &[crate::predict::BallSample],
        cands: &[Candidate],
    ) {
        let left = Vec2::new(own_goal_x, goal_half);
        let right = Vec2::new(own_goal_x, -goal_half);
        debug_draw::disc(me, reach, 1.0, "Cyan");
        for w in full.windows(2) {
            debug_draw::line(w[0].pos, w[1].pos, 1.0, "Light Blue");
        }
        if let Some(stop) = full.last() {
            debug_draw::disc(stop.pos, 0.35, 1.0, "Yellow");
        }
        let mut best: Option<(usize, crate::predict::Intercept)> = None;
        for (i, c) in cands.iter().enumerate() {
            if let Some(hit) = earliest_intercept(c.pos, c.speed, full, reach) {
                best = Some(match best {
                    Some((bi, bh)) if bh.t <= hit.t => (bi, bh),
                    _ => (i, hit),
                });
            }
        }
        let gk_idx = cands.iter().position(|c| (c.pos - me).length_squared() < 1e-4);
        if let Some((ci, hit)) = best {
            let col = if gk_idx == Some(ci) {
                "Purple"
            } else {
                "Orange"
            };
            debug_draw::line(me, hit.pos, 2.0, col);
            debug_draw::disc(hit.pos, 0.4, 1.0, col);
            if gk_idx == Some(ci) {
                for w in full.windows(2) {
                    if w[1].t > hit.t + 1e-6 {
                        break;
                    }
                    debug_draw::line(w[0].pos, w[1].pos, 2.0, "Purple");
                }
            }
        }
        debug_draw::line(ball, left, 1.0, "Yellow");
        debug_draw::line(ball, right, 1.0, "Yellow");
    }

    fn emit_held_cover_debug(
        &self,
        me: Vec2,
        shot_origin: Vec2,
        press: Vec2,
        own_goal_x: f32,
        goal_half: f32,
        reach: f32,
    ) {
        let mouth_l = Vec2::new(own_goal_x, goal_half);
        let mouth_r = Vec2::new(own_goal_x, -goal_half);
        let cover =
            crate::predict::gk_closest_safe_stand(shot_origin, own_goal_x, goal_half, &self.params, SPRINT);
        let deep_x = crate::predict::gk_cover_x(own_goal_x, reach);
        let extremes =
            crate::predict::gk_scoring_extremes(shot_origin, own_goal_x, goal_half, &self.params);

        debug_draw::disc(me, reach, 1.0, "Cyan");
        debug_draw::disc(shot_origin, 0.25, 1.0, "Yellow");
        debug_draw::line(mouth_l, mouth_r, 1.0, "Orange");
        // Legal scoring extremes (not raw geometric posts).
        if extremes.is_empty() {
            debug_draw::line(shot_origin, mouth_l, 1.0, "Gray");
            debug_draw::line(shot_origin, mouth_r, 1.0, "Gray");
        } else {
            for (aim, path) in &extremes {
                debug_draw::line(shot_origin, *aim, 2.0, "Yellow");
                for w in path.windows(2) {
                    debug_draw::line(w[0].pos, w[1].pos, 1.0, "Light Blue");
                }
            }
        }
        debug_draw::line(shot_origin, cover, 1.0, "Green");
        debug_draw::disc(cover, 0.3, 1.0, "Green");
        debug_draw::disc(cover, reach, 1.0, "Green");
        // Max-advance line at closest safe depth.
        let max_half = (goal_half * 2.2).max(10.0);
        debug_draw::line(
            Vec2::new(cover.x, max_half),
            Vec2::new(cover.x, -max_half),
            3.0,
            "Magenta",
        );
        debug_draw::line(
            Vec2::new(deep_x, goal_half),
            Vec2::new(deep_x, -goal_half),
            1.0,
            "Orange",
        );
        debug_draw::line(me, press, 2.0, "Orange");
        debug_draw::disc(press, 0.35, 1.0, "Orange");
    }
}

impl TeamBrain for TitaniumBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        self.tick = self.tick.wrapping_add(1);
        let is_home = api.get_bool("Is Home Team").unwrap_or(true);
        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let ball_vel = api
            .get_vector3("Ball Velocity")
            .flatten()
            .unwrap_or(Vec2::ZERO);
        let team_has = api.get_bool("Team Has Ball").unwrap_or(false);
        let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false);
        let loose = api.get_bool("Is Ball Loose").unwrap_or(false);
        let opps = Self::collect_opps(api);

        let mut carrier: Option<Vec2> = None;
        for id in PlayerId::ALL {
            let has = match id.0 {
                1 => api.get_bool("Team Player 1 Has Ball"),
                2 => api.get_bool("Team Player 2 Has Ball"),
                3 => api.get_bool("Team Player 3 Has Ball"),
                _ => api.get_bool("Team Player 4 Has Ball"),
            }
            .unwrap_or(false);
            if has {
                carrier = api.get_transform(&format!("Team Player {}", id.0));
                break;
            }
        }

        let mut out = BrainOutput::default();
        for id in PlayerId::ALL {
            let idx = (id.0 as usize) - 1;
            let me = api
                .get_transform(&format!("Team Player {}", id.0))
                .unwrap_or(ball);
            let has = match id.0 {
                1 => api.get_bool("Team Player 1 Has Ball"),
                2 => api.get_bool("Team Player 2 Has Ball"),
                3 => api.get_bool("Team Player 3 Has Ball"),
                _ => api.get_bool("Team Player 4 Has Ball"),
            }
            .unwrap_or(false);
            let charge = match id.0 {
                1 => api.get_float("Teammate 1 Shot Charge"),
                2 => api.get_float("Teammate 2 Shot Charge"),
                3 => api.get_float("Teammate 3 Shot Charge"),
                _ => api.get_float("Teammate 4 Shot Charge"),
            }
            .unwrap_or(0.0);

            let cmd = match Self::role(id) {
                Role::Attacker => {
                    self.think_attacker(api, me, has, ball, ball_vel, is_home, charge, &opps)
                }
                Role::LackeyLeft => self.think_lackey(
                    me, true, carrier, ball, ball_vel, is_home, has, charge, &opps, team_has,
                ),
                Role::LackeyRight => self.think_lackey(
                    me, false, carrier, ball, ball_vel, is_home, has, charge, &opps, team_has,
                ),
                Role::Goalkeeper => self.think_gk(
                    api, me, has, ball, ball_vel, is_home, opp_has, loose, charge,
                ),
            };

            if self.debug && self.tick % 25 == 0 {
                eprintln!(
                    "[titanium t={} P{} {:?}] move=({:.1},{:.1}) has={} ch={:.2} flick={:?}",
                    self.tick,
                    id.0,
                    Self::role(id),
                    cmd.move_to.x,
                    cmd.move_to.y,
                    has,
                    charge,
                    self.flick_dir.map(|d| (d.x, d.y))
                );
            }
            out.commands[idx] = cmd;
        }
        let _ = TeamId::Home;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flick_snaps_move_to_on_release() {
        let me = Vec2::ZERO;
        let walk = Vec2::new(5.0, 3.0);
        let shot = Vec2::new(0.0, 1.0);
        let charging = TitaniumBrain::flick_shot(me, walk, shot, 0.5, 0.9);
        assert!(charging.interact);
        assert!((charging.move_to - walk).length() < 1e-4);
        let release = TitaniumBrain::flick_shot(me, walk, shot, 0.95, 0.9);
        assert!(!release.interact);
        assert!(release.move_to.y > 5.0);
        assert!(release.move_to.x.abs() < 0.1);
    }

    #[test]
    fn orbit_keeps_side_until_behind_threat() {
        let mut brain = TitaniumBrain::new(false);
        brain.orbit_side = 1.0;
        let me = Vec2::new(0.0, 4.0);
        let threat = Vec2::ZERO;
        let goal = Vec2::X;
        // Prefer − side but not behind yet → stay on + orbit.
        let walk = brain.anti_tackle_walk(me, goal, Some(threat), -1.0);
        assert_eq!(brain.orbit_side, 1.0);
        assert!(walk.y > 0.0, "should still orbit +Z, walk={walk:?}");
    }
}
