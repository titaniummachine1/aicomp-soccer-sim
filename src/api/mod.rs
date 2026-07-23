//! AIComp Soccer API surface for the 2D sim.
//!
//! Goal: every `SoccerGet*` / `SoccerController` a graph can touch has a
//! matching read/write here, team-relative (Home graph vs Away graph).
//! Positions are pitch-plane Vec2 (x, z). Unity Y is unused (visual-only).
//!
//! Null Vector3 → `None` (same as AIComp null).
//!
//! RuntimeBrain LoadApi uses dense `u16` catalog ids (see [`dense`]), not
//! per-op string HashMap lookups.

mod clear;
mod dense;
mod labels;
mod snapshot;

#[cfg(test)]
mod coverage_test;

pub use clear::{
    bias_away_opening_clear_f, first_clear_dir, hit_tags, CLEAR_ORDER_AWAY,
    CLEAR_ORDER_HOME, SensorDir, SPHERECAST_DISTANCE, SPHERECAST_RADIUS,
};
pub use dense::{
    bool_index, float_index, transform_index, vector_index, DenseTeamApi, UNKNOWN_ID,
};
pub use labels::*;
pub use snapshot::{build_team_api, WorldSensors};

use bevy::prelude::*;

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::player::PlayerId;

/// One team's view of the match — dense SoccerGet catalogs (int-indexed).
pub type TeamApi = DenseTeamApi;

/// Controller outputs for players 1–4 (graph write side).
#[derive(Debug, Clone, Default)]
pub struct TeamControllers {
    pub commands: [BrainCommand; 4],
}

impl TeamControllers {
    pub fn set(&mut self, player: PlayerId, cmd: BrainCommand) {
        let i = (player.0.saturating_sub(1) as usize).min(3);
        self.commands[i] = cmd;
    }

    pub fn to_brain_output(&self) -> BrainOutput {
        BrainOutput {
            commands: self.commands,
        }
    }
}

/// Both teams' API snapshots for one tick (each team thread can read its half).
#[derive(Resource, Debug, Clone, Default)]
pub struct MatchApi {
    pub home: Option<TeamApi>,
    pub away: Option<TeamApi>,
}

impl MatchApi {
    pub fn for_team(&self, team: TeamId) -> Option<&TeamApi> {
        match team {
            TeamId::Home => self.home.as_ref(),
            TeamId::Away => self.away.as_ref(),
        }
    }
}
