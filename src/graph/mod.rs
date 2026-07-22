//! AIComp team-graph loader + tick evaluator (MVP).
//!
//! Implements constants, float math (incl. Power), SoccerGet*, RelativePosition,
//! and SoccerController1–4. Variables / functions / sensors come later.

mod dropdowns;
mod eval;
mod load;
mod value;

pub use eval::GraphBrain;
pub use load::{load_team_graph, TeamGraph};
pub use value::GraphValue;
