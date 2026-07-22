//! ProgramBuilder — packing only. Encoding may change without ABI bump.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::graph_vm::ir::LoweredIR;
use crate::graph_vm::opcode::Instruction;
use crate::graph_vm::program::{RuntimeProgram, GRAPH_ABI_VERSION};

#[derive(Debug, Default)]
pub struct ProgramBuilder;

impl ProgramBuilder {
    pub fn pack(&self, ir: &LoweredIR) -> RuntimeProgram {
        let mut ops = Vec::with_capacity(ir.instructions.len());
        let mut max_reg = 0u32;
        for inst in &ir.instructions {
            if let Some(d) = inst.dest {
                max_reg = max_reg.max(d.0 + 1);
            }
            for a in &inst.args {
                max_reg = max_reg.max(a.0 + 1);
            }
            let mut operands = Vec::new();
            if let Some(d) = inst.dest {
                operands.push(d.0);
            }
            for a in &inst.args {
                operands.push(a.0);
            }
            operands.extend_from_slice(&inst.immediates);
            ops.push(Instruction::new(
                inst.op,
                &operands,
                inst.source_sid.clone(),
                inst.source_port.clone(),
            ));
        }
        let program_hash = hash_ops(&ops);
        RuntimeProgram {
            abi_version: GRAPH_ABI_VERSION,
            ir_version: ir.ir_version,
            program_hash,
            register_count: max_reg,
            variable_count: 0,
            ops: ops.into(),
        }
    }
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
