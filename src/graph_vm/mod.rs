//! Graph VM — compiled RuntimeProgram + Backend (faithful to GraphBrain observables).
//!
//! Spec: plan `graph_jit_interpreter` v1.2. Strings are metadata only (no String registers).

pub mod builder;
pub mod context;
pub mod interpreter;
pub mod ir;
pub mod diagnostics;
pub mod watch;
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
pub use opcode::{Instruction, OpCode, OpEffect, OpInfo};
pub use program::RuntimeProgram;
pub use runtime_brain::{CachedProgram, RuntimeBrain};
pub use trace::{compare_traces, ObservableTrace, TraceMismatch};
pub use value::{RegisterKind, VmValue};
