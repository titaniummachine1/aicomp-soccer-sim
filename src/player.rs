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
    /// Accel from rest / toward a non-zero desired (measured 100).
    pub accel: f32,
    /// Brake-to-rest (TimePlot median 200 — exactly 2× accel).
    pub decel: f32,
}

impl SimpleMover {
    pub fn from_params(params: &SimParams) -> Self {
        Self {
            max_speed: params.player_max_speed,
            walk_speed: params.player_walk_speed,
            sprint_empty_speed: params.player_sprint_empty_speed,
            accel: params.player_accel,
            decel: params.player_decel,
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
    let speed = player.vel.length();
    // Physics brake distance: v²/(2a). From sprint 8 @ decel 200 → 0.16 m
    // (was wrongly 1.25 m NavMesh stoppingDistance — pathfinding only).
    let brake_dist = if speed > 1e-6 {
        speed * speed / (2.0 * mover.decel.max(1e-6))
    } else {
        0.0
    };
    // Arrive / brake-to-rest: target reached, or within the distance needed
    // to stop at decel 200. Kinematic update matches continuous s=v²/(2a)
    // (same form as ball Coulomb slide).
    if dist <= 1e-3 || dist <= brake_dist {
        if speed <= 1e-6 {
            player.vel = Vec2::ZERO;
            return;
        }
        let dir = player.vel.normalize();
        let decel = mover.decel.max(1e-6);
        let dv = decel * dt;
        if speed <= dv {
            // Coast the residual stop distance, then rest.
            player.pos += dir * (speed * speed / (2.0 * decel));
            player.vel = Vec2::ZERO;
        } else {
            player.pos += dir * (speed * dt - 0.5 * decel * dt * dt);
            player.vel = dir * (speed - dv);
        }
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

    fn test_player(pos: Vec2, vel: Vec2) -> Player {
        Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos,
            vel,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        }
    }

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

    /// TimePlot 2026-07-25: decel-to-rest median 200. From sprint 8 m/s:
    /// s = v²/(2a) = 0.16 m, t = v/a = 0.04 s. Old sim braked over 1.25 m.
    #[test]
    fn brake_from_sprint_stops_in_0_16m() {
        let params = SimParams::default();
        let mover = SimpleMover::from_params(&params);
        assert!((mover.decel - 200.0).abs() < 1e-3);
        let mut player = test_player(Vec2::ZERO, Vec2::new(8.0, 0.0));
        let dt = 0.019;
        let start = player.pos;
        let mut t = 0.0;
        for _ in 0..20 {
            if player.vel.length() < 1e-4 {
                break;
            }
            // MoveTo underfoot = "arrived / stop" — pure decel-to-rest path.
            let move_to = player.pos;
            step_mover(
                &mut player,
                &mover,
                move_to,
                true,
                false,
                false,
                true,
                None,
                params.angular_speed_deg,
                dt,
            );
            t += dt;
        }
        let travel = start.distance(player.pos);
        assert!(
            player.vel.length() < 1e-3,
            "should be at rest, vel={}",
            player.vel.length()
        );
        assert!(
            (travel - 0.16).abs() < 0.02,
            "stop distance {travel} (want ≈0.16 = 8²/(2·200))"
        );
        assert!(
            (t - 0.04).abs() < 0.025,
            "stop time {t} (want ≈0.04 = 8/200)"
        );
    }

    #[test]
    fn brake_tick_uses_decel_200_not_accel_100() {
        let params = SimParams::default();
        let mover = SimpleMover::from_params(&params);
        let mut player = test_player(Vec2::ZERO, Vec2::new(8.0, 0.0));
        let dt = 0.019;
        step_mover(
            &mut player,
            &mover,
            Vec2::ZERO,
            true,
            false,
            false,
            true,
            None,
            params.angular_speed_deg,
            dt,
        );
        let expected = 8.0 - 200.0 * dt;
        assert!(
            (player.vel.x - expected).abs() < 1e-3,
            "vel.x={} want {expected} (decel 200, not accel 100)",
            player.vel.x
        );
    }

    #[test]
    fn accel_from_rest_is_100() {
        let params = SimParams::default();
        let mover = SimpleMover::from_params(&params);
        assert!((mover.accel - 100.0).abs() < 1e-3);
        let mut player = test_player(Vec2::ZERO, Vec2::ZERO);
        let dt = 0.02;
        step_mover(
            &mut player,
            &mover,
            Vec2::new(50.0, 0.0),
            true,
            false,
            false,
            true,
            None,
            params.angular_speed_deg,
            dt,
        );
        let expected = 100.0 * dt; // 2.0 m/s after one tick from rest
        assert!(
            (player.vel.x - expected).abs() < 1e-3,
            "vel.x={} want {expected}",
            player.vel.x
        );
    }
}
