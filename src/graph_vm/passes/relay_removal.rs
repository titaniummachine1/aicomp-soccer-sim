//! RelayRemoval — empty until prior passes land one-by-one.

use crate::graph_vm::ir::LoweredIR;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct RelayRemoval;

impl Pass for RelayRemoval {
    fn name(&self) -> &'static str {
        "RelayRemoval"
    }
    fn run(&self, _ir: &mut LoweredIR) {}
}
