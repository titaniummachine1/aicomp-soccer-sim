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
    /// 64-bit Interact history. Each tick: `bits = (bits << 1) | held`.
    /// LSB = this tick, bit1 = previous tick. Cleared on whistle / goal / reset.
    /// Rising edge (claim/tackle): LSB set, bit1 clear. Shot charge uses the
    /// consecutive held run in the low bits; release (falling edge) kicks.
    pub interact_bits: u64,
    /// After pickup/tackle, the carrier must RELEASE Interact before charging
    /// or kicking. While true, holding Interact does NOT charge and releasing
    /// does NOT kick — the ball stays held. Cleared on the first release.
    pub grab_hold_active: bool,
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
    // 1. DIRECTION. There is no look direction: facing IS walk direction, set
    // instantly and unconditionally. Measured from a real-game TimePlot over
    // 3858 ticks of a player carrying the ball while moving — the angle between
    // (ball - player) and the movement delta had median 0.00 deg and p90
    // 0.00 deg. The 51 ticks above 10 deg were all braking at a turnaround
    // (speed 3.2 -> 1.3 -> 0.6 m/s, the same three values every reversal),
    // i.e. the movement delta shrinking below the point where it defines a
    // direction — not the ball pointing anywhere on its own.
    //
    // Two mechanisms used to decouple the two and are gone:
    //   * `face_aim` forced the Away carrier onto Clear F for the opening
    //     kickoff. A hardcoded parity hack for one recording.
    //   * a `sticky` guard blocked turns over ~90 deg during charge warmup.
    // Both invented a state the recording says does not exist, and both broke
    // the invariant every anti-tackle prediction rests on: a held ball is at
    // player.pos + walk_direction * hold_offset, no exceptions, so the ball
    // rotates WITH the chosen heading before that heading is walked along.
    //
    // The removed `angular_speed_deg` rate limit is what made anti-tackle
    // unfixable: a carrier that chose a new heading had its ball swing out
    // along the OLD facing, misplaced by up to 3.2 m (2x the 1.67 m hold
    // offset), so every escape angle was planned for a ball position the tick
    // never produced.
    if dist > 1e-6 {
        player.facing = to.normalize();
    }

    // 2. MOVEMENT. Arrive / brake-to-rest first, then the accelerating case.
    if dist <= 1e-3 || dist <= brake_dist {
        if speed > 1e-6 {
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
        } else {
            player.vel = Vec2::ZERO;
        }
        return;
    }

    // Movement follows the heading set above. Reaching here implies dist > 1e-3,
    // so `player.facing` is exactly `to.normalize()` — you walk where you look
    // because they are the same vector.
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

/// Clearance past the kickoff circle a receiving player is pushed to.
/// Measured 7.75 against a 7.25 circle (Test1/Test2 probes, 2026-07-26).
pub const KICKOFF_PUSH_CLEARANCE: f32 = 0.50;

/// Faceoff space → world. **Identity for both sides — there is no mirroring.**
///
/// Measured, Test1/Test2 probes 2026-07-26. Home declared (0,20) spawns at
/// (0,20) and (-30,0) at (-30,0); Away declared (25,0) spawns at (25,0),
/// (0,-20) at (0,-20), (18,8) at (18,8). Both sides, unchanged, in world
/// coordinates.
///
/// So a faceoff position is NOT written in the team's own frame: a graph that
/// wants its keeper at the back has to know which end it is defending, and the
/// same numbers played from the other side put that keeper in the opponent's
/// half (where rule 2 below drags it onto the halfway line).
pub fn team_space_to_world(_team: TeamId, v: Vec2) -> Vec2 {
    v
}

/// Where a player actually stands when the whistle goes.
///
/// Three rules, in order:
///
/// 1. The graph picks the spot. `ConstructSoccerProperties` declares one per
///    player; a slot the graph left unwired keeps the engine default. This is
///    also what decides who takes the kickoff — there is no "kicker" field,
///    whoever you place on the ball takes it, so it need not be player 1.
/// 2. A team may only place inside its OWN half. Anything past the halfway
///    line is pulled back to it, and everything is kept inside the pitch.
/// 3. Only the team kicking off may stand in the center circle. The other
///    side is pushed straight out to the edge — observed in-game: at a
///    kickoff that is not theirs, players inside the circle get shoved out
///    of it while the kicking team's stay put.
pub fn kickoff_spawn(
    team: TeamId,
    slot: PlayerId,
    kickoff_team: TeamId,
    declared: Option<Vec2>,
    params: &SimParams,
) -> Vec2 {
    let mut pos = match declared {
        Some(v) => team_space_to_world(team, v),
        None => faceoff_world(team, slot, kickoff_team),
    };
    pos = clamp_to_own_half(team, pos, params);
    if team != kickoff_team {
        pos = push_out_of_kickoff_circle(pos, team, params);
    }
    pos
}

