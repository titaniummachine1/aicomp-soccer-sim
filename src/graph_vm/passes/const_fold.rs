//! ConstFold — empty until O0 verify is green.

use crate::graph_vm::ir::LoweredIR;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct ConstFold;

impl Pass for ConstFold {
    fn name(&self) -> &'static str {
        "ConstFold"
    }
    fn run(&self, _ir: &mut LoweredIR) {}
}
