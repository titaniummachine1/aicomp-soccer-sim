//! Immutable RuntimeProgram (Graph ABI v1).

use std::sync::Arc;

use crate::graph_vm::lower::ApiKind;
use crate::graph_vm::opcode::Instruction;

pub const GRAPH_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    pub abi_version: u32,
    pub ir_version: u32,
    pub program_hash: u64,
    pub register_count: u32,
    pub variable_count: u32,
    pub settle_count: u32,
    pub ops: Arc<[Instruction]>,
    pub var_names: Arc<[String]>,
    pub set_variable_sids: Arc<[String]>,
    pub api_labels: Arc<[String]>,
    pub api_kinds: Arc<[ApiKind]>,
}

impl RuntimeProgram {
    pub fn empty() -> Self {
        Self {
            abi_version: GRAPH_ABI_VERSION,
            ir_version: crate::graph_vm::ir::LOWERED_IR_VERSION,
            program_hash: 0,
            register_count: 0,
            variable_count: 0,
            settle_count: 0,
            ops: Arc::from([]),
            var_names: Arc::from([]),
            set_variable_sids: Arc::from([]),
            api_labels: Arc::from([]),
            api_kinds: Arc::from([]),
        }
    }

    pub fn settle_ops(&self) -> &[Instruction] {
        let n = self.settle_count as usize;
        &self.ops[..n.min(self.ops.len())]
    }

    pub fn controller_ops(&self) -> &[Instruction] {
        let n = self.settle_count as usize;
        if n >= self.ops.len() {
            &[]
        } else {
            &self.ops[n..]
        }
    }
}

pub trait Backend {
    fn execute_settle(
        &mut self,
        program: &RuntimeProgram,
        ctx: &mut crate::graph_vm::context::ExecutionContext,
    );

    fn execute_controllers(
        &mut self,
        program: &RuntimeProgram,
        ctx: &mut crate::graph_vm::context::ExecutionContext,
    );
}
