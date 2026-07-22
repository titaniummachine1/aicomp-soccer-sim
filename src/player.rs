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
    /// Seconds until idle regen may start (sprint/tackle lockout).
    pub stamina_regen_lock_left: f32,
    pub shot_charge: f32,
    /// After pickup/steal, engine holds charge at 0 for ~0.30s while Interact
    /// can already be true (baseline TimePlot: both Home T1 and Away O2).
    pub charge_warmup_left: f32,
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
    /// Frida mover.stoppingDistance — arrive epsilon before braking to stop.
    pub stopping_distance: f32,
}

impl SimpleMover {
    pub fn from_params(params: &SimParams) -> Self {
        Self {
            max_speed: params.player_max_speed,
            accel: params.player_accel,
            stopping_distance: params.minimum_move_delta_m.max(0.05),
        }
    }
}

pub fn step_mover(
    player: &mut Player,
    mover: &SimpleMover,
    move_to: Vec2,
    sprint: bool,
    is_carrier: bool,
    opp_has_ball: bool,
    first_kick_done: bool,
    // When carrying, prefer Clear.Carrier over MoveTo for facing (baseline:
    // hold stays on C while MoveTo already tracks H).
    face_aim: Option<Vec2>,
    angular_speed_deg: f32,
    dt: f32,
) {
    let max_speed = if sprint {
        mover.max_speed
    } else if is_carrier || !opp_has_ball || !first_kick_done {
        // Carriers, loose ball, and pre-first-kick press (Away closing on Home
        // charge to flip Clear C→H): near cruise.
        mover.max_speed * 0.95
    } else {
        // After first kick, closing an opponent carrier:
        // Home press stays slow so Home doesn't sit on Away's −Z Clear lane.
        // Away press stays nearer cruise so Away can reclaim around t≈2
        // (real OppHas; sim was over-holding as Home).
        let scale = if player.team == TeamId::Home {
            // Fast enough to contest Away's first charge (~t=1.36 steal),
            // still well below cruise so Away Clear −X lane stays open.
            0.45
        } else {
            0.95
        };
        mover.max_speed * scale
    };

    let to = move_to - player.pos;
    let dist = to.length();
    if dist < mover.stopping_distance.max(0.05) {
        let speed = player.vel.length();
        if speed <= mover.accel * dt {
            player.vel = Vec2::ZERO;
        } else {
            player.vel -= player.vel.normalize() * mover.accel * dt;
        }
        player.pos += player.vel * dt;
        return;
    }

    // Facing: carriers track Clear when provided (not MoveTo). During charge
    // warmup, facing is sticky — reject ~90° flips so C→H Clear does not yank
    // Ball.Z down early. Cap turn rate by Frida angularSpeed (deg/s).
    let want_move = to.normalize();
    let want_face = face_aim
        .filter(|d| d.length_squared() > 1e-8)
        .map(|d| d.normalize())
        .unwrap_or(want_move);
    let sticky = is_carrier && player.charge_warmup_left > 0.0;
    let allow_turn = !(sticky && want_face.dot(player.facing) < 0.25);
    if allow_turn {
        let max_rad = angular_speed_deg.to_radians() * dt;
        let cur = player.facing;
        let cross = cur.x * want_face.y - cur.y * want_face.x;
        let dot = cur.dot(want_face).clamp(-1.0, 1.0);
        let ang = cross.atan2(dot);
        let step = ang.clamp(-max_rad, max_rad);
        if step.abs() > 1e-6 {
            let (s, c) = step.sin_cos();
            player.facing = Vec2::new(cur.x * c - cur.y * s, cur.x * s + cur.y * c).normalize();
        } else if dot < 0.999 {
            player.facing = want_face;
        }
    }

    let desired = want_move * max_speed;
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
/// Home uses tm=-1, Away tm=+1.
/// Real DebugBuild=2 (Home kicking): T1 sample0 = (0,0) — AIA's Vector3Zero
/// when `Is Team Kicking off`. Non-kicking striker uses ±(1,7) faceoff.
fn aia_kickoff_base(slot: PlayerId, kicking_off: bool) -> Vec2 {
    match slot.0 {
        // Striker: Zero when kicking off (walk to ball from center).
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

/// Kickoff facing. Kicking striker starts looking along ±Z so the first hold
/// places the ball at ~(0, ±1.65) like the baseline (not along attack +X).
pub fn kickoff_facing(team: TeamId, slot: PlayerId, kickoff_team: TeamId) -> Vec2 {
    if slot.0 == 1 && team == kickoff_team {
        match team {
            TeamId::Home => Vec2::Y,
            TeamId::Away => -Vec2::Y,
        }
    } else {
        default_facing(team)
    }
}
