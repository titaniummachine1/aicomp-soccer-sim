//! Shared Titanium Scenario 1 harness setup (viewer + `titanium_drill`).
//!
//! Scenario 1 is a visible 1v1: only attacker P1 and goalkeeper P4 remain on
//! the pitch; the other six roster players stay off-pitch and frozen.

use bevy::prelude::Vec2;

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::match_state::{place_kickoff, MatchPhase};
use crate::params::SimParams;
use crate::player::PlayerId;
use crate::possession::reset_possession_for_kickoff;
use crate::world::MatchWorld;

fn park_off_pitch(team: TeamId, id: u8, params: &SimParams) -> Vec2 {
    let back = match team {
        TeamId::Home => -1.0,
        TeamId::Away => 1.0,
    };
    // Park extras at opposite ends and wide on the sidelines, well beyond the
    // playable AABB (Home deep −X, Away deep +X). Offsets keep spherecasts
    // from stacking the full roster.
    let x = back * (params.x_max.abs() + 20.0 + id as f32 * 3.0);
    let z = back * (60.0 + id as f32 * 4.0);
    Vec2::new(x, z)
}

/// Freeze every teammate except `active` (P1 attacker / P4 GK) in Scenario 1.
pub fn freeze_except(out: &mut BrainOutput, world: &MatchWorld, team: TeamId, active: u8) {
    freeze_except_many(out, world, team, &[active]);
}

/// Freeze every teammate whose id is not in `active`.
pub fn freeze_except_many(out: &mut BrainOutput, world: &MatchWorld, team: TeamId, active: &[u8]) {
    for p in &world.players {
        if p.team != team {
            continue;
        }
        let idx = (p.id.0 as usize).saturating_sub(1);
        if idx >= 4 || active.contains(&p.id.0) {
            continue;
        }
        out.commands[idx] = BrainCommand {
            move_to: p.pos,
            sprint: false,
            interact: false,
        };
    }
}

/// Scenario 1 layout only: attacker P1 at center with ball; GK P4 on the goal
/// line center. No custom tackle rules — possession uses the global duel.
/// Other roster players are parked off-pitch.
pub fn setup_1v1_harness(world: &mut MatchWorld, attack_home: bool, z_bias: f32) {
    let params = world.params.clone();
    let (atk, gk_team) = if attack_home {
        (TeamId::Home, TeamId::Away)
    } else {
        (TeamId::Away, TeamId::Home)
    };
    let sign = if attack_home { 1.0 } else { -1.0 };
    let own_goal_x = if attack_home {
        params.goal_line_x.abs()
    } else {
        -params.goal_line_x.abs()
    };

    let atk_pos = Vec2::new(0.0, z_bias * 3.0);
    // Middle of the goal mouth, slightly off the line into the pitch.
    let gk_pos = Vec2::new(own_goal_x - sign * 1.5, 0.0);

    world.match_state.phase = MatchPhase::Play;
    world.match_state.phase_timer = 0.0;
    world.match_state.kickoff_circle_lock = false;
    world.match_state.kickoff_suppress_away_team_side = false;
    world.match_state.clock_s = 0.0;
    // Scores are intentionally preserved — Scenario 1 is a continuous bout;
    // callers zero scores only on a full restart / new drill trial.

    for p in &mut world.players {
        if p.team == atk && p.id == PlayerId(1) {
            p.pos = atk_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(sign, 0.0);
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        } else if p.team == gk_team && p.id == PlayerId(4) {
            p.pos = gk_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(-sign, 0.0);
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        } else {
            p.pos = park_off_pitch(p.team, p.id.0, &params);
            p.vel = Vec2::ZERO;
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        }
    }

    world.ball.pos = atk_pos + Vec2::new(sign * params.hold_offset, 0.0);
    world.ball.vel = Vec2::ZERO;
    world.ball.vel_y = 0.0;
    world.ball.height = params.ball_rest_height;
    world.ball.held = true;
    world.possession.carrier = Some((atk, 1));
    world.possession.pickup_lockout = 0.0;
    world.possession.kick_exclude_shooter = None;
    world.possession.kick_exclude_left = 0.0;
    world.possession.first_kick_done = true;
    world.possession.kickoff_touch_done = true;
    world.possession.opening_dump_hang = false;
    world.possession.opening_hot_reclaim = false;
    world.match_state.reset_stale_tracker(world.ball.pos);
}

