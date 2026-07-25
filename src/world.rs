//! Cheap headless match world — no navmesh; no player↔ball body push.
//!
//! Design for 10⁵–10⁶ sims:
//! - Fixed-dt step, plain structs (Bevy is only the interactive viewer)
//! - Ball is the only sliding physics body (walls/posts only; no player shove)
//! - Players = accel movers + axis-aligned walk limits (pitch AABB only)
//! - Possession via interact radius only
//! - Brains run inline on the sim thread (no per-match worker threads in batch)
//!
//! Goal-mouth entry is open through the center; outside the mouth, players
//! remain clamped to the playable AABB. Free-ball post response is unchanged.

use bevy::prelude::Vec2;

use crate::api::{build_team_api, first_clear_dir, TeamApi, WorldSensors};
use crate::ball::{goal_at, step_free_ball, Ball, EndReason};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::match_state::{
    kickoff_control_allowed, place_kickoff, receiving_team_circle_locked, MatchPhase, MatchState,
};
use crate::params::SimParams;
use crate::player::{faceoff_world, kickoff_facing, step_mover, Player, PlayerId, SimpleMover};
use crate::possession::{
    apply_interact, reset_possession_for_kickoff, sync_held_ball, tick_possession_timers,
    Possession,
};
/// Confirmed AIComp fixed step (~52.6 Hz). Independent of render FPS.
pub const FIXED_DT: f32 = 0.019;

#[derive(Debug, Clone)]
pub struct MatchWorld {
    pub params: SimParams,
    pub match_state: MatchState,
    pub possession: Possession,
    pub ball: Ball,
    pub players: Vec<Player>,
    pub mover: SimpleMover,
}

impl MatchWorld {
    /// Fresh match: opening kickoff side is random; after a goal the conceded side restarts.
    pub fn new_kickoff(params: SimParams) -> Self {
        Self::new_kickoff_with(params, MatchState::new_match())
    }

    /// Deterministic opening (tests / seeded batch runs).
    pub fn new_kickoff_opening(params: SimParams, opening: TeamId) -> Self {
        Self::new_kickoff_with(params, MatchState::with_opening_kickoff(opening))
    }

    fn new_kickoff_with(params: SimParams, match_state: MatchState) -> Self {
        let mover = SimpleMover::from_params(&params);
        let opening = match_state.kickoff_team;
        let mut players = Vec::with_capacity(8);
        for team in [TeamId::Home, TeamId::Away] {
            for id in PlayerId::ALL {
                players.push(Player {
                    team,
                    id,
                    pos: faceoff_world(team, id, opening),
                    vel: Vec2::ZERO,
                    facing: kickoff_facing(team, id, opening),
                    stamina: 1.0,
                    stamina_regen_lock_left: 0.0,
                    shot_charge: 0.0,
                    charge_warmup_left: 0.0,
                });
            }
        }
        let mut world = Self {
            params,
            match_state,
            possession: Possession::default(),
            ball: Ball::default(),
            players,
            mover,
        };
        place_kickoff(
            &mut world.ball,
            &mut world.players,
            world.match_state.kickoff_team,
            world.params.ball_rest_height,
        );
        world
    }

    pub fn build_apis(&self) -> (TeamApi, TeamApi) {
        let sensors = WorldSensors {
            ball: &self.ball,
            players: &self.players,
            possession: &self.possession,
            match_state: &self.match_state,
            params: &self.params,
        };
        (
            build_team_api(TeamId::Home, &sensors),
            build_team_api(TeamId::Away, &sensors),
        )
    }

