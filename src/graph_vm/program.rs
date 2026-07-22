//! Immutable RuntimeProgram (Graph ABI v1).

use std::sync::Arc;

use crate::graph_vm::opcode::Instruction;

pub const GRAPH_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    pub abi_version: u32,
    pub ir_version: u32,
    pub program_hash: u64,
    pub register_count: u32,
    pub variable_count: u32,
    pub ops: Arc<[Instruction]>,
}

impl RuntimeProgram {
    pub fn empty() -> Self {
        Self {
            abi_version: GRAPH_ABI_VERSION,
            ir_version: crate::graph_vm::ir::LOWERED_IR_VERSION,
            program_hash: 0,
            register_count: 0,
            variable_count: 0,
            ops: Arc::from([]),
        }
    }
}

pub trait Backend {
    fn execute(
        &mut self,
        program: &RuntimeProgram,
        ctx: &mut crate::graph_vm::context::ExecutionContext,
    );
}
