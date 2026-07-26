//! Tournament ruleset: goals, then attrition, then stats.
//!
//! Every duration is derived from ONE number — the match time you assign,
//! which the API reports as `Max Simulation Time` (default 180). Call it `T`.
//! Each decision block is `T/2`:
//!
//! ```text
//!    0 ..  T/2    4v4   decide on goals
//!  T/2 ..  T      4v4   extra time, decide on goals again
//!    T .. 1.5T    3v3   one RANDOM player removed from each side
//! 1.5T .. 2T      2v2   another random removal
//!   2T .. 2.5T    1v1   another random removal
//!       2.5T      still level -> better stats wins
//! ```
//!
//! With the default T = 180 that is decision points at 90, 180, 270, 360, 450.
//!
//! UNRESOLVED, and deliberately not hidden: a real-game recording
//! (`timeplot_2026-07-26_14-49-59.json`) shows kickoff restarts at exactly
//! 180 / 360 / 540 s — every `T`, not every `T/2` — while `Max Simulation Time`
//! read 180.0 the whole run. The author describes the blocks as half-time each,
//! which is what is implemented here. Either those restarts are not decision
//! points, or the recording's display/API factor differs from the assumed one.
//! `block_s` is the single knob that reconciles it.
//!
//! Removal is RANDOM, not ordered. That matters beyond fairness: it is a real
//! source of run-to-run variance, so any match reaching attrition genuinely
//! differs between runs. Good for sampling, fatal for replay unless the RNG is
//! seeded — hence `removal_seed`, which makes a given (seed, match) pair replay
//! exactly while still varying across seeds.

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::world::MatchWorld;

/// Roster size per side. Matches `PlayerId::ALL`.
pub const SQUAD: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TournamentRules {
    /// The assigned match time — the API's `Max Simulation Time`.
    pub match_time_s: f32,
    /// Stop removing at this many players per side.
    pub min_players: u8,
    /// Seeds the removal draw so a run replays exactly.
    pub removal_seed: u64,
}

impl Default for TournamentRules {
    fn default() -> Self {
        Self {
            match_time_s: 180.0,
            min_players: 1,
            removal_seed: 0,
        }
    }
}

impl TournamentRules {
    /// One decision block: half the assigned match time.
    pub fn block_s(&self) -> f32 {
        self.match_time_s * 0.5
    }

    /// Attrition begins once full time has been played without a winner.
    pub fn attrition_start_s(&self) -> f32 {
        self.match_time_s
    }

    /// Number of removals needed to reach `min_players`.
    pub fn drop_count(&self) -> u8 {
        SQUAD.saturating_sub(self.min_players)
    }

    /// Clock at which stats decide it.
    pub fn full_length_s(&self) -> f32 {
        self.attrition_start_s() + self.block_s() * self.drop_count() as f32
    }

    /// Players per side at `clock_s`.
    pub fn players_at(&self, clock_s: f32) -> u8 {
        let start = self.attrition_start_s();
        if clock_s < start {
            return SQUAD;
        }
        let blocks = ((clock_s - start) / self.block_s()).floor() as i64 + 1;
        SQUAD - blocks.clamp(0, self.drop_count() as i64) as u8
    }

    /// Which player ids are still playing for `team` at `clock_s`.
    ///
    /// The draw is a deterministic function of (seed, team, removal number), so
    /// it is random across seeds and identical on replay. Each removal picks
    /// uniformly from whoever is still on the pitch, independently per side —
    /// the two teams do not lose the same slot.
    pub fn active_ids(&self, team: TeamId, clock_s: f32) -> Vec<u8> {
        let mut alive: Vec<u8> = (1..=SQUAD).collect();
        let removals = SQUAD - self.players_at(clock_s);
        for round in 0..removals {
            let pick = self.draw(team, round, alive.len() as u64) as usize;
            alive.remove(pick);
        }
        alive
    }

