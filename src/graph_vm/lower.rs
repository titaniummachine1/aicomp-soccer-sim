//! Lower LoadedGraph / TeamGraph → LoweredIR (stub).

use crate::graph::TeamGraph;
use crate::graph_vm::ir::LoweredIR;

#[derive(Debug, Default)]
pub struct Lowerer;

impl Lowerer {
    pub fn lower(&self, _graph: &TeamGraph) -> LoweredIR {
        LoweredIR::new()
    }
}