/// Keep a spawn inside the pitch and on the team's own side of halfway.
/// Home defends −X, Away defends +X.
///
/// Measured: an opponent-half spot is pulled to the halfway line EXACTLY —
/// Home's (30,0) and Away's (-25,0) both spawn at (0,0) — and Z is left
/// alone. No body-radius standoff, and no rejection back to a default.
fn clamp_to_own_half(team: TeamId, pos: Vec2, params: &SimParams) -> Vec2 {
    let x_limit = params.x_max.max(0.0);
    let z_limit = params.z_max.max(0.0);
    let x = match team {
        TeamId::Home => pos.x.clamp(-x_limit, 0.0),
        TeamId::Away => pos.x.clamp(0.0, x_limit),
    };
    Vec2::new(x, pos.y.clamp(-z_limit, z_limit))
}

/// Shove a receiving player out of the kickoff circle.
///
/// Measured 2026-07-26: a receiving player on the centre spot lands at
/// (0, 7.75) — radius `kickoff_circle_r + 0.50`, along +Z — for BOTH sides.
/// The 0.5 is not the body radius (0.655), and 7.75 is also exactly where
/// TimePlot DB33 put AIA's receiving striker, so two independent runs agree
/// on the distance.
///
/// The direction is only half measured. Both probe cases started from the
/// centre, where there is no radial direction to preserve, and both resolved
/// to +Z — so the degenerate case is known. Whether an off-centre player is
/// pushed radially (assumed here) or also snapped to +Z is NOT measured.
fn push_out_of_kickoff_circle(pos: Vec2, _team: TeamId, params: &SimParams) -> Vec2 {
    let edge = params.kickoff_circle_r + KICKOFF_PUSH_CLEARANCE;
    // Nudge exact-center +Z by 1 unit first so there's a direction to push along.
    let pos = if pos.length() <= 1e-4 {
        Vec2::new(0.0, 1.0)
    } else {
        pos
    };
    let d = pos.length();
    if d >= edge {
        return pos;
    }
    let dir = pos / d;
    dir * edge
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
            interact_bits: 0,
            grab_hold_active: false,
        }
    }

    #[test]
    fn facing_is_walk_direction_with_no_turn_rate_limit() {
        // The invariant the whole anti-tackle model rests on: choose a heading,
        // and by the time the tick is finalised you are facing it exactly — so
        // the held ball (facing * hold_offset) is where the solver placed it.
        //
        // Measured over 3858 real-game ticks of a carrier moving with the ball:
        // median angle between (ball - player) and the movement delta 0.00 deg.
        let params = SimParams::default();
        let mover = SimpleMover::from_params(&params);
        let mut player = test_player(Vec2::ZERO, Vec2::new(7.0, 0.0));
        assert_eq!(player.facing, Vec2::X);

        // A full 180 reversal, the worst case a turn cap would smear over
        // several ticks. One tick is enough.
        let target = Vec2::new(-50.0, 0.0);
        step_mover(&mut player, &mover, target, false, true, false, true, 0.019);
        assert!(
            (player.facing - Vec2::NEG_X).length() < 1e-5,
            "facing {:?} did not reach the chosen heading in one tick",
            player.facing
        );

        // And an arbitrary angle, while charging a shot — the old `sticky`
        // guard blocked exactly this turn.
        let mut charging = test_player(Vec2::ZERO, Vec2::new(7.0, 0.0));
        charging.charge_warmup_left = 0.2;
        let want = Vec2::new(-3.0, 4.0).normalize();
        step_mover(
            &mut charging,
            &mover,
            want * 50.0,
            false,
            true,
            false,
            true,
            0.019,
        );
        assert!(
            (charging.facing - want).length() < 1e-5,
            "charge warmup still rate-limits facing: {:?} want {want:?}",
            charging.facing
        );
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
            interact_bits: 0,
            grab_hold_active: false,
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
            dt,
        );
        let expected = 100.0 * dt; // 2.0 m/s after one tick from rest
        assert!(
            (player.vel.x - expected).abs() < 1e-3,
            "vel.x={} want {expected}",
            player.vel.x
        );
    }

    // Every expectation below is a measured value from the Test1/Test2
    // kickoff probes run in the real game on 2026-07-26, not a derivation.

    #[test]
    fn declared_spots_are_world_absolute_for_both_sides() {
        let params = SimParams::default();
        // Home declared (0,20) spawned at (0,20); (-30,0) at (-30,0).
        // Away declared (0,-20) spawned at (0,-20); (18,8) at (18,8).
        let cases = [
            (TeamId::Home, Vec2::new(0.0, 20.0)),
            (TeamId::Home, Vec2::new(-30.0, 0.0)),
            (TeamId::Away, Vec2::new(0.0, -20.0)),
            (TeamId::Away, Vec2::new(18.0, 8.0)),
            (TeamId::Away, Vec2::new(25.0, 0.0)),
        ];
        for (team, spot) in cases {
            // Kicking off, so the circle push cannot be what keeps it in place.
            let got = kickoff_spawn(team, PlayerId(1), team, Some(spot), &params);
            assert!(
                got.distance(spot) < 1e-4,
                "{team:?} declared {spot} should spawn unchanged, got {got}"
            );
        }
    }

    #[test]
    fn an_opponent_half_spot_is_dragged_to_the_halfway_line() {
        let params = SimParams::default();
        // Home's (30,0) and Away's (-25,0) both spawned at exactly (0,0):
        // X clamped to the halfway line, Z untouched, no body-radius standoff.
        for (team, spot) in [
            (TeamId::Home, Vec2::new(30.0, 0.0)),
            (TeamId::Away, Vec2::new(-25.0, 0.0)),
        ] {
            let got = kickoff_spawn(team, PlayerId(1), team, Some(spot), &params);
            assert!(
                got.distance(Vec2::ZERO) < 1e-4,
                "{team:?} illegal {spot} should land on (0,0), got {got}"
            );
        }
    }

    #[test]
    fn only_the_receiving_team_is_pushed_out_of_the_circle() {
        let params = SimParams::default();
        let edge = params.kickoff_circle_r + KICKOFF_PUSH_CLEARANCE;
        let centre = Vec2::ZERO;
        for team in [TeamId::Home, TeamId::Away] {
            // Kicking off: allowed to stand on the centre spot.
            let kicking = kickoff_spawn(team, PlayerId(1), team, Some(centre), &params);
            assert!(
                kicking.distance(centre) < 1e-4,
                "{team:?} kicking off was pushed off the spot: {kicking}"
            );
            // Receiving: pushed to 7.75, and the degenerate direction is world
            // +Z for BOTH sides — the fallback is not mirrored per team.
            let receiving = kickoff_spawn(team, PlayerId(1), team.other(), Some(centre), &params);
            assert!(
                receiving.distance(Vec2::new(0.0, edge)) < 1e-3,
                "{team:?} receiving should land on (0,{edge}), got {receiving}"
            );
        }
    }

    #[test]
    fn a_receiver_already_clear_of_the_circle_is_left_alone() {
        let params = SimParams::default();
        let spot = Vec2::new(0.0, 20.0);
        let got = kickoff_spawn(TeamId::Home, PlayerId(3), TeamId::Away, Some(spot), &params);
        assert!(got.distance(spot) < 1e-4, "moved a clear receiver: {got}");
    }

    #[test]
    fn unwired_slot_keeps_the_engine_default() {
        let params = SimParams::default();
        for team in [TeamId::Home, TeamId::Away] {
            for slot in PlayerId::ALL {
                let got = kickoff_spawn(team, slot, TeamId::Home, None, &params);
                let want = faceoff_world(team, slot, TeamId::Home);
                // 1e-3, not tighter: the AIA striker default sits at radius
                // 7.7503 against a 7.75 push edge, so the push does touch it —
                // by 3e-4. That the measured default lands within a thousandth
                // of the measured push edge is itself a cross-check.
                assert!(
                    got.distance(want) < 1e-3,
                    "{team:?} {slot:?}: default moved {want} -> {got}"
                );
            }
        }
    }

    #[test]
    fn a_spot_outside_the_pitch_is_pulled_inside() {
        let params = SimParams::default();
        let p = kickoff_spawn(
            TeamId::Home,
            PlayerId(1),
            TeamId::Home,
            Some(Vec2::new(-999.0, 999.0)),
            &params,
        );
        assert!(p.x >= -params.x_max - 1e-4, "off the back line: {p}");
        assert!(p.y <= params.z_max + 1e-4, "off the touchline: {p}");
    }
}