/// Re-park inactive Scenario 1 players after physics (goal-mouth clamp can pull
/// off-pitch parks back onto the AABB edges). Harness-only; keeps extras away
/// from the ball.
pub fn repark_1v1_inactive(world: &mut MatchWorld, attack_home: bool) {
    let (atk, gk_team) = if attack_home {
        (TeamId::Home, TeamId::Away)
    } else {
        (TeamId::Away, TeamId::Home)
    };
    let params = world.params.clone();
    for p in &mut world.players {
        if (p.team == atk && p.id == PlayerId(1)) || (p.team == gk_team && p.id == PlayerId(4)) {
            continue;
        }
        p.pos = park_off_pitch(p.team, p.id.0, &params);
        p.vel = Vec2::ZERO;
    }
}

/// After brains think: keep only attacker P1 / GK P4 live.
pub fn apply_1v1_freeze(
    home_out: &mut BrainOutput,
    away_out: &mut BrainOutput,
    world: &MatchWorld,
    attack_home: bool,
) {
    if attack_home {
        freeze_except(home_out, world, TeamId::Home, 1);
        freeze_except(away_out, world, TeamId::Away, 4);
    } else {
        freeze_except(home_out, world, TeamId::Home, 4);
        freeze_except(away_out, world, TeamId::Away, 1);
    }
}

/// Which side of the goal the Scenario 2 receiver (P2) is scripted to shoot at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotExtreme {
    Left,
    Right,
}

fn safe_dir(v: Vec2) -> Vec2 {
    if v.length_squared() > 1e-6 {
        v.normalize()
    } else {
        Vec2::X
    }
}

/// Scripted attacker pair for Scenario 2 (2v1 passing drill): P1 starts with
/// the ball and charges to full power. P2 runs continuously from its start
/// spot toward the opposite flank (a real "making a run" pattern, not a
/// stationary target) — P1 predicts that motion and leads the pass to where
/// P2 *will be* when the ball arrives, not where P2 is right now. Once P2
/// claims the pass it stops running, charges to full, and shoots at the
/// requested post extreme. Fully deterministic — no decision-making, purely
/// a fixed timeline to test the GK against (charge time + lead-pass travel
/// time + reception + second charge + shot).
pub struct PassDrillAttackerBrain {
    pub extreme: ShotExtreme,
    pub receiver_pos: Vec2,
    pub receiver_run_target: Vec2,
    pub receiver_run_speed: f32,
    /// Approximate effective ball speed for the pass, used only to solve the
    /// lead-intercept point — doesn't need to match the real kick physics
    /// exactly, just be in the right ballpark for a realistic lead.
    pub pass_speed: f32,
    pub goal_half_width: f32,
}

/// Where a runner moving at `speed` toward `target` from `start` will be when
/// a shot fired from `origin` at `shot_speed` reaches them — a standard
/// linear pursuit-curve intercept, solved by fixed-point iteration (a few
/// iterations converge easily since the correction shrinks each pass).
fn lead_intercept(origin: Vec2, start: Vec2, target: Vec2, speed: f32, shot_speed: f32) -> Vec2 {
    let run_dir = safe_dir(target - start);
    let mut t = origin.distance(start) / shot_speed.max(0.1);
    for _ in 0..6 {
        let predicted = start + run_dir * (speed * t);
        t = origin.distance(predicted) / shot_speed.max(0.1);
    }
    start + run_dir * (speed * t)
}

