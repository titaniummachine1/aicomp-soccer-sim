//! Match scenarios (viewer / drill). Full match keeps live-game contracts;
//! Scenario 1 still uses a full 8-player roster with inactive players parked.

use crate::brain::TeamId;
use crate::world::MatchWorld;

/// The viewer/game layout selected for a match.
///
/// Every variant is the ORDINARY game — same kickoff, same rules, same scoring
/// — differing only in how many players each side fields. That mirrors what the
/// real game does as a match runs long: it removes one player per side at each
/// `Max Simulation Time` boundary (measured 2026-07-26: 3v3 at 180 s, 2v2 at
/// 360 s, 1v1 at 540 s), so these are the states a real match actually passes
/// through, not artificial drills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchScenario {
    /// 4v4 — the default.
    #[default]
    Full,
    ThreeV3,
    TwoV2,
    OneV1,
    /// 1v1 with one extra rule: the side that did NOT take the kickoff is the
    /// goalkeeper, and it wins by GETTING POSSESSION of the ball — not by
    /// catching it specifically, just by having it. Goals still count as they
    /// normally do; this is an additional win condition, not a replacement.
    GoalkeeperDuel,
    /// A full-strength opponent (all 4, unmodified) against our keeper alone.
    Scenario3Full4v1,
}

impl MatchScenario {
    pub const ALL: [Self; 6] = [
        Self::Full,
        Self::ThreeV3,
        Self::TwoV2,
        Self::OneV1,
        Self::GoalkeeperDuel,
        Self::Scenario3Full4v1,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full match (4v4)",
            Self::ThreeV3 => "3v3",
            Self::TwoV2 => "2v2",
            Self::OneV1 => "1v1",
            Self::GoalkeeperDuel => "GK duel (1v1, GK wins on possession)",
            Self::Scenario3Full4v1 => "GK vs 4v1",
        }
    }

    /// Players per side. Removal is highest id first, matching the real game's
    /// own attrition order, so 1v1 always leaves P1.
    pub fn squad_size(self) -> u8 {
        match self {
            Self::Full | Self::Scenario3Full4v1 => 4,
            Self::ThreeV3 => 3,
            Self::TwoV2 => 2,
            Self::OneV1 | Self::GoalkeeperDuel => 1,
        }
    }

    /// True when the receiving side also wins by simply having the ball.
    pub fn gk_wins_on_possession(self) -> bool {
        matches!(self, Self::GoalkeeperDuel | Self::Scenario3Full4v1)
    }

    pub fn cycle(self) -> Self {
        let all = Self::ALL;
        let i = all.iter().position(|s| *s == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    pub fn is_scenario1(self) -> bool {
        matches!(self, Self::GoalkeeperDuel)
    }

    pub fn is_scenario_4v1(self) -> bool {
        matches!(self, Self::Scenario3Full4v1)
    }
}

/// Terminal result for Scenario 1 (attacker vs GK).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario1Outcome {
    /// Attacking side scored.
    AttackerGoal,
    /// GK P4 captured or tackled the ball.
    GkHold,
    /// Time budget expired (drill / optional viewer cap).
    GkTimeout,
}

impl Scenario1Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::AttackerGoal => "ATK WIN (goal)",
            Self::GkHold => "GK WIN (capture)",
            Self::GkTimeout => "GK WIN (timeout)",
        }
    }
}

/// Evaluate Scenario 1 terminal conditions after a physics step.
///
/// `start_home` / `start_away` are scores at round start. Own-goal by the
/// attacker counts as GK win (same as `titanium_drill`).
pub fn evaluate_scenario1(
    world: &MatchWorld,
    attack_home: bool,
    start_home: u32,
    start_away: u32,
) -> Option<Scenario1Outcome> {
    let scored_home = world.match_state.score_home > start_home;
    let scored_away = world.match_state.score_away > start_away;
    if scored_home || scored_away {
        let attacker_scored = if attack_home { scored_home } else { scored_away };
        return Some(if attacker_scored {
            Scenario1Outcome::AttackerGoal
        } else {
            Scenario1Outcome::GkHold
        });
    }

    let gk_team = if attack_home {
        TeamId::Away
    } else {
        TeamId::Home
    };
    // The GK side wins the moment it has the ball AT ALL — any player on that
    // side, however it arrived. This is IN ADDITION to the goal conditions
    // above, which are unchanged.
    //
    // It used to require slot 4 specifically. That made the drill a test of
    // which player did the taking rather than of whether the attack was
    // stopped, and it does not survive the scenarios where the keeper is not
    // slot 4 — nothing in the engine makes 4 a goalkeeper, it is only a
    // convention a graph chooses.
    //
    // Possession is authoritative for both a free-ball capture and a tackle
    // steal; held-ball synchronization can happen later in the same tick.
    if matches!(world.possession.carrier, Some((t, _)) if t == gk_team) {
        return Some(Scenario1Outcome::GkHold);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{evaluate_scenario1, MatchScenario, Scenario1Outcome};
    use crate::brain::TeamId;
    use crate::params::SimParams;
    use crate::titanium::setup_1v1_harness;
    use crate::world::MatchWorld;

    #[test]
    fn cycle_round_trips() {
        // One full pass through ALL returns to the start, whatever its length.
        for start in MatchScenario::ALL {
            let mut s = start;
            for _ in 0..MatchScenario::ALL.len() {
                s = s.cycle();
            }
            assert_eq!(s, start, "cycling {start:?} once per variant must round trip");
        }
    }

    #[test]
    fn squad_sizes_step_down_one_at_a_time() {
        assert_eq!(MatchScenario::Full.squad_size(), 4);
        assert_eq!(MatchScenario::ThreeV3.squad_size(), 3);
        assert_eq!(MatchScenario::TwoV2.squad_size(), 2);
        assert_eq!(MatchScenario::OneV1.squad_size(), 1);
        // The GK duel IS 1v1 — the only difference is the extra win condition.
        assert_eq!(MatchScenario::GoalkeeperDuel.squad_size(), 1);
        assert!(MatchScenario::GoalkeeperDuel.gk_wins_on_possession());
        assert!(!MatchScenario::OneV1.gk_wins_on_possession());
    }

    #[test]
    fn scenario1_gk_hold_is_terminal() {
        let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);
        setup_1v1_harness(&mut world, true, 1.0);
        assert!(evaluate_scenario1(&world, true, 0, 0).is_none());

        world.possession.carrier = Some((TeamId::Away, 4));
        world.ball.held = true;
        assert_eq!(
            evaluate_scenario1(&world, true, 0, 0),
            Some(Scenario1Outcome::GkHold)
        );
    }

    #[test]
    fn scenario1_gk_capture_is_terminal_before_hold_sync() {
        let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);
        setup_1v1_harness(&mut world, true, 1.0);
        world.possession.carrier = Some((TeamId::Away, 4));
        world.ball.held = false;
        assert_eq!(
            evaluate_scenario1(&world, true, 0, 0),
            Some(Scenario1Outcome::GkHold)
        );
    }

    #[test]
    fn scenario1_attacker_goal_is_terminal() {
        let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);
        setup_1v1_harness(&mut world, true, 1.0);
        world.match_state.score_home = 1;
        assert_eq!(
            evaluate_scenario1(&world, true, 0, 0),
            Some(Scenario1Outcome::AttackerGoal)
        );
        // Own-goal → GK side win signal (treated as GkHold in drill parity).
        world.match_state.score_home = 0;
        world.match_state.score_away = 1;
        assert_eq!(
            evaluate_scenario1(&world, true, 0, 0),
            Some(Scenario1Outcome::GkHold)
        );
    }
}
