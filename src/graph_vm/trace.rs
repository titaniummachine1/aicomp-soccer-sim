//! Observable traces for acceptance: GraphBrain ≡ Runtime (phased).

use bevy::prelude::Vec2;

use crate::brain::BrainOutput;
use crate::graph_vm::value::VmValue;

/// One SetVariable commit observed during a settle pass.
#[derive(Debug, Clone, PartialEq)]
pub struct VarCommit {
    pub name: String,
    pub value: VmValue,
}

/// Full think observability — acceptance criterion (not score/win-rate).
#[derive(Debug, Clone, PartialEq)]
pub struct ObservableTrace {
    /// Exactly 8 passes matching GraphBrain settle loops.
    pub passes: [Vec<VarCommit>; 8],
    pub controllers: BrainOutput,
}

impl ObservableTrace {
    pub fn empty() -> Self {
        Self {
            passes: Default::default(),
            controllers: BrainOutput::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraceMismatch {
    pub tick: u64,
    pub phase: String,
    pub detail: String,
    pub source_sid: String,
    pub source_port: String,
    pub expected: String,
    pub actual: String,
}

impl std::fmt::Display for TraceMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tick {}\n  → {}\n  → {}\n  Instruction SID: {} Port: {}\n  Expected: {}\n  Actual: {}",
            self.tick,
            self.phase,
            self.detail,
            self.source_sid,
            self.source_port,
            self.expected,
            self.actual
        )
    }
}

pub fn compare_traces(
    tick: u64,
    reference: &ObservableTrace,
    runtime: &ObservableTrace,
) -> Option<TraceMismatch> {
    for (pi, (rp, tp)) in reference
        .passes
        .iter()
        .zip(runtime.passes.iter())
        .enumerate()
    {
        if rp.len() != tp.len() {
            return Some(TraceMismatch {
                tick,
                phase: format!("Pass {}", pi + 1),
                detail: "variable commit count".into(),
                source_sid: String::new(),
                source_port: String::new(),
                expected: format!("{} commits", rp.len()),
                actual: format!("{} commits", tp.len()),
            });
        }
        for (a, b) in rp.iter().zip(tp.iter()) {
            if a != b {
                return Some(TraceMismatch {
                    tick,
                    phase: format!("Pass {}", pi + 1),
                    detail: format!("Variable: {}", a.name),
                    source_sid: String::new(),
                    source_port: String::new(),
                    expected: format!("{:?}", a.value),
                    actual: format!("{:?}", b.value),
                });
            }
        }
    }
    for i in 0..4 {
        let a = &reference.controllers.commands[i];
        let b = &runtime.controllers.commands[i];
        if a.sprint != b.sprint || a.interact != b.interact || !vec_eq(a.move_to, b.move_to) {
            return Some(TraceMismatch {
                tick,
                phase: "Controllers".into(),
                detail: format!("Controller {}", i + 1),
                source_sid: String::new(),
                source_port: String::new(),
                expected: format!("{:?}", a),
                actual: format!("{:?}", b),
            });
        }
    }
    None
}

fn vec_eq(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= f32::EPSILON && (a.y - b.y).abs() <= f32::EPSILON
}
