//! Fully 2D AIComp Soccer close-copy.
//!
//! **Cheap core:** `world::MatchWorld` — fixed-dt, no navmesh, no player↔ball
//! collision, pitch AABB walk clamp (no goal entry for now). Bevy viewer is
//! optional; batch sims should call `MatchWorld::step_brains` only.
//!
//! Pitch `Vec2`: `.x` = world X (goals), `.y` = world Z (sidelines).

pub mod api;
pub mod ball;
pub mod brain;
pub mod graph;
pub mod match_state;
pub mod params;
pub mod player;
pub mod possession;
pub mod team_threads;
pub mod world;

pub use ball::{step_free_ball, Ball, EndReason};
pub use brain::{BrainCommand, BrainOutput, ChaseBallBrain, TeamBrain, TeamId};
pub use graph::{load_team_graph, GraphBrain, TeamGraph};
pub use match_state::{MatchPhase, MatchState};
pub use params::SimParams;
pub use player::{Player, PlayerId, SimpleMover};
pub use possession::Possession;
pub use world::{MatchWorld, FIXED_DT};
