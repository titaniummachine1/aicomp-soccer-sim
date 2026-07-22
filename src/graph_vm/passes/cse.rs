//! CSE — Pure ops only, same cache scope.

use crate::graph_vm::ir::LoweredIR;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct Cse;

impl Pass for Cse {
    fn name(&self) -> &'static str {
        "CSE"
    }
    fn run(&self, _ir: &mut LoweredIR) {}
}
