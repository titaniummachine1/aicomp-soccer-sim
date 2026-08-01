//! Square-weights NN brain harness.
//!
//! Compiled only with `--features nn_train` (off by default). Gating is for
//! Bevy cold-build isolation — this module is not deleted or broken.

pub mod brain_harness;
pub use brain_harness::{
    NetWeights, NetGradients, Activations, TrainedBrain,
    INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM,
    VALUES_PER_PLAYER, COMPASS_DIRS, SOFTMAX_GROUPS,
    SPRINT_OFFSET, TACKLE_OFFSET, TEAM_OUTPUT_START,
    extract_features,
};