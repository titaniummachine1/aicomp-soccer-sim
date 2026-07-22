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
use crate::ball::{goal_at, resolve_player_bodies, step_free_ball, Ball, EndReason};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::match_state::{
    kickoff_control_allowed, place_kickoff, receiving_team_circle_locked, MatchPhase, MatchState,
};
use crate::params::SimParams;
use crate::player::{
    faceoff_world, kickoff_facing, step_mover, Player, PlayerId, SimpleMover,
};
use crate::possession::{
    apply_interact, sync_held_ball, tick_possession_timers, Possession,
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
                );
                self.possession.carrier = None;
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
            // End kickoff on first touch, ball leaving center, or long fallback.
            let first_touch = self.possession.carrier.is_some()
                || self.ball.vel.length() > 0.35
                || self.ball.pos.length() > 0.75;
            if first_touch || self.match_state.phase_timer > 8.0 {
                self.match_state.phase = MatchPhase::Play;
                self.match_state.reset_stale_tracker(self.ball.pos);
            }
        } else {
            self.match_state.clock_s += dt;
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
            let cmd = bias_away_defender_opening_hold(
                &self.players[i],
                cmd,
                &self.match_state,
            );
            let is_carrier = matches!(
                self.possession.carrier,
                Some((t, id)) if t == team && id == self.players[i].id.0
            );
            let opp_has_ball = matches!(
                self.possession.carrier,
                Some((t, _)) if t != team
            );
            let face_aim = if is_carrier {
                let origin = self.players[i].pos;
                let is_home = team == TeamId::Home;
                let blockers: Vec<Vec2> = self
                    .players
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, p)| p.pos)
                    .collect();
                let blocker_r = self.params.body_radius * 1.5;
                first_clear_dir(
                    origin,
                    is_home,
                    &blockers,
                    blocker_r,
                    12.0,
                    true,
                    true,
                    self.params.x_min,
                    self.params.x_max,
                    self.params.z_min,
                    self.params.z_max,
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
                dt,
            );
            clamp_player_to_pitch(&mut self.players[i], &self.params);
            clamp_receiving_team_outside_kickoff_circle(
                &mut self.players[i],
                &self.match_state,
                &self.params,
            );
            tick_stamina(&mut self.players[i], cmd.sprint, dt);
            if let Some(drain) = apply_interact(
                &mut self.players[i],
                &mut self.ball,
                &mut self.possession,
                cmd,
                &self.params,
                dt,
                carrier_stam,
                carrier_charge,
            ) {
                if let Some(c) = self
                    .players
                    .iter_mut()
                    .find(|p| p.team == drain.team && p.id.0 == drain.id)
                {
                    c.stamina = (c.stamina - drain.drain).max(0.0);
                }
            }
        }

        sync_held_ball(
            &mut self.ball,
            &self.players,
            &self.possession,
            self.params.hold_offset,
        );

        // Always verify goal volume after hold sync — a carrier walking the
        // ball past the line (hold offset past x_max) must score too.
        let scored = if self.ball.held {
            goal_at(self.ball.pos, &self.params)
        } else {
            let scored = step_free_ball(&mut self.ball, &self.params, dt);
            resolve_player_bodies(&mut self.ball, &self.players, &self.params);
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
            self.match_state.on_whistle();
            self.possession.carrier = None;
            self.ball.held = false;
            self.ball.vel = Vec2::ZERO;
            self.ball.pos = Vec2::ZERO;
        }
    }

    fn apply_goal(&mut self, scored: EndReason) {
        match scored {
            EndReason::GoalHome => {
                self.match_state.on_goal(TeamId::Home);
                self.possession.carrier = None;
                self.ball.held = false;
                self.ball.vel = Vec2::ZERO;
            }
            EndReason::GoalAway => {
                self.match_state.on_goal(TeamId::Away);
                self.possession.carrier = None;
                self.ball.held = false;
                self.ball.vel = Vec2::ZERO;
            }
            EndReason::None => {}
        }
    }

    pub fn step_brains<H: TeamBrain, A: TeamBrain>(
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
    if kickoff_control_allowed(player.team, player.pos, match_state, params) {
        cmd
    } else {
        BrainCommand {
            move_to: player.pos,
            sprint: false,
            interact: false,
        }
    }
}

/// While Away opening-chase is suppressed, pin Defender MoveTo.x near the
/// real State0 skirt (~5.5) so they don't march to 6+BallX on the C-lane.
fn bias_away_defender_opening_hold(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
) -> BrainCommand {
    if player.team != TeamId::Away
        || player.id.0 != 3
        || !match_state.kickoff_suppress_away_team_side
    {
        return cmd;
    }
    BrainCommand {
        move_to: Vec2::new(5.5, cmd.move_to.y),
        sprint: cmd.sprint,
        interact: cmd.interact,
    }
}

/// While circle-locked (Kickoff phase), walk toward the ring ∩ axis.
fn project_move_outside_kickoff_circle(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    params: &SimParams,
) -> BrainCommand {
    if player.team == match_state.kickoff_team || !receiving_team_circle_locked(match_state) {
        return cmd;
    }
    let min_r = params.kickoff_circle_r;
    // Walk toward the ring ∩ attacking-direction axis (Away +X, Home −X).
    let sx = if player.pos.x >= 0.0 { 1.0 } else { -1.0 };
    let move_to = Vec2::new(sx * min_r, 0.0);
    BrainCommand {
        move_to,
        sprint: cmd.sprint,
        interact: cmd.interact,
    }
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

/// Community stamina: ~30s full drain sprint, ~15s full regen idle. Cheap linear.
/// TODO: map Frida stamina (max=100 consume=0.15 regen=5 regenDelay=1 tackleRegenDelay=1.5)
/// from `worldcupv0.5/_modding_dumps/soccer_mover_stamina.json` once per-second semantics
/// on 0..1 sim stamina are confirmed — see `bevy_sim_params_v05.json` stamina_frida.
fn tick_stamina(player: &mut Player, sprint: bool, dt: f32) {
    if sprint && player.vel.length_squared() > 0.01 {
        player.stamina = (player.stamina - dt / 30.0).max(0.0);
    } else {
        player.stamina = (player.stamina + dt / 15.0).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::ChaseBallBrain;

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
    fn scored_on_team_gets_next_kickoff() {
        let mut state = MatchState::with_opening_kickoff(TeamId::Away);
        assert_eq!(state.kickoff_team, TeamId::Away);
        assert_eq!(state.opening_kickoff_team, TeamId::Away);
        state.on_goal(TeamId::Home);
        assert_eq!(state.score_away, 1);
        assert_eq!(state.kickoff_team, TeamId::Home);
        state.on_goal(TeamId::Away);
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
}
