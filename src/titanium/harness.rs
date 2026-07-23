//! Shared Titanium Scenario 1 harness setup (viewer + `titanium_drill`).
//!
//! Scenario 1 is a visible 1v1: only attacker P1 and goalkeeper P4 remain on
//! the pitch; the other six roster players stay off-pitch and frozen.

use bevy::prelude::Vec2;

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::match_state::MatchPhase;
use crate::params::SimParams;
use crate::player::PlayerId;
use crate::predict::gk_intercept_cover;
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
    for p in &world.players {
        if p.team != team {
            continue;
        }
        let idx = (p.id.0 as usize).saturating_sub(1);
        if idx >= 4 || p.id.0 == active {
            continue;
        }
        out.commands[idx] = BrainCommand {
            move_to: p.pos,
            sprint: false,
            interact: false,
        };
    }
}

/// Scenario 1: attacker P1 at center with ball; GK P4 on cone-bisector cover;
/// other roster players are parked off-pitch.
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
    let mut cover = gk_intercept_cover(
        atk_pos,
        own_goal_x,
        params.goal_half_width,
        &params,
        8.0,
    );
    cover.x = if own_goal_x > 0.0 {
        cover.x.max(0.0).max(atk_pos.x + 8.0)
    } else {
        cover.x.min(0.0).min(atk_pos.x - 8.0)
    };
    let deep = if own_goal_x > 0.0 {
        own_goal_x - 1.5
    } else {
        own_goal_x + 1.5
    };
    if own_goal_x > 0.0 {
        cover.x = cover.x.min(deep);
    } else {
        cover.x = cover.x.max(deep);
    }

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
            p.pos = cover;
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

    world.ball.pos = atk_pos + Vec2::new(sign * 0.55, 0.0);
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

/// Re-park inactive Scenario 1 players after physics clamps the pitch roster.
///
/// MatchWorld intentionally clamps every player to the playable AABB. Keeping
/// this harness-only correction here preserves that core behavior while making
/// inactive players remain invisible and unable to interact with the ball.
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