    /// One fixed tick. `home` / `away` brains evaluated by caller (or inline).
    pub fn step_with_commands(&mut self, home: &BrainOutput, away: &BrainOutput, dt: f32) {
        if self.match_state.phase == MatchPhase::GoalPause {
            self.match_state.phase_timer -= dt;
            if self.match_state.phase_timer <= 0.0 {
                place_kickoff(
                    &mut self.ball,
                    &mut self.players,
                    self.match_state.kickoff_team,
                    self.params.ball_rest_height,
                );
                reset_possession_for_kickoff(&mut self.possession);
                self.possession.carrier = if self.ball.held {
                    Some((self.match_state.kickoff_team, 1))
                } else {
                    None
                };
                self.match_state.phase = MatchPhase::Kickoff;
                self.match_state.phase_timer = 0.0;
                self.match_state.kickoff_circle_lock = true;
                self.match_state.kickoff_suppress_away_team_side = true;
                self.match_state.kickoff_seen_carrier = false;
            }
            return;
        }

        if self.match_state.phase == MatchPhase::Kickoff {
            self.match_state.phase_timer += dt;
            // Unity Is_Kickoff clears on pickup (DB33 ~1s), not on a body nudge of
            // the free center ball. Fall back only if nobody claims for a long time.
            let first_touch = self.possession.carrier.is_some();
            if first_touch || self.match_state.phase_timer > 8.0 {
                self.match_state.phase = MatchPhase::Play;
                self.match_state.reset_stale_tracker(self.ball.pos);
            }
        } else {
            self.match_state.clock_s += dt;
            self.match_state
                .tick_match_stats(dt, self.possession.carrier, self.ball.pos.x);
        }

        // Unlock circle + Away team-side chase after first release or ball leaves ring.
        if self.possession.carrier.is_some() {
            self.match_state.kickoff_seen_carrier = true;
        } else if self.match_state.kickoff_seen_carrier {
            self.match_state.kickoff_circle_lock = false;
            self.match_state.kickoff_suppress_away_team_side = false;
        }
        if self.ball.pos.length() > self.params.kickoff_circle_r {
            self.match_state.kickoff_circle_lock = false;
            self.match_state.kickoff_suppress_away_team_side = false;
        }

        tick_possession_timers(&mut self.possession, dt);

        // Index loop so we can re-read live carrier stamina and apply drain
        // immediately — a frame-start snapshot let later tacklers duel a stale
        // (often 0) carrier stam and ping-pong steals every tick.
        for i in 0..self.players.len() {
            let (team, id) = (self.players[i].team, self.players[i].id);
            let raw = match team {
                TeamId::Home => home.for_player(id),
                TeamId::Away => away.for_player(id),
            };
            let carrier_stam = self.possession.carrier.and_then(|(t, cid)| {
                self.players
                    .iter()
                    .find(|p| p.team == t && p.id.0 == cid)
                    .map(|p| p.stamina)
            });
            let carrier_charge = self.possession.carrier.and_then(|(t, cid)| {
                self.players
                    .iter()
                    .find(|p| p.team == t && p.id.0 == cid)
                    .map(|p| p.shot_charge)
            });
            // Kickoff restrictions are engine rules, not AI substitution:
            // only the kicking striker may walk into the opening ball.
            let cmd = filter_kickoff(
                &self.players[i],
                raw,
                &self.match_state,
                &self.params,
                self.ball.pos,
            );
            // let cmd = project_move_outside_kickoff_circle(
            //     &self.players[i],
            //     cmd,
            //     &self.match_state,
            //     &self.params,
            // );
            // let cmd = bias_receiving_defender_opening_hold(&self.players[i], cmd, &self.match_state);

            // Opening Away dump: keep carrier on Unity's mostly-west lane while
            // charge facing stays Clear F (face_aim bias). Raw Clear often falls
            // to A (+Z) and flips O1/Ball.Z vs DB35.
            // let cmd = bias_away_opening_carrier_dump_lane(&self.players[i], cmd, &self.possession);
            // DB33: Home T2 was tackling Away mid-charge (~1.8s) before the
            // opening dump. Unity lets Away finish Charge→release first; Home
            // then claims the loose ball. Strip receiving interact until the
            // first kick lands (live flag so same-tick post-kick reclaim works).
            // let cmd = bias_receiving_opening_no_tackle(
            //     &self.players[i],
            //     cmd,
            //     &self.match_state,
            //     self.possession.kickoff_touch_done,
            // );
            let is_carrier = matches!(
                self.possession.carrier,
                Some((t, id)) if t == team && id == self.players[i].id.0
            );
            let opp_has_ball = matches!(
                self.possession.carrier,
                Some((t, _)) if t != team
            );
            let face_aim = if is_carrier
                && (self.players[i].charge_warmup_left > 0.0
                    || (team == TeamId::Away && !self.possession.first_kick_done))
            {
                // AIA quirk #24: during charge warmup, hold faces Clear
                // (MoveTo may already track H). Opening Away dump (DB35): keep
                // facing Clear F for the whole hold so Ball.Z stays −Z; warmup
                // alone left facing on MoveTo−X and Ball.Z sat +Z of O1.
                let origin = self.players[i].pos;
                let is_home = team == TeamId::Home;
                let blockers: Vec<Vec2> = self
                    .players
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, p)| p.pos)
                    .collect();
                let blocker_r = self.params.body_radius + crate::api::SPHERECAST_RADIUS;
                let raw = first_clear_dir(
                    origin,
                    is_home,
                    &blockers,
                    blocker_r,
                    crate::api::SPHERECAST_DISTANCE,
                    true,
                    true,
                    self.params.x_min,
                    self.params.x_max,
                    self.params.z_min,
                    self.params.z_max,
                );
                crate::api::bias_away_opening_clear_f(
                    is_home,
                    self.possession.first_kick_done,
                    true,
                    raw,
                )
            } else {
                None
            };
            step_mover(
                &mut self.players[i],
                &self.mover,
                cmd.move_to,
                cmd.sprint,
                is_carrier,
                opp_has_ball,
                self.possession.first_kick_done,
                face_aim,
                self.params.angular_speed_deg,
                dt,
            );
            clamp_player_to_pitch(&mut self.players[i], &self.params);
            clamp_receiving_team_outside_kickoff_circle(
                &mut self.players[i],
                &self.match_state,
                &self.params,
            );
            tick_stamina(&mut self.players[i], cmd.sprint, dt, &self.params);
            let kickoff_elapsed = if self.match_state.phase == MatchPhase::Kickoff {
                Some(self.match_state.phase_timer)
            } else {
                None
            };
            let outcome = apply_interact(
                &mut self.players[i],
                &mut self.ball,
                &mut self.possession,
                cmd,
                &self.params,
                dt,
                carrier_stam,
                carrier_charge,
                kickoff_elapsed,
            );
            if outcome.shot {
                self.match_state.record_shot(team);
            }
            if let Some(drain) = outcome.drain {
                if let Some(c) = self
                    .players
                    .iter_mut()
                    .find(|p| p.team == drain.team && p.id.0 == drain.id)
                {
                    c.stamina = (c.stamina - drain.drain).max(0.0);
                    if drain.drain > 0.0 {
                        c.stamina_regen_lock_left = self
                            .params
                            .stamina_tackle_regen_delay_s
                            .max(c.stamina_regen_lock_left);
                    }
                }
            }
        }

        sync_held_ball(
            &mut self.ball,
            &self.players,
            &self.possession,
            &self.params,
        );

        // Always verify goal volume after hold sync. Ball center past the
        // line inside the mouth scores, held or not — no carrier-body gate.
        // The real game does not check who is holding the ball, only where
        // it is; treating that as an exploit to guard against was wrong.
        let scored = if self.ball.held {
            goal_at(self.ball.pos, &self.params)
        } else {
            let scored = step_free_ball(&mut self.ball, &self.params, dt);
            // No player↔ball body shove (Unity: Interact-only possession).
            if scored != EndReason::None {
                scored
            } else {
                goal_at(self.ball.pos, &self.params)
            }
        };
        self.apply_goal(scored);
        if scored == EndReason::None && self.match_state.phase == MatchPhase::Play {
            self.tick_stale_ball(dt);
        }
    }

    /// Frida: ball must travel ≥ `stale_ball_distance_threshold_m` within
    /// `stale_ball_timeout_s` or whistle resets to a flipped kickoff (no score).
    fn tick_stale_ball(&mut self, dt: f32) {
        let moved = (self.ball.pos - self.match_state.stale_anchor).length();
        if moved >= self.params.stale_ball_distance_threshold_m {
            self.match_state.reset_stale_tracker(self.ball.pos);
            return;
        }
        self.match_state.stale_idle_s += dt;
        if self.match_state.stale_idle_s >= self.params.stale_ball_timeout_s {
            self.match_state.on_whistle(self.params.kickoff_delay_s);
            // Ball parked at center until GoalPause → place_kickoff; clear
            // possession machine now so shooter-exclude can't block the next
            // kickoff claim (timers freeze during GoalPause).
            reset_possession_for_kickoff(&mut self.possession);
            self.ball.held = false;
            self.ball.vel = Vec2::ZERO;
            self.ball.vel_y = 0.0;
            self.ball.height = self.params.ball_rest_height;
            self.ball.pos = Vec2::ZERO;
        }
    }

    fn apply_goal(&mut self, scored: EndReason) {
        match scored {
            EndReason::GoalHome => {
                self.match_state
                    .on_goal(TeamId::Home, self.params.kickoff_delay_s);
                reset_possession_for_kickoff(&mut self.possession);
                self.ball.held = false;
                self.ball.vel = Vec2::ZERO;
                self.ball.vel_y = 0.0;
                self.ball.height = self.params.ball_rest_height;
            }
            EndReason::GoalAway => {
                self.match_state
                    .on_goal(TeamId::Away, self.params.kickoff_delay_s);
                reset_possession_for_kickoff(&mut self.possession);
                self.ball.held = false;
                self.ball.vel = Vec2::ZERO;
                self.ball.vel_y = 0.0;
                self.ball.height = self.params.ball_rest_height;
            }
            EndReason::None => {}
        }
    }

    pub fn step_brains<H: TeamBrain + ?Sized, A: TeamBrain + ?Sized>(
        &mut self,
        home: &mut H,
        away: &mut A,
        dt: f32,
    ) {
        let (home_api, away_api) = self.build_apis();
        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        self.step_with_commands(&home_out, &away_out, dt);
    }
}

