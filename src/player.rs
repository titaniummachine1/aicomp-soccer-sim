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

    /// Hold point after wall projection (Unity v0.61 / PerfectController probe).
    pub fn hold_pos_playable(&self, params: &SimParams) -> Vec2 {
        project_hold_into_playable(self.hold_pos(params.hold_offset), params)
    }
}

/// Project a held-ball center into the playable region.
///
/// Matches free-ball walls: sidelines always; endlines only outside the goal
/// mouth (mouth stays open so walk-in goals remain possible). Yaw is not
/// rejected — Unity TimePlot 2026-07-24: free rotate against walls while the
/// ball slides on the boundary and `|Ball-Body|` compresses.
pub fn project_hold_into_playable(hold: Vec2, params: &SimParams) -> Vec2 {
    let mut actual = hold;
    actual.y = actual.y.clamp(params.z_min, params.z_max);
    let in_mouth = actual.y.abs() <= params.goal_half_width;
    if !in_mouth {
        actual.x = actual.x.clamp(params.x_min, params.x_max);
    }
    actual
}

#[derive(Component, Debug, Clone, Copy)]
pub struct SimpleMover {
    /// Sprint cap with stamina remaining (measured 8.0).
    pub max_speed: f32,
    /// Cruise / non-sprint (measured 7.0).
    pub walk_speed: f32,
    /// Sprint held with stamina at zero (measured 7.65) — faster than a walk,
    /// slower than a real sprint.
    pub sprint_empty_speed: f32,
    pub accel: f32,
    /// Frida mover.stoppingDistance — arrive epsilon before braking to stop.
    pub stopping_distance: f32,
}

impl SimpleMover {
    pub fn from_params(params: &SimParams) -> Self {
        Self {
            max_speed: params.player_max_speed,
            walk_speed: params.player_walk_speed,
            sprint_empty_speed: params.player_sprint_empty_speed,
            accel: params.player_accel,
            stopping_distance: params.stopping_distance_m.max(0.05),
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
    // Measured (user, 2026-07-25): walk 7.0, sprint 8.0, sprint on EMPTY
    // stamina 7.65. The old code used max_speed*0.95 (=7.6) for every
    // non-sprint case and ignored player_walk_speed entirely, so walking was
    // 8.6% too fast; sprinting also ignored stamina and always gave 8.0.
    let max_speed = if sprint {
        if player.stamina <= 0.0 {
            mover.sprint_empty_speed
        } else {
            mover.max_speed
        }
    } else if is_carrier || !opp_has_ball || !first_kick_done {
        mover.walk_speed
    } else {
        // After the match opening kick, both sides close an opponent carrier
        // at the same near-cruise rate. An older Home-only 0.45 scale was for
        // the opening Away Clear lane; leaving it on for the whole match made
        // every post-goal Home kickoff a free Away goal (and flipped cleanly
        // when Away took kickoffs).
        mover.walk_speed
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

    // Facing follows MoveTo (held ball sits on facing × hold_offset). Carriers
    // only override via `face_aim` during charge warmup (Clear sticky, quirk #24).
    let want_move = to.normalize();
    let want_face = face_aim
        .filter(|d| d.length_squared() > 1e-8)
        .map(|d| d.normalize())
        .unwrap_or(want_move);
    let sticky = is_carrier
        && player.charge_warmup_left > 0.0
        // Away opening must be allowed to yaw from walk-in +Z onto Clear F;
        // the generic sticky reject of ~90° flips blocked exactly that turn.
        && !(player.team == TeamId::Away && !first_kick_done);
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
///
/// Engine spawn is always the wing faceoff for strikers. AIA's
/// `StrikerKickoffPos = Vector3Zero` when kicking off is a **MoveTo target**,
/// not spawn — TimePlot DB33 (Away open): O1 starts ~(1,-7) and walks ~1s to
/// the free ball at origin; Home T1 parks ~(-1.1, 7.67) **outside** r=7.25.
/// Older T1=(0,0) samples are post-walk / mid-pickup, not engine teleport.
fn aia_kickoff_base(slot: PlayerId, kicking_off: bool) -> Vec2 {
    match slot.0 {
        // Striker: kicker walks in from inner wing; receiver parks outside circle
        // (DB33 Home T1 ≈ (-1.096, 7.672) → base (1.096, -7.672)).
        1 => {
            if kicking_off {
                Vec2::new(1.0, -7.0)
            } else {
                Vec2::new(1.096, -7.672)
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

/// Kickoff facing at spawn (wing faceoff). Opening hold ±Z is applied when the
/// kicker actually picks up at center (possession / charge path), not by
/// teleporting them onto the ball at place_kickoff.
pub fn kickoff_facing(team: TeamId, _slot: PlayerId, _kickoff_team: TeamId) -> Vec2 {
    default_facing(team)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_ball_projects_onto_sideline_without_changing_facing() {
        let params = SimParams::default();
        let facing = Vec2::Y;
        let player = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::new(0.0, params.z_max - 0.25),
            vel: Vec2::ZERO,
            facing,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        let desired = player.hold_pos(params.hold_offset);
        let actual = player.hold_pos_playable(&params);
        assert!(desired.y > params.z_max);
        assert_eq!(actual.y, params.z_max);
        assert_eq!(player.facing, facing);
        // Offset compresses (Unity wall rub) — ball closer than full hold_offset.
        assert!(actual.distance(player.pos) < params.hold_offset - 0.05);
    }

    #[test]
    fn held_ball_may_enter_goal_mouth_past_endline() {
        let params = SimParams::default();
        let hold = Vec2::new(params.x_max + 2.0, 0.0);
        assert_eq!(project_hold_into_playable(hold, &params), hold);
    }

    #[test]
    fn held_ball_clamps_endline_outside_mouth() {
        let params = SimParams::default();
        let hold = Vec2::new(params.x_max + 2.0, params.goal_half_width + 1.0);
        let actual = project_hold_into_playable(hold, &params);
        assert_eq!(actual.x, params.x_max);
        assert_eq!(actual.y, hold.y.clamp(params.z_min, params.z_max));
    }
}
