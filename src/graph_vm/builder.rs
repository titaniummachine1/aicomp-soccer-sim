//! ProgramBuilder — packing only. Encoding may change without ABI bump.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::graph_vm::ir::{IrInst, LoweredIR};
use crate::graph_vm::lower::CompileResult;
use crate::graph_vm::opcode::Instruction;
use crate::graph_vm::program::{RuntimeProgram, GRAPH_ABI_VERSION};

#[derive(Debug, Default)]
pub struct ProgramBuilder;

impl ProgramBuilder {
    pub fn pack(&self, compiled: &CompileResult) -> RuntimeProgram {
        let mut ops = Vec::new();
        let settle_count = pack_ir(&mut ops, &compiled.settle);
        pack_ir(&mut ops, &compiled.controllers);
        let register_count = max_reg(&compiled.settle) .max(max_reg(&compiled.controllers));
        let program_hash = hash_ops(&ops);
        RuntimeProgram {
            abi_version: GRAPH_ABI_VERSION,
            ir_version: compiled.settle.ir_version,
            program_hash,
            register_count,
            variable_count: compiled.vars.len() as u32,
            settle_count: settle_count as u32,
            ops: ops.into(),
            var_names: Arc::from(compiled.vars.names.clone()),
            set_variable_sids: Arc::from(compiled.set_variable_sids.clone()),
            api_labels: Arc::from(compiled.apis.labels.clone()),
            api_kinds: Arc::from(compiled.apis.kinds.clone()),
        }
    }
}

fn pack_ir(ops: &mut Vec<Instruction>, ir: &LoweredIR) -> usize {
    let start = ops.len();
    for inst in &ir.instructions {
        ops.push(pack_inst(inst));
    }
    ops.len() - start
}

fn pack_inst(inst: &IrInst) -> Instruction {
    let mut operands = Vec::new();
    if let Some(d) = inst.dest {
        operands.push(d.0);
    }
    for a in &inst.args {
        operands.push(a.0);
    }
    operands.extend_from_slice(&inst.immediates);
    Instruction::new(
        inst.op,
        &operands,
        inst.source_sid.clone(),
        inst.source_port.clone(),
    )
}

fn max_reg(ir: &LoweredIR) -> u32 {
    let mut max_reg = 0u32;
    for inst in &ir.instructions {
        if let Some(d) = inst.dest {
            max_reg = max_reg.max(d.0 + 1);
        }
        for a in &inst.args {
            max_reg = max_reg.max(a.0 + 1);
        }
    }
    max_reg
}

fn hash_ops(ops: &[Instruction]) -> u64 {
    let mut h = DefaultHasher::new();
    GRAPH_ABI_VERSION.hash(&mut h);
    for op in ops {
        (op.opcode as u16).hash(&mut h);
        op.operands.as_slice().hash(&mut h);
        op.source_sid.hash(&mut h);
        op.source_port.hash(&mut h);
    }
    h.finish()
}
