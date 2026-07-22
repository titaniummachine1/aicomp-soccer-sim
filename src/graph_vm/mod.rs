//! Graph VM — compiled RuntimeProgram + Backend (faithful to GraphBrain observables).
//!
//! Spec: plan `graph_jit_interpreter` v1.2. Strings are metadata only (no String registers).

pub mod builder;
pub mod context;
pub mod interpreter;
pub mod ir;
pub mod lower;
pub mod opcode;
pub mod passes;
pub mod program;
pub mod runtime_brain;
pub mod trace;
pub mod value;
pub mod verify;

pub use builder::ProgramBuilder;
pub use context::{ApiSnapshot, ExecutionContext, ExecutionFrame, RuntimeState, VariableId};
pub use interpreter::Interpreter;
pub use lower::{ApiKind, ApiSlotTable, CompileResult, Lowerer, VariableTable};
pub use opcode::{OpCode, OpEffect, OpInfo, Instruction};
pub use program::RuntimeProgram;
pub use runtime_brain::RuntimeBrain;
pub use trace::{ObservableTrace, TraceMismatch, compare_traces};
pub use value::{RegisterKind, VmValue};
