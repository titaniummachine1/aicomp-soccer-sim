//! Fusion — O1 Const→Math only when enabled.

use crate::graph_vm::ir::LoweredIR;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct Fusion;

impl Pass for Fusion {
    fn name(&self) -> &'static str {
        "Fusion"
    }
    fn run(&self, _ir: &mut LoweredIR) {}
}
