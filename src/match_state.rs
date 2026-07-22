//! Match FSM: kickoff → play → goal → kickoff.

use bevy::prelude::*;

use crate::ball::Ball;
use crate::brain::TeamId;
use crate::params::SimParams;
use crate::player::{default_facing, faceoff_world, Player};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    Kickoff,
    Play,
    GoalPause,
}

#[derive(Resource, Debug, Clone)]
pub struct MatchState {
    pub phase: MatchPhase,
    pub clock_s: f32,
    pub duration_s: f32,
    pub score_home: u32,
    pub score_away: u32,
    pub kickoff_team: TeamId,
    pub opening_kickoff_team: TeamId,
    pub phase_timer: f32,
}

impl Default for MatchState {
    /// Deterministic default (Home opens). Prefer [`MatchState::new_match`] for live games.
    fn default() -> Self {
        Self::with_opening_kickoff(TeamId::Home)
    }
}

impl MatchState {
    /// New match: opening kickoff is random; after goals, scored-on team restarts.
    pub fn new_match() -> Self {
        Self::with_opening_kickoff(random_opening_kickoff())
    }

    pub fn with_opening_kickoff(opening: TeamId) -> Self {
        Self {
            phase: MatchPhase::Kickoff,
            clock_s: 0.0,
            duration_s: 180.0,
            score_home: 0,
            score_away: 0,
            kickoff_team: opening,
            opening_kickoff_team: opening,
            phase_timer: 0.0,
        }
    }

    /// `scored_on` conceded — they receive the next kickoff (AIComp rule).
    pub fn on_goal(&mut self, scored_on: TeamId) {
        match scored_on {
            TeamId::Home => self.score_away += 1,
            TeamId::Away => self.score_home += 1,
        }
        self.phase = MatchPhase::GoalPause;
        self.phase_timer = 1.0;
        self.kickoff_team = scored_on;
    }
}

fn random_opening_kickoff() -> TeamId {
    use std::time::{SystemTime, UNIX_EPOCH};
    let bit = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        & 1;
    if bit == 0 {
        TeamId::Home
    } else {
        TeamId::Away
    }
}

pub fn place_kickoff(ball: &mut Ball, players: &mut [Player], kickoff_team: TeamId) {
    ball.pos = Vec2::ZERO;
    ball.vel = Vec2::ZERO;
    ball.held = false;
    for p in players.iter_mut() {
        p.pos = faceoff_world(p.team, p.id, kickoff_team);
        p.vel = Vec2::ZERO;
        p.facing = default_facing(p.team);
        p.shot_charge = 0.0;
    }
}

pub fn kickoff_control_allowed(
    team: TeamId,
    player_pos: Vec2,
    match_state: &MatchState,
    params: &SimParams,
) -> bool {
    match match_state.phase {
        MatchPhase::Play => true,
        MatchPhase::GoalPause => false,
        MatchPhase::Kickoff => {
            if team == match_state.kickoff_team {
                true
            } else {
                player_pos.length() >= params.kickoff_circle_r
            }
        }
    }
}
