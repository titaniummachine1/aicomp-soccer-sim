//! Differential verify: observable TRACE identity (acceptance criterion).

use crate::brain::BrainOutput;
use crate::graph_vm::trace::{compare_traces, ObservableTrace, TraceMismatch};

pub use crate::graph_vm::trace::compare_traces as compare_observable_traces;

/// Convenience: controller-only compare (insufficient for acceptance — prefer traces).
pub fn compare_outputs(reference: &BrainOutput, runtime: &BrainOutput) -> Option<TraceMismatch> {
    let mut a = ObservableTrace::empty();
    let mut b = ObservableTrace::empty();
    a.controllers = reference.clone();
    b.controllers = runtime.clone();
    compare_traces(0, &a, &b)
}

pub type VerifyMismatch = TraceMismatch;
