//! AIComp Soccer API surface for the 2D sim.
//!
//! Goal: every `SoccerGet*` / `SoccerController` a graph can touch has a
//! matching read/write here, team-relative (Home graph vs Away graph).
//! Positions are pitch-plane Vec2 (x, z). Unity Y is unused (visual-only).
//!
//! Null Vector3 → `None` (same as AIComp null).

mod clear;
mod labels;
mod snapshot;

#[cfg(test)]
mod coverage_test;

pub use clear::{
    first_clear_dir, CLEAR_ORDER_AWAY, CLEAR_ORDER_HOME, SensorDir,
    SPHERECAST_DISTANCE, SPHERECAST_RADIUS,
};
pub use labels::*;
pub use snapshot::{build_team_api, WorldSensors};

use bevy::prelude::*;

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::player::PlayerId;

/// One team's view of the match — what a loaded `.txt` graph would read.
#[derive(Debug, Clone)]
pub struct TeamApi {
    pub team: TeamId,
    pub bools: std::collections::HashMap<&'static str, bool>,
    pub floats: std::collections::HashMap<&'static str, f32>,
    /// Transforms as pitch positions (x, z). Always present for known labels.
    pub transforms: std::collections::HashMap<&'static str, Vec2>,
    /// Vector3 getters; `None` = AIComp null.
    pub vectors: std::collections::HashMap<&'static str, Option<Vec2>>,
}

impl TeamApi {
    pub fn get_bool(&self, label: &str) -> Option<bool> {
        self.bools.get(label).copied()
    }

    pub fn get_float(&self, label: &str) -> Option<f32> {
        self.floats.get(label).copied()
    }

    pub fn get_transform(&self, label: &str) -> Option<Vec2> {
        self.transforms.get(label).copied()
    }

    pub fn get_vector3(&self, label: &str) -> Option<Option<Vec2>> {
        self.vectors.get(label).copied()
    }
}

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
