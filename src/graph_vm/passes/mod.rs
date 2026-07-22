//! Optimization passes — one at a time after O0 verify is green.

pub mod const_fold;
pub mod cse;
pub mod fusion;
pub mod relay_removal;

use crate::graph_vm::ir::LoweredIR;

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
}
