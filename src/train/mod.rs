//! Square-weights NN brain harness.
//!
//! Compiled only with `--features nn_train` (off by default). Gating is for
//! Bevy cold-build isolation — this module is not deleted or broken.

pub mod brain_harness;
pub use brain_harness::TrainedBrain;