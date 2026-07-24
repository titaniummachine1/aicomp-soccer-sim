//! Match scenarios (viewer / drill). Full match keeps live-game contracts;
//! Scenario 1 still uses a full 8-player roster with inactive players parked.

use crate::brain::TeamId;
use crate::world::MatchWorld;

/// The viewer/game layout selected for a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchScenario {
    #[default]
    Full,
    /// Scenario 1: center attacker P1 vs preset GK P4; extras parked+frozen.
    Scenario1AtkVsGk,
    /// Scenario 3: real full-strength opponent team (all 4, unmodified AI)
    /// vs our GK alone — our own P1-3 parked off-pitch and frozen. GK
    /// capture/tackle at any point = GK goal, same as Scenario 1.
    Scenario3Full4v1,
}

impl MatchScenario {
    pub const ALL: [Self; 3] = [Self::Full, Self::Scenario1AtkVsGk, Self::Scenario3Full4v1];

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full match",
            Self::Scenario1AtkVsGk => "GK duel",
            Self::Scenario3Full4v1 => "GK vs 4v1",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Full => Self::Scenario1AtkVsGk,
            Self::Scenario1AtkVsGk => Self::Scenario3Full4v1,
            Self::Scenario3Full4v1 => Self::Full,
        }
    }

    pub fn is_scenario1(self) -> bool {
        matches!(self, Self::Scenario1AtkVsGk)
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
    // Possession is authoritative for both a free-ball capture and a tackle
    // steal; held-ball synchronization can happen later in the same tick.
    if world.possession.carrier == Some((gk_team, 4)) {
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
        // Three variants now — a full round trip takes 3 cycles, not 2.
        assert_eq!(
            MatchScenario::Full.cycle().cycle().cycle(),
            MatchScenario::Full
        );
        assert_eq!(
            MatchScenario::Scenario1AtkVsGk.cycle().cycle().cycle(),
            MatchScenario::Scenario1AtkVsGk
        );
        assert_eq!(
            MatchScenario::Scenario3Full4v1.cycle().cycle().cycle(),
            MatchScenario::Scenario3Full4v1
        );
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
