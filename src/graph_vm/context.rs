//! Execution context: frame + persistent state + frozen API snapshot.

use std::sync::Arc;

use crate::api::TeamApi;
use crate::graph_vm::value::VmValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariableId(pub u16);

/// Frozen for one `think` — Backend must not observe live API mutation.
#[derive(Debug, Clone)]
pub struct ApiSnapshot {
    // M0: hold the TeamApi by value (already a snapshot from build_apis).
    pub api: Arc<TeamApi>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeState {
    pub vars: Vec<VmValue>,
    /// Controllers 1–4 packed later; placeholder slots.
    pub controller_scratch: [VmValue; 12],
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionFrame {
    pub registers: Vec<VmValue>,
    pub call_depth: u16,
}

impl ExecutionFrame {
    pub fn reset(&mut self) {
        for r in &mut self.registers {
            *r = VmValue::Null;
        }
        self.call_depth = 0;
    }

    pub fn ensure_regs(&mut self, n: usize) {
        if self.registers.len() < n {
            self.registers.resize(n, VmValue::Null);
        }
    }
}

#[derive(Debug)]
pub struct ExecutionContext {
    pub frame: ExecutionFrame,
    pub state: RuntimeState,
    pub api: ApiSnapshot,
}

impl ExecutionContext {
    pub fn new(api: TeamApi, var_count: usize, reg_count: usize) -> Self {
        let mut state = RuntimeState::default();
        state.vars.resize(var_count, VmValue::Null);
        let mut frame = ExecutionFrame::default();
        frame.ensure_regs(reg_count);
        Self {
            frame,
            state,
            api: ApiSnapshot { api: Arc::new(api) },
        }
    }
}
