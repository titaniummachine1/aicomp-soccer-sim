//! Cheap headless match world — no navmesh; loose ball bounces off player bodies.
//!
//! Design for 10⁵–10⁶ sims:
//! - Fixed-dt step, plain structs (Bevy is only the interactive viewer)
//! - Ball is the only sliding physics body (+ circle bounce vs players when loose)
//! - Players = accel movers + axis-aligned walk limits (pitch AABB only)
//! - Possession via interact radius only
//! - Brains run inline on the sim thread (no per-match worker threads in batch)
//!
//! Goal entry / post-corner pathfinding: deferred. If a move target is inside
//! the goal volume we'd need a collision-circle sweep vs posts+walls (go
//! straight if clear, else pathfind). For now assume nobody needs to walk
//! into the goals — clamp players to the playable AABB.

use bevy::prelude::Vec2;

use crate::api::{build_team_api, first_clear_dir, TeamApi, WorldSensors};
use crate::ball::{goal_at, held_goal_at, resolve_player_bodies, step_free_ball, Ball, EndReason};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::match_state::{
    kickoff_control_allowed, place_kickoff, receiving_team_circle_locked, MatchPhase, MatchState,
};
use crate::params::SimParams;
use crate::player::{faceoff_world, kickoff_facing, step_mover, Player, PlayerId, SimpleMover};
use crate::possession::{apply_interact, sync_held_ball, tick_possession_timers, Possession};

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
            let cmd = filter_kickoff(&self.players[i], raw, &self.match_state, &self.params);
            let cmd = project_move_outside_kickoff_circle(
                &self.players[i],
                cmd,
                &self.match_state,
                &self.params,
            );
            let cmd =
                bias_receiving_defender_opening_hold(&self.players[i], cmd, &self.match_state);
            // Opening Away dump: keep carrier on Unity's mostly-west lane while
            // charge facing stays Clear F (face_aim bias). Raw Clear often falls
            // to A (+Z) and flips O1/Ball.Z vs DB35.
            let cmd = bias_away_opening_carrier_dump_lane(&self.players[i], cmd, &self.possession);
            // DB33: Home T2 was tackling Away mid-charge (~1.8s) before the
            // opening dump. Unity lets Away finish Charge→release first; Home
            // then claims the loose ball. Strip receiving interact until the
            // first kick lands (live flag so same-tick post-kick reclaim works).
            let cmd = bias_receiving_opening_no_tackle(
                &self.players[i],
                cmd,
                &self.match_state,
                self.possession.first_kick_done,
            );
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
            self.params.hold_offset,
            self.params.ball_rest_height,
        );

        // Always verify goal volume after hold sync.
        // Held: require carrier body on/over the goal line (hold_offset alone
        // must not score — players are clamped to the goal line AABB).
        let scored = if self.ball.held {
            let carrier_pos = self.possession.carrier.and_then(|(t, id)| {
                self.players
                    .iter()
                    .find(|p| p.team == t && p.id.0 == id)
                    .map(|p| p.pos)
            });
            carrier_pos
                .map(|cp| held_goal_at(self.ball.pos, cp, &self.params))
                .unwrap_or(EndReason::None)
        } else {
            let scored = step_free_ball(&mut self.ball, &self.params, dt);
            // Free ball at kickoff stays planted until Interact pickup — body
            // shoves were ending Kickoff early and scattering Ball.X/Z (DB33).
            if self.match_state.phase != MatchPhase::Kickoff {
                resolve_player_bodies(&mut self.ball, &self.players, &self.params);
            }
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
            self.possession.carrier = None;
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
                self.possession.carrier = None;
                self.ball.held = false;
                self.ball.vel = Vec2::ZERO;
                self.ball.vel_y = 0.0;
                self.ball.height = self.params.ball_rest_height;
            }
            EndReason::GoalAway => {
                self.match_state
                    .on_goal(TeamId::Away, self.params.kickoff_delay_s);
                self.possession.carrier = None;
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
) -> BrainCommand {
    if kickoff_control_allowed(player.team, player.id, match_state, params) {
        cmd
    } else {
        BrainCommand {
            move_to: player.pos,
            sprint: false,
            interact: false,
        }
    }
}

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

/// Block receiving-team tackles on the opening carrier until the first kick
/// has been released (Playmaker T2 was poaching before Away's dump).
fn bias_receiving_opening_no_tackle(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    first_kick_done: bool,
) -> BrainCommand {
    if first_kick_done
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

/// Pitch walk box only. Ball still uses open goal mouths for scoring; players
/// do not enter the net (no post sweep / pathfind in batch sims).
pub fn clamp_player_to_pitch(player: &mut Player, params: &SimParams) {
    player.pos.x = player.pos.x.clamp(params.x_min, params.x_max);
    player.pos.y = player.pos.y.clamp(params.z_min, params.z_max);

    if player.pos.x <= params.x_min && player.vel.x < 0.0
        || player.pos.x >= params.x_max && player.vel.x > 0.0
    {
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
        // Everyone commanded toward the ball — filter must freeze non-strikers.
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
                assert!(moved > 0.5, "kicking striker should walk in; moved={moved}");
            } else {
                assert!(
                    moved < 0.05,
                    "{team:?} P{id} should stay on faceoff during Kickoff; moved={moved}"
                );
            }
        }
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
    fn player_cannot_enter_goal_past_line() {
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
    /// Maia nested-Function Null is unrelated — AIA itself has zero nested calls.
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
