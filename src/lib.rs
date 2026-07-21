//! Fully 2D AIComp Soccer close-copy for rapid offline testing.
//!
//! Pitch `Vec2`: `.x` = world X (goals ±X), `.y` = world Z (sidelines ±Z).
//! Unity 3D / Y is visual-only — not simulated here.
//!
//! Top-down view only. Players = discs at nav-agent radius; facing/hold spot
//! = small circle in look direction (BallHoldLocation).

pub mod ball;
pub mod brain;
pub mod match_state;
pub mod params;
pub mod player;
pub mod possession;

pub use ball::{step_free_ball, Ball, EndReason};
pub use brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
pub use match_state::{MatchPhase, MatchState};
pub use params::SimParams;
pub use player::{Player, PlayerId, SimpleMover};
pub use possession::Possession;