impl crate::brain::TeamBrain for PassDrillAttackerBrain {
    fn think(&mut self, api: &crate::api::TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();

        let p1 = api.get_transform("Team Player 1").unwrap_or(Vec2::ZERO);
        let h1 = api.get_bool("Team Player 1 Has Ball").unwrap_or(false);
        let c1 = api.get_float("Teammate 1 Shot Charge").unwrap_or(0.0);
        let p2 = api.get_transform("Team Player 2").unwrap_or(Vec2::ZERO);
        out.commands[0] = if h1 {
            // Lead P2's run toward the opposite flank rather than aiming at
            // its current spot — P2 is a moving target by the time the pass
            // actually arrives.
            let lead = lead_intercept(
                p1,
                p2,
                self.receiver_run_target,
                self.receiver_run_speed,
                self.pass_speed,
            );
            let aim = safe_dir(lead - p1);
            if c1 >= 0.97 {
                BrainCommand {
                    move_to: p1 + aim * 10.0,
                    sprint: true,
                    interact: false,
                }
            } else {
                BrainCommand {
                    move_to: p1,
                    sprint: false,
                    interact: true,
                }
            }
        } else {
            BrainCommand {
                move_to: p1,
                sprint: false,
                interact: false,
            }
        };

        let h2 = api.get_bool("Team Player 2 Has Ball").unwrap_or(false);
        let c2 = api.get_float("Teammate 2 Shot Charge").unwrap_or(0.0);
        let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(Vec2::ZERO);
        let target_z = match self.extreme {
            ShotExtreme::Left => self.goal_half_width * 0.8,
            ShotExtreme::Right => -self.goal_half_width * 0.8,
        };
        let target = Vec2::new(opp_goal.x, target_z);
        out.commands[1] = if h2 {
            let aim = safe_dir(target - p2);
            if c2 >= 0.97 {
                BrainCommand {
                    move_to: p2 + aim * 10.0,
                    sprint: true,
                    interact: false,
                }
            } else {
                BrainCommand {
                    move_to: p2,
                    sprint: false,
                    interact: true,
                }
            }
        } else {
            // Making a run toward the opposite flank — a real moving
            // target, not standing still waiting for the ball.
            BrainCommand {
                move_to: self.receiver_run_target,
                sprint: true,
                interact: true,
            }
        };

        out
    }
}

/// Scenario 2 layout: attacker P1 at center with the ball, attacker P2 at a
/// wide receiving spot, GK P4 on cover. Everyone else parked off-pitch.
pub fn setup_2v1_harness(
    world: &mut MatchWorld,
    attack_home: bool,
    z_bias: f32,
    receiver_wide_z: f32,
    receiver_forward: f32,
) -> Vec2 {
    let params = world.params.clone();
    let (atk, gk_team) = if attack_home {
        (TeamId::Home, TeamId::Away)
    } else {
        (TeamId::Away, TeamId::Home)
    };
    let sign = if attack_home { 1.0 } else { -1.0 };
    let own_goal_x = if attack_home {
        params.goal_line_x.abs()
    } else {
        -params.goal_line_x.abs()
    };

    let atk_pos = Vec2::new(0.0, z_bias * 3.0);
    let receiver_pos = Vec2::new(atk_pos.x + sign * receiver_forward, receiver_wide_z);
    let gk_pos = Vec2::new(own_goal_x - sign * 1.5, 0.0);

    world.match_state.phase = MatchPhase::Play;
    world.match_state.phase_timer = 0.0;
    world.match_state.kickoff_circle_lock = false;
    world.match_state.kickoff_suppress_away_team_side = false;
    world.match_state.clock_s = 0.0;

    for p in &mut world.players {
        if p.team == atk && p.id == PlayerId(1) {
            p.pos = atk_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(sign, 0.0);
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        } else if p.team == atk && p.id == PlayerId(2) {
            p.pos = receiver_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(sign, 0.0);
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        } else if p.team == gk_team && p.id == PlayerId(4) {
            p.pos = gk_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(-sign, 0.0);
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        } else {
            p.pos = park_off_pitch(p.team, p.id.0, &params);
            p.vel = Vec2::ZERO;
            p.shot_charge = 0.0;
            p.charge_warmup_left = 0.0;
            p.stamina = 1.0;
        }
    }

    world.ball.pos = atk_pos + Vec2::new(sign * params.hold_offset, 0.0);
    world.ball.vel = Vec2::ZERO;
    world.ball.vel_y = 0.0;
    world.ball.height = params.ball_rest_height;
    world.ball.held = true;
    world.possession.carrier = Some((atk, 1));
    world.possession.pickup_lockout = 0.0;
    world.possession.kick_exclude_shooter = None;
    world.possession.kick_exclude_left = 0.0;
    world.possession.first_kick_done = true;
    world.possession.kickoff_touch_done = true;
    world.possession.opening_dump_hang = false;
    world.possession.opening_hot_reclaim = false;
    world.match_state.reset_stale_tracker(world.ball.pos);
    receiver_pos
}

/// Re-park inactive Scenario 2 players after physics.
pub fn repark_2v1_inactive(world: &mut MatchWorld, attack_home: bool) {
    let (atk, gk_team) = if attack_home {
        (TeamId::Home, TeamId::Away)
    } else {
        (TeamId::Away, TeamId::Home)
    };
    let params = world.params.clone();
    for p in &mut world.players {
        let keep = (p.team == atk && (p.id == PlayerId(1) || p.id == PlayerId(2)))
            || (p.team == gk_team && p.id == PlayerId(4));
        if keep {
            continue;
        }
        p.pos = park_off_pitch(p.team, p.id.0, &params);
        p.vel = Vec2::ZERO;
    }
}

