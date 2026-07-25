//! How much you can trust a simulated result.
//!
//! The lowerer's fallback turns an unknown node into a Null constant. The
//! graph then loads, the match runs, and a decision gets made on a value that
//! was never computed — silently. A result nobody can trust is worse than a
//! crash, because it looks like evidence.
//!
//! So every node type is classified, and anything unclassified is an ERROR
//! rather than a shrug. When something goes wrong, this says which bucket to
//! look in.

use std::collections::BTreeSet;
use std::sync::Mutex;

/// How faithfully one node type reproduces the real game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trust {
    /// Verified against the real game, or exact by construction (pure
    /// arithmetic). Results are guaranteed parity.
    Exact,
    /// Deliberate, documented approximation. Close, but a known model — not
    /// the engine's own behaviour.
    Approximated,
    /// Implemented on reasoning alone, never measured against the game. May
    /// hold, may not. If a result looks wrong, suspect these first.
    Vibed,
    /// Not implemented at all. Evaluates to Null: any decision touching it was
    /// made on nothing. Always an error.
    Unimplemented,
}

impl Trust {
    pub fn label(self) -> &'static str {
        match self {
            Trust::Exact => "EXACT (guaranteed parity)",
            Trust::Approximated => "APPROXIMATED (documented model)",
            Trust::Vibed => "VIBED (unverified — suspect first)",
            Trust::Unimplemented => "UNIMPLEMENTED (evaluates to Null)",
        }
    }
}

/// Node types that do not compute a value used by any decision. Stubbing them
/// cannot change play, so they are exact for simulation purposes.
const SIDE_EFFECT_ONLY: &[&str] = &[
    "Debug",
    "DebugDrawDisc",
    "DebugDrawLine",
    "TimePlot",
    "ConstructSoccerProperties",
    "Country",
    "Stat",
];

/// Implemented, but as an acknowledged approximation of the engine.
const APPROXIMATED: &[&str] = &[
    // Geometric blocker disc rather than Unity's real collider sweep.
    "Spherecast",
    // Rectangular test standing in for the engine's region shape.
    "Region",
];

/// Classify a node type. Anything not listed and not lowered is Unimplemented.
pub fn classify(node_id: &str) -> Trust {
    if SIDE_EFFECT_ONLY.contains(&node_id) {
        Trust::Exact
    } else if APPROXIMATED.contains(&node_id) {
        Trust::Approximated
    } else {
        Trust::Unimplemented
    }
}

static STUBBED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn lock() -> std::sync::MutexGuard<'static, BTreeSet<String>> {
    STUBBED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called by the lowerer whenever it stubs a node type to Null.
pub fn record_unimplemented(node_id: &str) {
    lock().insert(node_id.to_string());
}

pub fn clear() {
    lock().clear();
}

/// Node types stubbed this process, paired with how much they can be trusted.
pub fn stubbed() -> Vec<(String, Trust)> {
    lock().iter().map(|n| (n.clone(), classify(n))).collect()
}

/// Node types that are genuinely unimplemented — the ones that make a result
/// unsound rather than merely approximate.
pub fn unsound() -> Vec<String> {
    stubbed()
        .into_iter()
        .filter(|(_, t)| *t == Trust::Unimplemented)
        .map(|(n, _)| n)
        .collect()
}

/// Loudest honest summary of the run so far.
pub fn banner() -> String {
    let all = stubbed();
    if all.is_empty() {
        return "FIDELITY: EXACT — every node type implemented".into();
    }
    let bad = unsound();
    let mut out = String::new();
    if bad.is_empty() {
        out.push_str("FIDELITY: EXACT for decisions — stubbed nodes affect no result:\n");
    } else {
        out.push_str(&format!(
            "!!! FIDELITY: UNSOUND — {} unimplemented node type(s). Any decision \
             touching them was made on Null, NOT simulated:\n",
            bad.len()
        ));
    }
    for (n, t) in all {
        out.push_str(&format!("    {:<28} {}\n", n, t.label()));
    }
    out
}

/// Panic if the graph contained anything genuinely unimplemented.
///
/// Deliberately loud: a silent Null is indistinguishable from a real answer
/// downstream, so a gate result computed over one is not evidence of anything.
pub fn assert_sound() {
    let bad = unsound();
    assert!(
        bad.is_empty(),
        "simulator cannot faithfully run this graph — unimplemented node type(s): {}. \
         They evaluate to Null, so any result is UNSOUND. Implement them or classify \
         them in graph_vm::diagnostics.",
        bad.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: an unknown node must be loud, not silently Null.
    #[test]
    fn unknown_node_is_unimplemented_and_unsound() {
        clear();
        record_unimplemented("SomeNodeNobodyImplemented");
        assert_eq!(classify("SomeNodeNobodyImplemented"), Trust::Unimplemented);
        assert_eq!(unsound(), vec!["SomeNodeNobodyImplemented".to_string()]);
        assert!(banner().contains("UNSOUND"));
        clear();
    }

    /// Draw/plot sinks feed no decision, so stubbing them cannot change play.
    #[test]
    fn side_effect_sinks_do_not_make_a_result_unsound() {
        clear();
        for n in ["DebugDrawLine", "TimePlot", "Stat"] {
            record_unimplemented(n);
            assert_eq!(classify(n), Trust::Exact);
        }
        assert!(unsound().is_empty(), "sinks must not be flagged unsound");
        assert!(!banner().contains("UNSOUND"));
        clear();
    }

    /// Approximations are trusted less than exact, but are not unsound.
    #[test]
    fn approximations_are_flagged_but_not_unsound() {
        clear();
        record_unimplemented("Spherecast");
        assert_eq!(classify("Spherecast"), Trust::Approximated);
        assert!(unsound().is_empty());
        assert!(banner().contains("APPROXIMATED"));
        clear();
    }

    #[test]
    #[should_panic(expected = "UNSOUND")]
    fn assert_sound_panics_on_unimplemented() {
        clear();
        record_unimplemented("TotallyMadeUpNode");
        assert_sound();
    }
}
