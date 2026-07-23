//! Fully 2D AIComp Soccer close-copy.
//!
//! **Cheap core:** `world::MatchWorld` — fixed-dt, no navmesh, no player↔ball
//! body shove (Interact-only possession), pitch AABB walk clamp. Bevy
//! viewer is optional; batch sims should call `MatchWorld::step_brains` only.
//!
//! Pitch `Vec2`: `.x` = world X (goals), `.y` = world Z (sidelines).

pub mod api;
pub mod ball;
pub mod batch;
pub mod brain;
pub mod debug_draw;
pub mod graph;
pub mod graph_vm;
pub mod keypress;
pub mod match_state;
pub mod params;
pub mod player;
pub mod possession;
pub mod predict;
pub mod probe_brains;
pub mod scenario;
pub mod team_threads;
pub mod titanium;
pub mod timeplot;
pub mod world;
pub mod train;

pub use ball::{goal_at, resolve_player_bodies, step_free_ball, Ball, EndReason};
pub use brain::{BrainCommand, BrainOutput, ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
pub use graph::{load_team_graph, GraphBrain, TeamGraph};
pub use match_state::{MatchPhase, MatchState};
pub use params::SimParams;
pub use player::{project_hold_into_playable, Player, PlayerId, SimpleMover};
pub use possession::Possession;
pub use scenario::{evaluate_scenario1, MatchScenario, Scenario1Outcome};
pub use titanium::TitaniumBrain;
pub use probe_brains::{PerfectControllerBrain, Test1Brain, Test2Brain};
pub use timeplot::TimePlotRecorder;
pub use world::{MatchWorld, FIXED_DT};