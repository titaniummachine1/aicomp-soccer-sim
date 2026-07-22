//! Frozen opcode ABI (semantics). Encoding layout is NOT frozen.

use std::num::NonZeroU16;

/// Graph ABI v1 opcode set. Changing semantics requires ABI bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum OpCode {
    ConstFloat = 1,
    ConstBool,
    ConstVec,
    ConstNull,
    LoadVar,
    StoreVar,
    LoadApi,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Min,
    Max,
    Abs,
    Clamp,
    Lerp,
    Neg,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Not,
    ConstructVec,
    SplitVec,
    AddVec,
    SubVec,
    ScaleVec,
    Normalize,
    Magnitude,
    Distance,
    Dot,
    Select,
    Call,
    Return,
    EmitController,
    Debug,
    TimePlot,
    OpaqueEffect,
    Move,
    Operation,
    IsNull,
    Nor,
    Nand,
    ScaleAddVec,
}

/// Effect class for CSE / fusion / movement barriers.
///
/// - **Pure**: no state read/write.
/// - **ReadOnly**: reads frozen [`crate::graph_vm::ApiSnapshot`] / vars without write.
///   Deterministic inside one `think`. CSE of `LoadApi(slot)` allowed **within the
///   same snapshot only** — never across ticks.
/// - **Write**: modifies VM state (`StoreVar`, …).
/// - **External**: outside VM; ordering matters (controllers, debug, TimePlot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpEffect {
    Pure,
    ReadOnly,
    Write,
    External,
}

impl OpCode {
    pub fn effect(self) -> OpEffect {
        use OpCode::*;
        match self {
            ConstFloat | ConstBool | ConstVec | ConstNull => OpEffect::Pure,
            Add | Sub | Mul | Div | Mod | Pow | Min | Max | Abs | Clamp | Lerp | Neg => {
                OpEffect::Pure
            }
            Eq | Ne | Lt | Gt | Le | Ge | And | Or | Not | Nor | Nand => OpEffect::Pure,
            ConstructVec | SplitVec | AddVec | SubVec | ScaleVec | ScaleAddVec | Normalize
            | Magnitude | Distance | Dot | Select | Move | Operation | IsNull => OpEffect::Pure,
            LoadApi | LoadVar => OpEffect::ReadOnly,
            StoreVar => OpEffect::Write,
            Call | Return => OpEffect::Write,
            EmitController | Debug | TimePlot | OpaqueEffect => OpEffect::External,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OpInfo {
    pub opcode: OpCode,
    pub effect: OpEffect,
    pub min_inputs: u8,
    pub outputs: u8,
}

/// Flexible encoding — NOT part of Graph ABI binary layout freeze.
/// Every op carries visual-graph provenance for verify diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: OpCode,
    pub operands: Operands,
    /// Visual node sID that produced this op (empty if synthetic).
    pub source_sid: String,
    /// Output port name on that node (e.g. `"Float1"`, `"output"`).
    pub source_port: String,
}

impl Instruction {
    pub fn new(
        opcode: OpCode,
        operands: &[u32],
        source_sid: impl Into<String>,
        source_port: impl Into<String>,
    ) -> Self {
        Self {
            opcode,
            operands: Operands::from_slice(operands),
            source_sid: source_sid.into(),
            source_port: source_port.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Operands {
    pub data: [u32; 8],
    pub len: u8,
}

impl Operands {
    pub fn from_slice(xs: &[u32]) -> Self {
        let mut o = Self::default();
        let n = xs.len().min(8);
        o.data[..n].copy_from_slice(&xs[..n]);
        o.len = n as u8;
        o
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.data[..self.len as usize]
    }
}

pub type ApiSlot = NonZeroU16;