fn filter_kickoff(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    params: &SimParams,
    ball_pos: Vec2,
) -> BrainCommand {
    if kickoff_control_allowed(player.team, player.id, match_state, params) {
        return cmd;
    }
    // Engine-scripted kickoff walk-in (user 2026-07-23): empty team configs
    // still send the kicking striker to the free ball; graph MoveTo is ignored
    // until Play / pickup. Everyone else idles on faceoff.
    if match_state.phase == MatchPhase::Kickoff
        && player.team == match_state.kickoff_team
        && player.id.0 == 1
    {
        return BrainCommand {
            move_to: ball_pos,
            sprint: false,
            interact: true,
        };
    }
    BrainCommand {
        move_to: player.pos,
        sprint: false,
        interact: false,
    }
}

/// Retained Unity probe hook; currently disabled so raw TXT commands are used.
#[allow(dead_code)]
/// While opening-chase is suppressed, pin the *receiving* Defender MoveTo.x
/// near the State0 skirt so they don't march onto the carrier C-lane.
fn bias_receiving_defender_opening_hold(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
) -> BrainCommand {
    if player.id.0 != 3
        || !match_state.kickoff_suppress_away_team_side
        || player.team == match_state.kickoff_team
    {
        return cmd;
    }
    // Home defends −X (skirt ≈ −5.5); Away defends +X (skirt ≈ +5.5).
    let skirt_x = match player.team {
        TeamId::Home => -5.5,
        TeamId::Away => 5.5,
    };
    BrainCommand {
        move_to: Vec2::new(skirt_x, cmd.move_to.y),
        sprint: cmd.sprint,
        interact: false,
    }
}