    /// Uniform index in `0..n`, from a splitmix64 of the seeded key. Small,
    /// dependency-free, and good enough for choosing one of at most four.
    fn draw(&self, team: TeamId, round: u8, n: u64) -> u64 {
        let team_bit = match team {
            TeamId::Home => 0x9E37_79B9_7F4A_7C15u64,
            TeamId::Away => 0xBF58_476D_1CE4_E5B9u64,
        };
        let mut x = self
            .removal_seed
            .wrapping_mul(0x2545_F491_4F6C_DD1D)
            .wrapping_add(team_bit)
            .wrapping_add((round as u64).wrapping_mul(0x1234_5678_9ABC_DEF1));
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        x % n.max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// Ahead on goals at the end of normal time.
    Regulation,
    /// Ahead on goals at the end of extra time.
    ExtraTime,
    /// Ahead on goals at the end of an attrition block.
    Attrition,
    /// Level on goals the whole way; better stats wins.
    Stats,
    /// Level on goals AND level on stats.
    Draw,
}

impl DecidedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            DecidedBy::Regulation => "regulation",
            DecidedBy::ExtraTime => "extra_time",
            DecidedBy::Attrition => "attrition",
            DecidedBy::Stats => "stats",
            DecidedBy::Draw => "draw",
        }
    }
}

/// The end-of-match board, in the order the game shows it.
///
/// INCOMPLETE ON PURPOSE. The scoreboard carries Goals, Assists, Shots, Saves,
/// Passes, Touches, Giveaways, Tackles, Steals and Possession, plus a per-player
/// rating. Which of those actually decides a level 1v1 is NOT yet established —
/// the author first said possession, then "better stats". Only the fields the
/// simulator genuinely tracks are here; adding the rest would mean inventing
/// numbers, which is worse than admitting the gap.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TeamStats {
    pub goals: u32,
    pub shots: u32,
    pub possession_s: f32,
}

/// Compare two teams' stats. `Greater` means `a` wins the tiebreak.
///
/// Possession first, because that is the one the author named explicitly and
/// the one the simulator measures reliably. Shots break a possession tie only
/// as a documented guess — flagged rather than silently assumed.
pub fn compare_stats(a: &TeamStats, b: &TeamStats) -> std::cmp::Ordering {
    a.possession_s
        .partial_cmp(&b.possession_s)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(a.shots.cmp(&b.shots))
}

/// Park every removed player off pitch and blank their commands.
///
/// Re-park every tick, not once: a goal sends everyone back to a kickoff spawn
/// via `reset_to_kickoff`, which would otherwise walk a removed player straight
/// back on.
pub fn enforce_removals(world: &mut MatchWorld, rules: &TournamentRules, clock_s: f32) {
    if rules.players_at(clock_s) >= SQUAD {
        return;
    }
    let params = world.params.clone();
    let home = rules.active_ids(TeamId::Home, clock_s);
    let away = rules.active_ids(TeamId::Away, clock_s);
    for player in &mut world.players {
        let alive = match player.team {
            TeamId::Home => &home,
            TeamId::Away => &away,
        };
        if alive.contains(&player.id.0) {
            continue;
        }
        player.pos = crate::titanium::harness::park_off_pitch(player.team, player.id.0, &params);
        player.vel = bevy::prelude::Vec2::ZERO;
    }
}

/// Blank the commands of removed players so their graph cannot act.
pub fn freeze_removed(
    out: &mut BrainOutput,
    world: &MatchWorld,
    rules: &TournamentRules,
    team: TeamId,
    clock_s: f32,
) {
    if rules.players_at(clock_s) >= SQUAD {
        return;
    }
    let alive = rules.active_ids(team, clock_s);
    for player in &world.players {
        if player.team != team || alive.contains(&player.id.0) {
            continue;
        }
        let index = (player.id.0 as usize).saturating_sub(1);
        if index < out.commands.len() {
            out.commands[index] = BrainCommand {
                move_to: player.pos,
                sprint: false,
                interact: false,
            };
        }
    }
}

