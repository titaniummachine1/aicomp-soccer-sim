//! Cheap headless match world — no navmesh, no player↔ball collision.
//!
//! Design for 10⁵–10⁶ sims:
//! - Fixed-dt step, plain structs (Bevy is only the interactive viewer)
//! - Ball is the only sliding physics body
//! - Players = accel movers + axis-aligned walk limits (pitch AABB only)
//! - Possession via interact radius only
//! - Brains run inline on the sim thread (no per-match worker threads in batch)
//!
//! Goal entry / post-corner pathfinding: deferred. If a move target is inside
//! the goal volume we'd need a collision-circle sweep vs posts+walls (go
//! straight if clear, else pathfind). For now assume nobody needs to walk
//! into the goals — clamp players to the playable AABB.

use bevy::prelude::Vec2;

use crate::api::{build_team_api, TeamApi, WorldSensors};
use crate::ball::{step_free_ball, Ball, EndReason};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::match_state::{
    kickoff_control_allowed, place_kickoff, MatchPhase, MatchState,
};
use crate::params::SimParams;
use crate::player::{
    default_facing, faceoff_world, step_mover, Player, PlayerId, SimpleMover,
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
                    facing: default_facing(team),
                    stamina: 1.0,
                    shot_charge: 0.0,
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
            }
            return;
        }

        if self.match_state.phase == MatchPhase::Kickoff {
            self.match_state.phase_timer += dt;
            if self.match_state.phase_timer > 0.5 {
                self.match_state.phase = MatchPhase::Play;
            }
        } else {
            self.match_state.clock_s += dt;
        }

        tick_possession_timers(&mut self.possession, dt);

        for player in &mut self.players {
            let raw = match player.team {
                TeamId::Home => home.for_player(player.id),
                TeamId::Away => away.for_player(player.id),
            };
            let cmd = filter_kickoff(player, raw, &self.match_state, &self.params);
            // TODO(goal-entry): if cmd.move_to is inside a goal volume, sweep
            // collision circle vs posts+walls; pathfind only if blocked.
            // For now targets are assumed on-pitch; clamp keeps us out of goals.
            step_mover(player, &self.mover, cmd.move_to, cmd.sprint, dt);
            clamp_player_to_pitch(player, &self.params);
            tick_stamina(player, cmd.sprint, dt);
            apply_interact(
                player,
                &mut self.ball,
                &mut self.possession,
                cmd,
                &self.params,
                dt,
            );
        }

        sync_held_ball(
            &mut self.ball,
            &self.players,
            &self.possession,
            self.params.hold_offset,
        );

        if !self.ball.held {
            match step_free_ball(&mut self.ball, &self.params, dt) {
                EndReason::GoalHome => {
                    self.match_state.on_goal(TeamId::Home);
                    self.possession.carrier = None;
                }
                EndReason::GoalAway => {
                    self.match_state.on_goal(TeamId::Away);
                    self.possession.carrier = None;
                }
                EndReason::None => {}
            }
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

/// Community stamina: ~30s full drain sprint, ~15s full regen idle. Cheap linear.
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
        };
        clamp_player_to_pitch(&mut p, &params);
        assert!(p.pos.x <= params.x_max + 1e-4);
    }
}
