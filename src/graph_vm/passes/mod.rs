//! Optimization passes — one at a time after O0 verify is green.

pub mod const_fold;
pub mod cse;
pub mod fusion;
pub mod relay_removal;

use crate::graph_vm::ir::LoweredIR;

pub use const_fold::ConstFold;
pub use relay_removal::RelayRemoval;

pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, ir: &mut LoweredIR);
}

#[derive(Default)]
pub struct PassManager {
    pub passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn run_all(&self, ir: &mut LoweredIR) {
        for p in &self.passes {
            p.run(ir);
        }
    }

    /// O1 pipeline: ConstFold followed by RelayRemoval.
    ///
    /// CSE and Fusion land later, in that order.
    pub fn o1() -> Self {
        Self {
            passes: vec![Box::new(ConstFold), Box::new(RelayRemoval)],
        }
    }

    /// Compatibility alias for the renamed O1 pipeline.
    pub fn o1_const_fold_only() -> Self {
        Self::o1()
    }
}
