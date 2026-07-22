//! Simple movers — controller-driven, pitch AABB clamped (no navmesh).

use bevy::prelude::*;

use crate::brain::TeamId;
use crate::params::SimParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(pub u8);

impl PlayerId {
    pub const ALL: [PlayerId; 4] = [PlayerId(1), PlayerId(2), PlayerId(3), PlayerId(4)];
}

#[derive(Component, Debug, Clone)]
pub struct Player {
    pub team: TeamId,
    pub id: PlayerId,
    pub pos: Vec2,
    pub vel: Vec2,
    /// Unit facing on the pitch (look / hold / shoot axis).
    pub facing: Vec2,
    pub stamina: f32,
    pub shot_charge: f32,
}

impl Player {
    /// World position of BallHoldLocation (capture / carry / shoot origin).
    pub fn hold_pos(&self, hold_offset: f32) -> Vec2 {
        self.pos + self.facing * hold_offset
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct SimpleMover {
    pub max_speed: f32,
    pub accel: f32,
}

impl SimpleMover {
    pub fn from_params(params: &SimParams) -> Self {
        Self {
            max_speed: params.player_max_speed,
            accel: params.player_accel,
        }
    }
}

pub fn step_mover(
    player: &mut Player,
    mover: &SimpleMover,
    move_to: Vec2,
    sprint: bool,
    dt: f32,
) {
    let max_speed = if sprint {
        mover.max_speed
    } else {
        mover.max_speed * 0.7
    };

    let to = move_to - player.pos;
    let dist = to.length();
    if dist < 0.05 {
        let speed = player.vel.length();
        if speed <= mover.accel * dt {
            player.vel = Vec2::ZERO;
        } else {
            player.vel -= player.vel.normalize() * mover.accel * dt;
        }
        player.pos += player.vel * dt;
        return;
    }

    // Facing follows move intent (same axis as hold/shoot in the real game).
    player.facing = to.normalize();

    let desired = player.facing * max_speed;
    let delta = desired - player.vel;
    let max_delta = mover.accel * dt;
    if delta.length() <= max_delta {
        player.vel = desired;
    } else {
        player.vel += delta.normalize() * max_delta;
    }
    let speed = player.vel.length();
    if speed > max_speed {
        player.vel *= max_speed / speed;
    }
    player.pos += player.vel * dt;
}

/// AIA kickoff bases before `TeamMultiplier` (world XZ → our xy).
/// Home uses tm=-1, Away tm=+1. Striker sits on center only when that team kicks off.
fn aia_kickoff_base(slot: PlayerId, kicking_off: bool) -> Vec2 {
    match slot.0 {
        // Striker
        1 => {
            if kicking_off {
                Vec2::ZERO
            } else {
                Vec2::new(1.0, -7.0)
            }
        }
        // Playmaker
        2 => Vec2::new(11.0, 0.0),
        // Defender
        3 => Vec2::new(5.0, 7.0),
        // Goalie (near own goal line ~±36; posts at ±40.2)
        4 => Vec2::new(36.0, 0.0),
        _ => Vec2::ZERO,
    }
}

/// World kickoff spot. Unity: Home defends −X, Away defends +X.
pub fn faceoff_world(team: TeamId, slot: PlayerId, kickoff_team: TeamId) -> Vec2 {
    let tm = match team {
        TeamId::Home => -1.0,
        TeamId::Away => 1.0,
    };
    let base = aia_kickoff_base(slot, team == kickoff_team);
    Vec2::new(base.x * tm, base.y * tm)
}

pub fn default_facing(team: TeamId) -> Vec2 {
    match team {
        // Home attacks +X (Away goal), Away attacks −X (Home goal).
        TeamId::Home => Vec2::X,
        TeamId::Away => -Vec2::X,
    }
}