/// After brains think: keep only attackers P1/P2 and GK P4 live.
pub fn apply_2v1_freeze(
    home_out: &mut BrainOutput,
    away_out: &mut BrainOutput,
    world: &MatchWorld,
    attack_home: bool,
) {
    if attack_home {
        freeze_except_many(home_out, world, TeamId::Home, &[1, 2]);
        freeze_except_many(away_out, world, TeamId::Away, &[4]);
    } else {
        freeze_except_many(home_out, world, TeamId::Home, &[4]);
        freeze_except_many(away_out, world, TeamId::Away, &[1, 2]);
    }
}

/// Scenario 3 setup: only the GK is a live participant for its own team —
/// park P1-3 off-pitch. The opponent team is untouched, plays at full
/// strength (all 4 players) with whatever brain is configured for it —
/// e.g. the real AIA3 graph, unmodified.
pub fn setup_4v1_harness(world: &mut MatchWorld, gk_home: bool) {
    let params = world.params.clone();
    let gk_team = if gk_home { TeamId::Home } else { TeamId::Away };
    let opp_team = gk_team.other();
    let opp_home = !gk_home;
    let sign = if opp_home { 1.0 } else { -1.0 };
    let own_goal_x = if opp_home {
        params.goal_line_x.abs()
    } else {
        -params.goal_line_x.abs()
    };
    let gk_pos = Vec2::new(own_goal_x - sign * 1.5, 0.0);

    // Canonical kickoff reset — the SAME sequence the real match loop runs
    // after any goal (world.rs's GoalPause -> Kickoff transition), not a
    // hand-rolled "ball already held by P1" shortcut. That shortcut was the
    // actual bug: it skipped the real walk-in/first-touch sequence the sim
    // (and AIA's own graph) expects, which is what caused the opponent to
    // score almost instantly and identically every single trial.
    place_kickoff(&mut world.ball, &mut world.players, opp_team, params.ball_rest_height);
    reset_possession_for_kickoff(&mut world.possession);
    world.possession.carrier = if world.ball.held {
        Some((opp_team, 1))
    } else {
        None
    };
    world.match_state.phase = MatchPhase::Kickoff;
    world.match_state.phase_timer = 0.0;
    world.match_state.kickoff_team = opp_team;
    world.match_state.kickoff_circle_lock = true;
    world.match_state.kickoff_suppress_away_team_side = true;
    world.match_state.kickoff_seen_carrier = false;
    world.match_state.clock_s = 0.0;
    world.match_state.reset_stale_tracker(world.ball.pos);

    // Drill-specific overrides on top of the canonical reset: only the GK
    // is a live participant for its own team (park P1-3, put the GK on
    // cover). `place_kickoff` already resets stamina/charge for every
    // player; the explicit reset below is redundant but harmless, kept so
    // this loop reads as a complete, self-sufficient round reset on its own.
    for p in &mut world.players {
        if p.team == gk_team && p.id == PlayerId(4) {
            p.pos = gk_pos;
            p.vel = Vec2::ZERO;
            p.facing = Vec2::new(-sign, 0.0);
        } else if p.team == gk_team {
            p.pos = park_off_pitch(p.team, p.id.0, &params);
            p.vel = Vec2::ZERO;
        }
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
        p.stamina = 1.0;
    }
}

/// After brains think: freeze the GK-only team's P1-3; opponent team is
/// fully live (all 4 players, untouched).
pub fn apply_4v1_freeze(
    home_out: &mut BrainOutput,
    away_out: &mut BrainOutput,
    world: &MatchWorld,
    gk_home: bool,
) {
    if gk_home {
        freeze_except(home_out, world, TeamId::Home, 4);
    } else {
        freeze_except(away_out, world, TeamId::Away, 4);
    }
}

/// Re-park the GK-only team's P1-3 after physics (Scenario 3 / 4v1).
pub fn repark_4v1_inactive(world: &mut MatchWorld, gk_home: bool) {
    let gk_team = if gk_home { TeamId::Home } else { TeamId::Away };
    let params = world.params.clone();
    for p in &mut world.players {
        if p.team == gk_team && p.id != PlayerId(4) {
            p.pos = park_off_pitch(p.team, p.id.0, &params);
            p.vel = Vec2::ZERO;
        }
    }
}
