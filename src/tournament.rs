//! Tournament ruleset: regulation → extra time → sudden death with players
//! removed → possession decides.
//!
//! Clock units are sim seconds, which is also the in-game minute: the match
//! clock in the real game advances roughly one displayed minute per real
//! second, so a "90 minute" match is 90 s of `clock_s` here. Every duration
//! below is therefore both, and needs no conversion.
//!
//! The ladder, with defaults:
//!
//! ```text
//!   0.. 90   regulation      — ahead on goals at 90 wins
//!  90..180   extra time      — played out in full, compared again at 180
//! 180..210   sudden death 3v3  \
//! 210..240   sudden death 2v2   >  first goal wins outright
//! 240..270   sudden death 1v1  /
//!      270   possession % decides; exactly equal is a draw
//! ```
//!
//! Extra time is deliberately NOT golden-goal — it is played to the end and
//! the score compared, matching "90 and whoever has more goals wins, 180 if
//! they tie". Only the removal phase is sudden death ("if still no goal").
//!
//! Removal is highest player id first (P3, then P2, then P1), so both sides
//! always end at P4 alone. It is by INDEX on purpose: this simulator has no
//! role concept — Player 4 is a goalkeeper only because a given graph chooses
//! to use it that way — so any rule phrased as "the keeper stays" would not be
//! well defined across opponents. Index order is identical for every entrant
//! and independent of match state, so the same fixture always plays out the
//! same way.

use crate::brain::{BrainCommand, BrainOutput, TeamId};
use crate::world::MatchWorld;

/// Roster size per side. Matches `PlayerId::ALL`.
pub const SQUAD: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TournamentRules {
    /// Normal time. Ahead on goals here ends the match.
    pub regulation_s: f32,
    /// Extra time played after a level regulation, in full.
    pub extra_time_s: f32,
    /// One player leaves each side this often, once sudden death starts.
    pub drop_interval_s: f32,
    /// Stop removing at this many players per side.
    pub min_players: u8,
    /// How long the final `min_players` stage runs before possession decides.
    pub final_block_s: f32,
}

impl Default for TournamentRules {
    /// MEASURED from the real game, 2026-07-26 (`timeplot_..._14-49-59.json`,
    /// a 560 s run against the blank control):
    ///
    ///   kickoff restarts at real 180.0 / 360.0 / 540.0 s — i.e. every
    ///   `Max Simulation Time`, whose default is 180
    ///   one player per side stops playing at each of those boundaries:
    ///     180 -> 3v3   360 -> 2v2   540 -> 1v1
    ///
    /// The author describes this as "every 90 minutes"; that is the same thing
    /// in DISPLAY minutes — 180 s of API clock renders as 90 on-screen minutes,
    /// so the display runs at 0.5 game-minutes per API second. Everything here
    /// is API seconds, the unit `Current Simulation Time` reports.
    fn default() -> Self {
        Self {
            regulation_s: 180.0,
            // Removals start immediately after regulation; there is no separate
            // extra-time block in the observed schedule.
            extra_time_s: 0.0,
            drop_interval_s: 180.0,
            min_players: 1,
            final_block_s: 180.0,
        }
    }
}

impl TournamentRules {
    /// Clock at which extra time ends and sudden death begins.
    pub fn sudden_death_start_s(&self) -> f32 {
        self.regulation_s + self.extra_time_s
    }

    /// Number of removals needed to reach `min_players`.
    pub fn drop_count(&self) -> u8 {
        SQUAD.saturating_sub(self.min_players)
    }

    /// Clock at which the match is over and possession decides.
    ///
    /// The LAST removal opens the final stage rather than adding a block of its
    /// own, so there are `drop_count - 1` full intervals before it plus
    /// `final_block_s`: 180 + 30 (3v3) + 30 (2v2) + 30 (1v1) = 270, not 300.
    pub fn full_length_s(&self) -> f32 {
        self.sudden_death_start_s()
            + self.drop_interval_s * (self.drop_count().saturating_sub(1)) as f32
            + self.final_block_s
    }

    /// Players per side at `clock_s`.
    ///
    /// The first removal lands exactly ON `sudden_death_start_s`, so the 4v4
    /// stage is regulation + extra time and nothing else.
    pub fn players_at(&self, clock_s: f32) -> u8 {
        let start = self.sudden_death_start_s();
        if clock_s < start {
            return SQUAD;
        }
        let elapsed = clock_s - start;
        let dropped = (elapsed / self.drop_interval_s).floor() as i64 + 1;
        let dropped = dropped.clamp(0, self.drop_count() as i64) as u8;
        SQUAD - dropped
    }

