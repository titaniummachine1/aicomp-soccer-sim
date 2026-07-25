//! Execution context: frame + persistent state + frozen API snapshot.

use std::sync::Arc;

use crate::api::TeamApi;
use crate::brain::BrainOutput;
use crate::graph::pitch_vec::vec3_from_pitch;
use crate::graph_vm::lower::{ApiKind, ApiSlotTable};
use crate::graph_vm::trace::{ObservableTrace, VarCommit};
use crate::graph_vm::value::VmValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariableId(pub u16);

/// Frozen for one `think` — Backend must not observe live API mutation.
#[derive(Debug, Clone)]
pub struct ApiSnapshot {
    pub api: Arc<TeamApi>,
    pub slot_kinds: Arc<[ApiKind]>,
    pub slot_dense_ids: Arc<[u16]>,
}

impl ApiSnapshot {
    pub fn from_table(api: TeamApi, table: &ApiSlotTable) -> Self {
        Self {
            api: Arc::new(api),
            slot_kinds: Arc::from(table.kinds.clone()),
            slot_dense_ids: Arc::from(table.dense_ids.clone()),
        }
    }

    pub fn load_slot(&self, slot: u16) -> VmValue {
        let Some(idx) = slot.checked_sub(1).map(usize::from) else {
            return VmValue::Null;
        };
        let dense = match self.slot_dense_ids.get(idx).copied() {
            Some(id) if id != crate::api::UNKNOWN_ID => id,
            _ => return VmValue::Null,
        };
        match self.slot_kinds.get(idx).copied().unwrap_or(ApiKind::Float) {
            ApiKind::Bool => VmValue::Bool(self.api.get_bool_id(dense).unwrap_or(false)),
            ApiKind::Float => VmValue::Float(self.api.get_float_id(dense).unwrap_or(0.0)),
            ApiKind::Transform => VmValue::Vector(vec3_from_pitch(
                self.api.get_transform_id(dense).unwrap_or_default(),
            )),
            ApiKind::Vector3 => match self.api.get_vector_id(dense) {
                Some(Some(v)) => VmValue::Vector(vec3_from_pitch(v)),
                _ => VmValue::Null,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeState {
    pub vars: Vec<VmValue>,
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
    pub output: BrainOutput,
    pub trace: Option<ObservableTrace>,
    pub var_names: Arc<[String]>,
    pub set_variable_sids: Arc<[String]>,
    current_pass: usize,
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
            api: ApiSnapshot {
                api: Arc::new(api),
                slot_kinds: Arc::from([]),
                slot_dense_ids: Arc::from([]),
            },
            output: BrainOutput::default(),
            trace: None,
            var_names: Arc::from([]),
            set_variable_sids: Arc::from([]),
            current_pass: 0,
        }
    }

    pub fn init_api_slots(&mut self, table: &ApiSlotTable) {
        let api = (*self.api.api).clone();
        self.api = ApiSnapshot::from_table(api, table);
    }

    pub fn begin_pass(&mut self, pass: usize) {
        self.current_pass = pass;
        self.frame.reset();
    }

    pub fn record_var_commit(&mut self, var_id: u16, value: VmValue, source_sid: &str) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        let name = self
            .var_names
            .get(var_id as usize)
            .cloned()
            .unwrap_or_else(|| format!("var_{var_id}"));
        let _ = source_sid;
        if self.current_pass < 8 {
            trace.passes[self.current_pass].push(VarCommit { name, value });
        }
    }
}
