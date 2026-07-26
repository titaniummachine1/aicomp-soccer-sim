//! How much you can trust each API getter's VALUE.
//!
//! `graph_vm::diagnostics` grades node TYPES — whether an operation is
//! implemented. This grades the other half: whether a getter that is
//! implemented actually returns what the real game returns. Both failures look
//! identical downstream (a number comes out, a decision gets made), and the
//! second is the one that has actually bitten us:
//!
//!   * `SoccerGetFloat` carried a phantom entry at index 10, shifting every
//!     label after it. 14 of the 15 floats the champion reads resolved to the
//!     wrong value — `Team Player 1 Stamina` returned the constant `Goal
//!     Height`. Fixed 2026-07-26; every gate result before that is void.
//!   * `Team Possession %` returns 0..100 here and 0..1 in the real game,
//!     measured the same day. Still open.
//!
//! Neither was caught by any test, because "returns a plausible float" is not
//! a testable property. Naming the evidence is.
//!
//! THE DEFAULT IS `Uncertain`. A getter earns `Confirmed` only by having been
//! compared against a real-game reading, and the `note` must say which one.
//! Anything unlisted is reported as `Uncertain`, never silently assumed fine.

use crate::graph::dropdowns::SOCCER_GET_FLOAT;

/// Confidence in a getter's returned value, worst-first ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Read from a real-game recording and matched. `note` cites the source.
    Confirmed,
    /// Deliberate documented model — right shape, not the engine's own code.
    Approximated,
    /// Implemented from reasoning, never compared to the game. Suspect first.
    Uncertain,
    /// Known WRONG or absent. Any decision using it is not evidence.
    Unimplemented,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Confirmed => "CONFIRMED (matched a real-game reading)",
            Confidence::Approximated => "APPROXIMATED (documented model)",
            Confidence::Uncertain => "UNCERTAIN (never measured)",
            Confidence::Unimplemented => "WRONG/ABSENT (do not trust results)",
        }
    }
}

pub struct Graded {
    pub label: &'static str,
    pub confidence: Confidence,
    pub note: &'static str,
}

/// Every judgement below cites its evidence. Add a row only with a source.
///
/// Evidence marked "MatchProbe 2026-07-26" is the real-game TimePlot export
/// `timeplot_2026-07-26_13-55-58.json`, produced by
/// `titanim-socker-engine/scripts/circle_probe.py`.
const FLOAT_GRADES: &[Graded] = &[
    Graded {
        label: "Player Interact Radius",
        confidence: Confidence::Confirmed,
        note: "1.750 in game and sim (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Field Width",
        confidence: Confidence::Confirmed,
        note: "50.0 in game and sim (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Field Depth",
        confidence: Confidence::Confirmed,
        note: "80.0 in game and sim (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Goal Width",
        confidence: Confidence::Confirmed,
        note: "11.4 in game and sim; half-width 5.7 (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Fixed Delta Time",
        confidence: Confidence::Confirmed,
        note: "0.019 in game; equals sim FIXED_DT and the TimePlot sample spacing",
    },
    Graded {
        label: "Delta Time",
        confidence: Confidence::Confirmed,
        note: "0.019 in game, constant across the recording (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Max Simulation Time",
        confidence: Confidence::Confirmed,
        note: "180.0 default in game; it is a SETTING and can be changed per match",
    },
    Graded {
        label: "Current Simulation Time",
        confidence: Confidence::Confirmed,
        note: "advances 1.0 per real second in game — the on-screen 20x is display only",
    },
    Graded {
        label: "Simulation Time Remaining",
        confidence: Confidence::Confirmed,
        note: "equals Max - Current throughout (MatchProbe 2026-07-26)",
    },
    Graded {
        label: "Pi",
        confidence: Confidence::Confirmed,
        note: "3.14159 in game — the canary that proves index alignment",
    },
    Graded {
        label: "Team Possession %",
        confidence: Confidence::Unimplemented,
        note: "SCALE MISMATCH: sim returns 0..100 starting at 50; game returns \
               0..1 (read 1.000 at 100% possession). Decides a goalless \
               tournament match, so this must be fixed before it is used.",
    },
    Graded {
        label: "Opponent Possession %",
        confidence: Confidence::Unimplemented,
        note: "same 0..100 vs 0..1 mismatch as Team Possession %",
    },
    Graded {
        label: "Team Attacking %",
        confidence: Confidence::Unimplemented,
        note: "sim returns 50.0 at kickoff, game returns 0.000 — scale and \
               baseline both unverified",
    },
    Graded {
        label: "Opponent Attacking %",
        confidence: Confidence::Unimplemented,
        note: "same as Team Attacking %",
    },
    Graded {
        label: "Ball Carrier Stamina",
        confidence: Confidence::Approximated,
        note: "0..1 in both; game reached 1.000 while carrying. The DRAIN model \
               is the sim's own and has never been compared tick-by-tick.",
    },
    Graded {
        label: "Goal Height",
        confidence: Confidence::Uncertain,
        note: "sim reports 2.44; no real-game reading captured yet",
    },
    Graded {
        label: "Kickoff Circle Radius",
        confidence: Confidence::Uncertain,
        note: "sim reports 7.25; matches the measured 7.75 standoff minus a body \
               radius, but was never read from the game directly",
    },
];

pub fn grade(label: &str) -> &'static Graded {
    static UNKNOWN: Graded = Graded {
        label: "<unlisted>",
        confidence: Confidence::Uncertain,
        note: "no evidence recorded — never compared against the real game",
    };
    FLOAT_GRADES
        .iter()
        .find(|g| g.label == label)
        .unwrap_or(&UNKNOWN)
}

/// Human-readable audit of every float getter, worst first.
pub fn report() -> String {
    let mut rows: Vec<(&Graded, &str)> = SOCCER_GET_FLOAT
        .iter()
        .map(|label| (grade(label), *label))
        .collect();
    rows.sort_by_key(|(g, label)| (std::cmp::Reverse(g.confidence), *label));

    let mut out = String::from("SoccerGetFloat value fidelity (worst first)\n");
    let mut counts = [0usize; 4];
    for (g, label) in &rows {
        counts[match g.confidence {
            Confidence::Confirmed => 0,
            Confidence::Approximated => 1,
            Confidence::Uncertain => 2,
            Confidence::Unimplemented => 3,
        }] += 1;
        out.push_str(&format!("  {:<46} {}\n", label, g.confidence.label()));
        if !g.note.is_empty() && g.confidence != Confidence::Uncertain {
            out.push_str(&format!("      {}\n", g.note));
        }
    }
    out.push_str(&format!(
        "\n  confirmed {}  approximated {}  uncertain {}  wrong/absent {}  (of {})\n",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        SOCCER_GET_FLOAT.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_graded_label_actually_exists() {
        // A typo in a grade row would silently grade nothing.
        for g in FLOAT_GRADES {
            assert!(
                SOCCER_GET_FLOAT.contains(&g.label),
                "graded label {:?} is not in SOCCER_GET_FLOAT",
                g.label
            );
        }
    }

    #[test]
    fn unlisted_getters_default_to_uncertain() {
        // The default must never be optimistic: an ungraded getter is unknown,
        // not fine.
        assert_eq!(grade("Team Shots").confidence, Confidence::Uncertain);
    }

    #[test]
    fn known_mismatches_are_flagged_not_forgotten() {
        assert_eq!(
            grade("Team Possession %").confidence,
            Confidence::Unimplemented,
            "the 0..100 vs 0..1 mismatch is unfixed; downgrade only when it is"
        );
    }
}