/// Retained Unity probe hook; currently disabled so raw TXT commands are used.
#[allow(dead_code)]
/// Away opening carrier (DB35): Unity walks mostly −X with mild −Z while
/// charging (O1≈(−3,−1.2) at release). Forcing Clear-F into MoveTo overshoots
/// −Z and the dump flies past Home T2. Lead with a capped west/south step;
/// facing/kick stay on F via `bias_away_opening_clear_f`.
fn bias_away_opening_carrier_dump_lane(
    player: &Player,
    cmd: BrainCommand,
    poss: &crate::possession::Possession,
) -> BrainCommand {
    if poss.first_kick_done || player.team != TeamId::Away || player.id.0 != 1 {
        return cmd;
    }
    if !matches!(poss.carrier, Some((TeamId::Away, 1))) {
        return cmd;
    }
    // Cap near Unity release pose ≈(−3,−1.2) so the F dump stays claimable
    // by Home T2 (Unity claim ~t=1.7–1.8). Uncapped west lead released from
    // ≈(−5,−2) and the ball flew past everyone.
    let target = Vec2::new(
        (player.pos.x - 2.0).max(-3.1),
        (player.pos.y - 0.45).clamp(-1.35, -0.4),
    );
    BrainCommand {
        move_to: target,
        ..cmd
    }
}

/// Retained Unity probe hook; currently disabled so raw TXT commands are used.
#[allow(dead_code)]
/// Block receiving-team tackles on the kickoff carrier until this kickoff's
/// first kick has been released (Playmaker T2 was poaching before Away's dump).
fn bias_receiving_opening_no_tackle(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    kickoff_touch_done: bool,
) -> BrainCommand {
    if kickoff_touch_done
        || !match_state.kickoff_suppress_away_team_side
        || player.team == match_state.kickoff_team
    {
        return cmd;
    }
    BrainCommand {
        interact: false,
        ..cmd
    }
}

/// Retained Unity probe hook; currently disabled so raw TXT commands are used.
#[allow(dead_code)]
/// Receiving team is idle during Kickoff (`filter_kickoff`); faceoff ±(1,7)
/// sits **inside** r=7.25 (quirk #13). Do not rewrite MoveTo to the ring —
/// that marched Home T1 off its park in the first second (Unity DB33 holds
/// T1 until pickup ~1s). Deeper intrusion is still blocked by
/// `clamp_receiving_team_outside_kickoff_circle`.
fn project_move_outside_kickoff_circle(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    _params: &SimParams,
) -> BrainCommand {
    let _ = (player, match_state);
    cmd
}

/// Pitch walk box. Goal mouths stay open so carriers can walk into the net
/// (Unity walk-in goals). Outside the mouth, X is clamped to the playable AABB.
pub fn clamp_player_to_pitch(player: &mut Player, params: &SimParams) {
    player.pos.y = player.pos.y.clamp(params.z_min, params.z_max);

    let in_mouth = player.pos.y.abs() <= params.goal_half_width;
    // Past the goal line into the net (posts sit ~0.7m past the line).
    let net_back = params.goal_line_x.abs() + 3.0;
    if in_mouth {
        player.pos.x = player.pos.x.clamp(-net_back, net_back);
    } else {
        player.pos.x = player.pos.x.clamp(params.x_min, params.x_max);
    }

    let x_lo = if in_mouth { -net_back } else { params.x_min };
    let x_hi = if in_mouth { net_back } else { params.x_max };
    if player.pos.x <= x_lo && player.vel.x < 0.0 || player.pos.x >= x_hi && player.vel.x > 0.0 {
        player.vel.x = 0.0;
    }
    if player.pos.y <= params.z_min && player.vel.y < 0.0
        || player.pos.y >= params.z_max && player.vel.y > 0.0
    {
        player.vel.y = 0.0;
    }
}

/// Non-kicking team stays outside the center circle until the ball leaves it.
fn clamp_receiving_team_outside_kickoff_circle(
    player: &mut Player,
    match_state: &MatchState,
    params: &SimParams,
) {
    if player.team == match_state.kickoff_team {
        return;
    }
    if !receiving_team_circle_locked(match_state) {
        return;
    }
    let min_r = params.kickoff_circle_r;
    let d = player.pos.length();
    if d < min_r && d > 1e-4 {
        let n = player.pos / d;
        player.pos = n * min_r;
        let inward = player.vel.dot(n);
        if inward < 0.0 {
            player.vel -= n * inward;
        }
    } else if d <= 1e-4 {
        // Degenerate: push to +Z edge.
        player.pos = Vec2::new(0.0, min_r);
        player.vel = Vec2::ZERO;
    }
}