    /// Ids still playing at `clock_s` — lowest ids leave last, so P4 remains.
    pub fn active_ids(&self, clock_s: f32) -> Vec<u8> {
        (1..=self.players_at(clock_s)).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// Ahead on goals at the end of regulation.
    Regulation,
    /// Ahead on goals at the end of extra time.
    ExtraTime,
    /// Scored during the player-removal phase.
    SuddenDeath,
    /// Level on goals throughout; more possession wins.
    Possession,
    /// Level on goals AND exactly level on possession.
    Draw,
}

impl DecidedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            DecidedBy::Regulation => "regulation",
            DecidedBy::ExtraTime => "extra_time",
            DecidedBy::SuddenDeath => "sudden_death",
            DecidedBy::Possession => "possession",
            DecidedBy::Draw => "draw",
        }
    }
}

/// Park every removed player off pitch and blank their commands.
///
/// Both halves are needed: a goal sends everyone back to a kickoff spawn via
/// `reset_to_kickoff`, which would otherwise walk a removed player straight
/// back onto the pitch, so re-park every tick rather than once per removal.
pub fn enforce_removals(world: &mut MatchWorld, active_count: u8) {
    if active_count >= SQUAD {
        return;
    }
    let params = world.params.clone();
    for player in &mut world.players {
        if player.id.0 <= active_count {
            continue;
        }
        player.pos = crate::titanium::harness::park_off_pitch(player.team, player.id.0, &params);
        player.vel = bevy::prelude::Vec2::ZERO;
    }
}

/// Blank the commands of removed players so their graph cannot act.
pub fn freeze_removed(out: &mut BrainOutput, world: &MatchWorld, team: TeamId, active_count: u8) {
    if active_count >= SQUAD {
        return;
    }
    for player in &world.players {
        if player.team != team || player.id.0 <= active_count {
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

/// Decide the match from its final state. `clock_s` is where play stopped.
pub fn decide(
    rules: &TournamentRules,
    clock_s: f32,
    score_home: u32,
    score_away: u32,
    possession_home: f32,
    possession_away: f32,
) -> (Option<TeamId>, DecidedBy) {
    let leader = match score_home.cmp(&score_away) {
        std::cmp::Ordering::Greater => Some(TeamId::Home),
        std::cmp::Ordering::Less => Some(TeamId::Away),
        std::cmp::Ordering::Equal => None,
    };

    if let Some(team) = leader {
        // `clock_s` is where the loop stopped, so the stage that produced the
        // lead is read off the clock rather than tracked separately.
        let stage = if clock_s <= rules.regulation_s + f32::EPSILON {
            DecidedBy::Regulation
        } else if clock_s <= rules.sudden_death_start_s() + f32::EPSILON {
            DecidedBy::ExtraTime
        } else {
            DecidedBy::SuddenDeath
        };
        return (Some(team), stage);
    }

    if possession_home > possession_away {
        (Some(TeamId::Home), DecidedBy::Possession)
    } else if possession_away > possession_home {
        (Some(TeamId::Away), DecidedBy::Possession)
    } else {
        (None, DecidedBy::Draw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ladder_matches_the_documented_timeline() {
        let r = TournamentRules::default();
        assert_eq!(r.sudden_death_start_s(), 180.0);
        assert_eq!(r.full_length_s(), 720.0);
        // 4v4 for all of regulation and extra time.
        assert_eq!(r.players_at(0.0), 4);
        assert_eq!(r.players_at(89.9), 4);
        assert_eq!(r.players_at(179.9), 4);
        // Removals land on the block boundaries.
        assert_eq!(r.players_at(180.0), 3);
        assert_eq!(r.players_at(359.9), 3);
        assert_eq!(r.players_at(360.0), 2);
        assert_eq!(r.players_at(540.0), 1);
        // Never below min_players, however long it runs.
        assert_eq!(r.players_at(719.9), 1);
        assert_eq!(r.players_at(10_000.0), 1);
    }

    #[test]
    fn removal_keeps_the_lowest_ids() {
        let r = TournamentRules::default();
        assert_eq!(r.active_ids(0.0), vec![1, 2, 3, 4]);
        assert_eq!(r.active_ids(180.0), vec![1, 2, 3]);
        assert_eq!(r.active_ids(360.0), vec![1, 2]);
        assert_eq!(r.active_ids(540.0), vec![1]);
    }

    #[test]
    fn goals_beat_possession_and_possession_breaks_a_tie() {
        let r = TournamentRules::default();
        // Behind on goals but far ahead on possession still loses.
        assert_eq!(decide(&r, 180.0, 0, 1, 80.0, 5.0), (Some(TeamId::Away), DecidedBy::Regulation));
        assert_eq!(decide(&r, 400.0, 1, 0, 0.0, 0.0), (Some(TeamId::Home), DecidedBy::SuddenDeath));
        assert_eq!(decide(&r, 720.0, 0, 0, 44.0, 43.0), (Some(TeamId::Home), DecidedBy::Possession));
        assert_eq!(decide(&r, 720.0, 3, 3, 10.0, 10.0), (None, DecidedBy::Draw));
    }
}
