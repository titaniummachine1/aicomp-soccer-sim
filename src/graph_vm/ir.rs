//! LoweredIR — SSA-ish write-once; 1:1 lowering target (no opts here).

use crate::graph_vm::opcode::OpCode;
use crate::graph_vm::value::RegisterKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reg(pub u32);

#[derive(Debug, Clone)]
pub struct IrInst {
    pub dest: Option<Reg>,
    pub kind: RegisterKind,
    pub op: OpCode,
    pub args: Vec<Reg>,
    pub immediates: Vec<u32>,
    pub source_sid: String,
    pub source_port: String,
}

pub const LOWERED_IR_VERSION: u32 = 1;

#[derive(Debug, Clone, Default)]
pub struct LoweredIR {
    pub ir_version: u32,
    pub instructions: Vec<IrInst>,
    pub block_succs: Vec<Vec<u32>>,
}

impl LoweredIR {
    pub fn new() -> Self {
        Self {
            ir_version: LOWERED_IR_VERSION,
            instructions: Vec::new(),
            block_succs: Vec::new(),
        }
    }
}