/// TimePlot 17-05-04 DB14: ~34.5s full drain while sprinting with ball,
/// ~20s full regen while walking; Frida regen delays still applied.
fn tick_stamina(player: &mut Player, sprint: bool, dt: f32, params: &SimParams) {
    if sprint && player.vel.length_squared() > 0.01 {
        player.stamina = (player.stamina - dt / params.stamina_drain_full_s).max(0.0);
        player.stamina_regen_lock_left = params.stamina_regen_delay_s;
        return;
    }
    player.stamina_regen_lock_left = (player.stamina_regen_lock_left - dt).max(0.0);
    if player.stamina_regen_lock_left > 0.0 {
        return;
    }
    player.stamina = (player.stamina + dt / params.stamina_regen_full_s).min(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::ChaseBallBrain;

    #[test]
    fn idle_carrier_stand_still_gk_steals_with_full_stam() {
        use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
        use crate::player::PlayerId;
        use crate::titanium::{apply_1v1_freeze, repark_1v1_inactive, setup_1v1_harness};

        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        setup_1v1_harness(&mut world, true, 1.0);
        for player in &mut world.players {
            if player.team == TeamId::Home && player.id == PlayerId(1) {
                player.stamina = 0.85;
            } else if player.team == TeamId::Away && player.id == PlayerId(4) {
                player.stamina = 1.0;
            }
        }

        let mut gk = ChaseBallBrain;

        let mut stole = false;
        let mut min_atk_stam = 1.0_f32;
        let mut min_gk_stam = 1.0_f32;
        let mut last_atk = 1.0_f32;
        let mut last_gk = 1.0_f32;

        for tick in 0..(25.0 / FIXED_DT) as u32 {
            let (_home_api, away_api) = world.build_apis();
            let mut home_out = BrainOutput::default();
            // Attacker stands still, holds ball (no Interact = no charge/kick).
            for i in 0..4 {
                let p = &world.players[i];
                home_out.commands[i] = BrainCommand {
                    move_to: p.pos,
                    sprint: false,
                    interact: false,
                };
            }
            let mut away_out = gk.think(&away_api);
            apply_1v1_freeze(&mut home_out, &mut away_out, &world, true);
            world.step_with_commands(&home_out, &away_out, FIXED_DT);
            repark_1v1_inactive(&mut world, true);

            let atk = world
                .players
                .iter()
                .find(|p| p.team == TeamId::Home && p.id == PlayerId(1))
                .unwrap();
            let gkp = world
                .players
                .iter()
                .find(|p| p.team == TeamId::Away && p.id == PlayerId(4))
                .unwrap();
            last_atk = atk.stamina;
            last_gk = gkp.stamina;
            min_atk_stam = min_atk_stam.min(atk.stamina);
            min_gk_stam = min_gk_stam.min(gkp.stamina);

            if matches!(world.possession.carrier, Some((TeamId::Away, 4))) {
                stole = true;
                eprintln!(
                    "STEAL at t={:.2}s atk_stam={:.3} gk_stam={:.3} dist={:.2}",
                    tick as f32 * FIXED_DT,
                    atk.stamina,
                    gkp.stamina,
                    atk.pos.distance(gkp.pos)
                );
                break;
            }
        }

        assert!(
            stole,
            "standing carrier must be tackled by GK; last atk_stam={last_atk:.3} gk_stam={last_gk:.3} min_atk={min_atk_stam:.3} min_gk={min_gk_stam:.3} carrier={:?}",
            world.possession.carrier
        );
    }

    #[test]
    fn equal_stam_walk_in_tackle_steals_and_drains_carrier() {
        use crate::brain::BrainOutput;
        use crate::player::PlayerId;
        use crate::titanium::setup_1v1_harness;

        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);
        setup_1v1_harness(&mut world, true, 1.0);

        // Put carrier on top of GK with equal stam — one Interact must steal.
        let gk_pos = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(4))
            .map(|p| p.pos)
            .unwrap();
        for p in &mut world.players {
            if p.team == TeamId::Home && p.id == PlayerId(1) {
                p.pos = gk_pos + Vec2::new(-0.5, 0.0);
                p.facing = Vec2::X;
                p.stamina = 1.0;
                p.shot_charge = 0.0;
            }
            if p.team == TeamId::Away && p.id == PlayerId(4) {
                p.stamina = 1.0;
            }
        }
        world.ball.held = true;
        world.ball.pos = gk_pos + Vec2::new(-0.5, 0.0) + Vec2::X * params.hold_offset;
        world.possession.carrier = Some((TeamId::Home, 1));
        world.possession.pickup_lockout = 0.0;

        let mut home = BrainOutput::default();
        let mut away = BrainOutput::default();
        for i in 0..4 {
            home.commands[i] = BrainCommand {
                move_to: world.players[i].pos,
                sprint: false,
                interact: false,
            };
            away.commands[i] = BrainCommand {
                move_to: world.ball.pos,
                sprint: false,
                interact: i + 1 == 4,
            };
        }

        world.step_with_commands(&home, &away, FIXED_DT);

        assert_eq!(
            world.possession.carrier,
            Some((TeamId::Away, 4)),
            "equal-stam walk-in must steal for the tackler"
        );
        let carrier_stam = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Home && p.id == PlayerId(1))
            .map(|p| p.stamina)
            .unwrap();
        assert!(
            carrier_stam < 1e-4,
            "carrier must drain to 0 on equal-stam steal, got {carrier_stam}"
        );
        let gk_stam = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(4))
            .map(|p| p.stamina)
            .unwrap();
        assert!(
            gk_stam < 1e-4,
            "equal-stam tackler also drains to 0, got {gk_stam}"
        );
    }

    #[test]
    fn scripted_test1_test2_steal_in_sim() {
        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        let mut home = crate::probe_brains::Test1Brain::default();
        let mut away = crate::probe_brains::Test2Brain::default();
        let mut stole = false;
        for _ in 0..(50.0 / FIXED_DT) as u32 {
            world.step_brains(&mut home, &mut away, FIXED_DT);
            if matches!(world.possession.carrier, Some((TeamId::Away, 1))) {
                stole = true;
                break;
            }
        }
        assert!(
            stole,
            "Test1/Test2 scripted brains must produce a steal in-sim"
        );
    }

    #[test]
    fn headless_steps_without_panic() {
        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        let mut home = ChaseBallBrain;
        let mut away = ChaseBallBrain;
        for _ in 0..200 {
            world.step_brains(&mut home, &mut away, FIXED_DT);
        }
        assert!(world.match_state.clock_s > 0.0 || world.match_state.phase != MatchPhase::Kickoff);
    }

    #[test]
    fn kickoff_only_striker_of_kicking_team_may_move() {
        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Away);
        assert_eq!(world.match_state.phase, MatchPhase::Kickoff);
        let start: Vec<(TeamId, u8, Vec2)> = world
            .players
            .iter()
            .map(|p| (p.team, p.id.0, p.pos))
            .collect();
        // Everyone commanded toward the ball — graph cmds must be ignored;
        // only engine walk-in moves kicking P1.
        let toward_ball = || {
            let mut out = BrainOutput::default();
            for id in PlayerId::ALL {
                out.commands[(id.0 as usize) - 1] = BrainCommand {
                    move_to: Vec2::ZERO,
                    sprint: true,
                    interact: true,
                };
            }
            out
        };
        for _ in 0..20 {
            let home = toward_ball();
            let away = toward_ball();
            world.step_with_commands(&home, &away, FIXED_DT);
        }
        for (team, id, pos0) in start {
            let p = world
                .players
                .iter()
                .find(|p| p.team == team && p.id.0 == id)
                .unwrap();
            let moved = (p.pos - pos0).length();
            if team == TeamId::Away && id == 1 {
                assert!(
                    moved > 0.5,
                    "engine walk-in should move kicking striker; moved={moved}"
                );
            } else {
                assert!(
                    moved < 0.05,
                    "{team:?} P{id} should stay on faceoff during Kickoff; moved={moved}"
                );
            }
        }
    }

    #[test]
    fn kickoff_engine_walk_in_even_when_brains_idle() {
        // Mirrors empty Unity team (XD.txt): no useful MoveTo, striker still walks.
        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        let idle = BrainOutput::default();
        let start = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Home && p.id.0 == 1)
            .unwrap()
            .pos;
        for _ in 0..20 {
            world.step_with_commands(&idle, &idle, FIXED_DT);
        }
        let end = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Home && p.id.0 == 1)
            .unwrap()
            .pos;
        assert!(
            (end - start).length() > 0.5,
            "idle brains must still get engine kickoff walk-in; start={start:?} end={end:?}"
        );
    }

    #[test]
    fn scored_on_team_gets_next_kickoff() {
        let mut state = MatchState::with_opening_kickoff(TeamId::Away);
        assert_eq!(state.kickoff_team, TeamId::Away);
        assert_eq!(state.opening_kickoff_team, TeamId::Away);
        state.on_goal(TeamId::Home, 1.0);
        assert_eq!(state.score_away, 1);
        assert_eq!(state.kickoff_team, TeamId::Home);
        state.on_goal(TeamId::Away, 1.0);
        assert_eq!(state.score_home, 1);
        assert_eq!(state.kickoff_team, TeamId::Away);
    }

    #[test]
    fn stale_ball_whistle_flips_kickoff_without_score() {
        let mut params = SimParams::default();
        params.stale_ball_timeout_s = 5.0;
        params.stale_ball_distance_threshold_m = 2.5;
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        world.match_state.phase = MatchPhase::Play;
        // Park everyone far from the ball so body bounce can't refresh the anchor.
        for p in &mut world.players {
            p.pos = Vec2::new(if p.team == TeamId::Home { -20.0 } else { 20.0 }, 10.0);
            p.vel = Vec2::ZERO;
        }
        world.ball.pos = Vec2::new(0.4, 0.0);
        world.ball.vel = Vec2::ZERO;
        world.ball.held = false;
        world.possession.carrier = None;
        world.match_state.reset_stale_tracker(Vec2::ZERO);
        // Stand still at current spots (default MoveTo=0 would march everyone onto the ball).
        let mut home = BrainOutput::default();
        let mut away = BrainOutput::default();
        for p in &world.players {
            let i = (p.id.0.saturating_sub(1) as usize).min(3);
            match p.team {
                TeamId::Home => home.commands[i].move_to = p.pos,
                TeamId::Away => away.commands[i].move_to = p.pos,
            }
        }
        let steps = (5.0 / FIXED_DT) as i32 + 10;
        for _ in 0..steps {
            if world.match_state.phase == MatchPhase::GoalPause {
                break;
            }
            world.step_with_commands(&home, &away, FIXED_DT);
        }
        assert_eq!(
            world.match_state.phase,
            MatchPhase::GoalPause,
            "stale_idle_s={}",
            world.match_state.stale_idle_s
        );
        assert_eq!(world.match_state.kickoff_team, TeamId::Away);
        assert_eq!(world.match_state.score_home, 0);
        assert_eq!(world.match_state.score_away, 0);
        // Whistle must clear mid-play contest flags; match opening flag stays.
        assert!(world.possession.kick_exclude_shooter.is_none());
        assert_eq!(world.possession.kick_exclude_left, 0.0);
        assert!(!world.possession.kickoff_touch_done);
    }

    #[test]
    fn goal_and_kickoff_clear_possession_contest_state() {
        let mut params = SimParams::default();
        params.kickoff_delay_s = 0.0;
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        // Simulate mid-play leak that used to survive place_kickoff.
        world.possession.first_kick_done = true;
        world.possession.kickoff_touch_done = true;
        world.possession.kick_exclude_shooter = Some((TeamId::Home, 1));
        world.possession.kick_exclude_left = 2.5;
        world.possession.pickup_lockout = 0.1;
        world.possession.opening_dump_hang = true;
        world.possession.opening_hot_reclaim = true;
        world.apply_goal(EndReason::GoalHome);
        // Match-level opening script stays done; per-kickoff touch resets.
        assert!(world.possession.first_kick_done);
        assert!(!world.possession.kickoff_touch_done);
        assert!(world.possession.kick_exclude_shooter.is_none());
        // Drain GoalPause → place_kickoff.
        world.step_with_commands(&BrainOutput::default(), &BrainOutput::default(), FIXED_DT);
        assert_eq!(world.match_state.phase, MatchPhase::Kickoff);
        assert!(world.possession.first_kick_done);
        assert!(!world.possession.kickoff_touch_done);
        assert!(world.possession.kick_exclude_shooter.is_none());
        assert_eq!(world.possession.pickup_lockout, 0.0);
        assert!(!world.possession.opening_dump_hang);
        assert!(!world.possession.opening_hot_reclaim);
        // Away scored into Home's net → Home (conceded) restarts.
        assert_eq!(world.match_state.kickoff_team, TeamId::Home);
    }

    #[test]
    fn player_stays_on_pitch_aabb() {
        let params = SimParams::default();
        let mut p = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::new(50.0, 40.0),
            vel: Vec2::new(5.0, 5.0),
            facing: Vec2::ONE,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        clamp_player_to_pitch(&mut p, &params);
        assert!(p.pos.x <= params.x_max + 1e-4);
        assert!(p.pos.y <= params.z_max + 1e-4);
        assert_eq!(p.vel.x, 0.0);
        assert_eq!(p.vel.y, 0.0);
    }

    #[test]
    fn player_can_walk_into_goal_mouth() {
        let params = SimParams::default();
        let mut p = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::new(40.5, 0.0),
            vel: Vec2::X,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        clamp_player_to_pitch(&mut p, &params);
        assert!(
            p.pos.x > params.x_max + 0.2,
            "mouth must open into the net; got x={}",
            p.pos.x
        );
        // Outside the mouth, still clamped to the pitch AABB.
        p.pos = Vec2::new(40.5, params.goal_half_width + 1.0);
        clamp_player_to_pitch(&mut p, &params);
        assert!(p.pos.x <= params.x_max + 1e-4);
    }

    #[test]
    fn carrying_ball_past_goal_line_scores() {
        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);
        world.match_state.phase = MatchPhase::Play;

        let carrier = world
            .players
            .iter_mut()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(1))
            .expect("away p1");
        carrier.pos = Vec2::new(params.x_max, 0.0);
        carrier.facing = Vec2::X;
        carrier.vel = Vec2::ZERO;
        world.possession.carrier = Some((TeamId::Away, 1));
        // Opening dump biases gate on !first_kick_done — this is a mid-play
        // carry-in score probe, not the opening Away charge.
        world.possession.first_kick_done = true;
        world.ball.held = true;
        world.ball.pos = world
            .players
            .iter()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(1))
            .unwrap()
            .hold_pos(params.hold_offset);

        // Hold everyone still so the mover doesn't drag the carrier off the line.
        let stay = |team: TeamId, w: &MatchWorld| {
            let mut out = BrainOutput::default();
            for id in PlayerId::ALL {
                let p = w
                    .players
                    .iter()
                    .find(|p| p.team == team && p.id == id)
                    .unwrap();
                out.commands[(id.0 as usize) - 1] = BrainCommand {
                    move_to: p.pos,
                    sprint: false,
                    interact: false,
                };
            }
            out
        };
        let home = stay(TeamId::Home, &world);
        let away = stay(TeamId::Away, &world);
        world.step_with_commands(&home, &away, FIXED_DT);

        assert!(
            world.match_state.score_home >= 1,
            "carry-in should score; ball={:?} phase={:?}",
            world.ball.pos,
            world.match_state.phase
        );
        assert_eq!(world.match_state.phase, MatchPhase::GoalPause);
    }

    /// Corner / wall-slam should not blow the FIXED_DT budget (viewer "OVER").
    /// Nested Function calls are legal as of Unity v0.63; AIA itself still has zero.
    #[test]
    fn corner_slam_tick_cost_stays_under_budget() {
        use crate::graph::load_team_graph;
        use crate::graph_vm::RuntimeBrain;
        use std::path::PathBuf;
        use std::time::Instant;

        let aia = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join("AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/AIA.txt");
        if !aia.exists() {
            eprintln!("skip: no AIA.txt at {aia:?}");
            return;
        }
        let graph = load_team_graph(&aia).expect("load AIA");
        let cached = RuntimeBrain::compile_cached(graph);
        let mut home = RuntimeBrain::from_cached(cached.clone());
        let mut away = RuntimeBrain::from_cached(cached);

        let params = SimParams::default();
        let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
        // Leave kickoff quickly.
        for _ in 0..30 {
            world.step_brains(&mut home, &mut away, FIXED_DT);
        }

        let mut time_mid = Vec::new();
        let mut time_corner = Vec::new();
        let mut time_api = Vec::new();

        for i in 0..120 {
            // Alternate: free midfield ball vs hard corner slam (can't leave stadium —
            // walls clamp; this is the "kick out" break attempt).
            if i % 2 == 0 {
                world.possession.carrier = None;
                world.ball.held = false;
                world.ball.pos = Vec2::ZERO;
                world.ball.vel = Vec2::new(8.0, 3.0);
                world.ball.height = world.params.ball_rest_height;
                world.ball.vel_y = 0.0;
            } else {
                world.possession.carrier = None;
                world.ball.held = false;
                world.ball.pos = Vec2::new(world.params.x_max - 0.2, world.params.z_max - 0.2);
                world.ball.vel = Vec2::new(30.0, 30.0);
                world.ball.height = world.params.ball_rest_height + 2.0;
                world.ball.vel_y = 5.0;
            }

            let a0 = Instant::now();
            let (home_api, away_api) = world.build_apis();
            let api_ms = a0.elapsed().as_secs_f32() * 1000.0;
            time_api.push(api_ms);

            let t0 = Instant::now();
            let home_out = home.think(&home_api);
            let away_out = away.think(&away_api);
            world.step_with_commands(&home_out, &away_out, FIXED_DT);
            let tick_ms = t0.elapsed().as_secs_f32() * 1000.0;
            if i % 2 == 0 {
                time_mid.push(tick_ms);
            } else {
                time_corner.push(tick_ms);
            }
        }

        let pct = |xs: &mut [f32], p: f32| {
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let i = ((xs.len() as f32 - 1.0) * p).round() as usize;
            xs[i]
        };
        let mid_p50 = pct(&mut time_mid, 0.5);
        let mid_p95 = pct(&mut time_mid, 0.95);
        let cor_p50 = pct(&mut time_corner, 0.5);
        let cor_p95 = pct(&mut time_corner, 0.95);
        let api_p95 = pct(&mut time_api, 0.95);
        eprintln!(
            "corner_slam mid p50={mid_p50:.2} p95={mid_p95:.2} | \
             corner p50={cor_p50:.2} p95={cor_p95:.2} | api p95={api_p95:.2} ms \
             (budget {:.1})",
            FIXED_DT * 1000.0
        );

        // Corner slam must not be dramatically worse than midfield (same brain work).
        assert!(
            cor_p95 < FIXED_DT * 1000.0 * 0.9,
            "corner p95 {cor_p95:.2}ms exceeds 90% of FIXED_DT budget"
        );
        assert!(
            cor_p95 < mid_p95 * 3.0 + 1.0,
            "corner p95 {cor_p95:.2} much worse than mid p95 {mid_p95:.2}"
        );
    }
}