/// Decide the match. `clock_s` is where play stopped.
pub fn decide(
    rules: &TournamentRules,
    clock_s: f32,
    home: &TeamStats,
    away: &TeamStats,
) -> (Option<TeamId>, DecidedBy) {
    // Goals always outrank stats. The scoreboard proves it: Titanium won 1:0
    // while trailing possession 48/51 and tackles 3/16.
    let leader = match home.goals.cmp(&away.goals) {
        std::cmp::Ordering::Greater => Some(TeamId::Home),
        std::cmp::Ordering::Less => Some(TeamId::Away),
        std::cmp::Ordering::Equal => None,
    };

    if let Some(team) = leader {
        let stage = if clock_s <= rules.block_s() + f32::EPSILON {
            DecidedBy::Regulation
        } else if clock_s <= rules.match_time_s + f32::EPSILON {
            DecidedBy::ExtraTime
        } else {
            DecidedBy::Attrition
        };
        return (Some(team), stage);
    }

    match compare_stats(home, away) {
        std::cmp::Ordering::Greater => (Some(TeamId::Home), DecidedBy::Stats),
        std::cmp::Ordering::Less => (Some(TeamId::Away), DecidedBy::Stats),
        std::cmp::Ordering::Equal => (None, DecidedBy::Draw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(goals: u32, shots: u32, poss: f32) -> TeamStats {
        TeamStats {
            goals,
            shots,
            possession_s: poss,
        }
    }

    #[test]
    fn ladder_is_half_time_blocks_off_the_assigned_match_time() {
        let r = TournamentRules::default();
        assert_eq!(r.block_s(), 90.0);
        assert_eq!(r.attrition_start_s(), 180.0);
        assert_eq!(r.full_length_s(), 450.0);

        assert_eq!(r.players_at(0.0), 4);
        assert_eq!(r.players_at(179.9), 4, "4v4 for regulation AND extra time");
        assert_eq!(r.players_at(180.0), 3);
        assert_eq!(r.players_at(269.9), 3);
        assert_eq!(r.players_at(270.0), 2);
        assert_eq!(r.players_at(360.0), 1);
        assert_eq!(r.players_at(10_000.0), 1, "never below min_players");
    }

    #[test]
    fn ladder_scales_with_the_assigned_time() {
        // Everything is relative, so a different match length just rescales.
        let r = TournamentRules {
            match_time_s: 60.0,
            ..Default::default()
        };
        assert_eq!(r.block_s(), 30.0);
        assert_eq!(r.players_at(59.9), 4);
        assert_eq!(r.players_at(60.0), 3);
        assert_eq!(r.players_at(90.0), 2);
        assert_eq!(r.full_length_s(), 150.0);
    }

    #[test]
    fn removals_are_random_but_replay_identically() {
        let r = TournamentRules {
            removal_seed: 7,
            ..Default::default()
        };
        let at_1v1 = r.active_ids(TeamId::Home, 400.0);
        assert_eq!(at_1v1.len(), 1);
        assert_eq!(at_1v1, r.active_ids(TeamId::Home, 400.0), "same seed replays");

        // Survivors shrink monotonically — a player removed stays removed.
        let three = r.active_ids(TeamId::Home, 200.0);
        let two = r.active_ids(TeamId::Home, 300.0);
        assert_eq!((three.len(), two.len()), (3, 2));
        assert!(two.iter().all(|id| three.contains(id)));
        assert!(at_1v1.iter().all(|id| two.contains(id)));
    }

    #[test]
    fn different_seeds_can_remove_different_players() {
        let picks: std::collections::HashSet<Vec<u8>> = (0..24u64)
            .map(|seed| {
                TournamentRules {
                    removal_seed: seed,
                    ..Default::default()
                }
                .active_ids(TeamId::Home, 400.0)
            })
            .collect();
        assert!(
            picks.len() > 1,
            "removal must actually vary across seeds, got {picks:?}"
        );
    }

    #[test]
    fn goals_outrank_stats_and_stats_break_a_level_score() {
        let r = TournamentRules::default();
        // The screenshot: 1:0 winner despite trailing possession.
        assert_eq!(
            decide(&r, 90.0, &stats(1, 1, 48.0), &stats(0, 0, 51.0)),
            (Some(TeamId::Home), DecidedBy::Regulation)
        );
        assert_eq!(
            decide(&r, 180.0, &stats(2, 3, 10.0), &stats(1, 9, 90.0)),
            (Some(TeamId::Home), DecidedBy::ExtraTime)
        );
        assert_eq!(
            decide(&r, 300.0, &stats(1, 1, 1.0), &stats(0, 0, 1.0)),
            (Some(TeamId::Home), DecidedBy::Attrition)
        );
        assert_eq!(
            decide(&r, 450.0, &stats(0, 0, 44.0), &stats(0, 0, 43.0)),
            (Some(TeamId::Home), DecidedBy::Stats)
        );
        assert_eq!(
            decide(&r, 450.0, &stats(3, 5, 10.0), &stats(3, 5, 10.0)),
            (None, DecidedBy::Draw)
        );
    }
}
