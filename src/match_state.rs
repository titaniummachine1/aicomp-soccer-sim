//! Match FSM: kickoff → play → goal → kickoff.

use bevy::prelude::*;

use crate::ball::Ball;
use crate::brain::TeamId;
use crate::params::SimParams;
use crate::player::{faceoff_world, kickoff_facing, Player};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    Kickoff,
    Play,
    GoalPause,
}

#[derive(Resource, Debug, Clone)]
pub struct MatchState {
    pub phase: MatchPhase,
    pub clock_s: f32,
    pub duration_s: f32,
    pub score_home: u32,
    pub score_away: u32,
    /// Team that scored the most recent goal (`None` until first goal).
    pub last_scorer: Option<TeamId>,
    pub shots_home: u32,
    pub shots_away: u32,
    /// Seconds each side held the ball (Play phase).
    pub possession_s_home: f32,
    pub possession_s_away: f32,
    /// Seconds the ball spent on each side's attacking half (Play phase).
    pub attacking_s_home: f32,
    pub attacking_s_away: f32,
    pub kickoff_team: TeamId,
    pub opening_kickoff_team: TeamId,
    pub phase_timer: f32,
    /// Receiving team stays outside the center circle until first release (or
    /// ball leaves the ring). Real Away O3 skirts early; free entry let them
    /// sit on the carrier C-lane or collapse midfield too soon.
    pub kickoff_circle_lock: bool,
    /// Suppress Away "Ball On Team Side" (defender chase) for the same window.
    pub kickoff_suppress_away_team_side: bool,
    /// True once someone has held the ball this kickoff (for release unlock).
    pub kickoff_seen_carrier: bool,
    /// Anchor for stale-ball whistle: ball must move ≥ threshold from this
    /// point within [`SimParams::stale_ball_timeout_s`] or kickoff is reset.
    pub stale_anchor: Vec2,
    /// Seconds since the ball last moved past the stale distance threshold.
    pub stale_idle_s: f32,
}

impl Default for MatchState {
    /// Deterministic default (Home opens). Prefer [`MatchState::new_match`] for live games.
    fn default() -> Self {
        Self::with_opening_kickoff(TeamId::Home)
    }
}

impl MatchState {
    /// New match: opening kickoff is random; after goals, scored-on team restarts.
    pub fn new_match() -> Self {
        Self::with_opening_kickoff(random_opening_kickoff())
    }

    pub fn with_opening_kickoff(opening: TeamId) -> Self {
        Self {
            phase: MatchPhase::Kickoff,
            clock_s: 0.0,
            duration_s: 180.0,
            score_home: 0,
            score_away: 0,
            last_scorer: None,
            shots_home: 0,
            shots_away: 0,
            possession_s_home: 0.0,
            possession_s_away: 0.0,
            attacking_s_home: 0.0,
            attacking_s_away: 0.0,
            kickoff_team: opening,
            opening_kickoff_team: opening,
            phase_timer: 0.0,
            kickoff_circle_lock: true,
            kickoff_suppress_away_team_side: true,
            kickoff_seen_carrier: false,
            stale_anchor: Vec2::ZERO,
            stale_idle_s: 0.0,
        }
    }

    /// `scored_on` conceded — they receive the next kickoff (AIComp rule).
    pub fn on_goal(&mut self, scored_on: TeamId, kickoff_delay_s: f32) {
        let scorer = scored_on.other();
        match scored_on {
            TeamId::Home => self.score_away += 1,
            TeamId::Away => self.score_home += 1,
        }
        self.last_scorer = Some(scorer);
        self.phase = MatchPhase::GoalPause;
        self.phase_timer = kickoff_delay_s;
        self.kickoff_team = scored_on;
        self.kickoff_circle_lock = true;
        self.kickoff_suppress_away_team_side = true;
        self.kickoff_seen_carrier = false;
        self.reset_stale_tracker(Vec2::ZERO);
    }

    pub fn record_shot(&mut self, team: TeamId) {
        match team {
            TeamId::Home => self.shots_home += 1,
            TeamId::Away => self.shots_away += 1,
        }
    }

    /// Accumulate possession / attacking clocks during Play.
    pub fn tick_match_stats(
        &mut self,
        dt: f32,
        carrier: Option<(TeamId, u8)>,
        ball_x: f32,
    ) {
        if self.phase != MatchPhase::Play {
            return;
        }
        match carrier.map(|(t, _)| t) {
            Some(TeamId::Home) => self.possession_s_home += dt,
            Some(TeamId::Away) => self.possession_s_away += dt,
            None => {}
        }
        // Home attacks +X; Away attacks −X.
        if ball_x > 0.0 {
            self.attacking_s_home += dt;
        } else if ball_x < 0.0 {
            self.attacking_s_away += dt;
        }
    }

    /// Stale-ball whistle: no score change; kickoff flips to opposite of who
    /// last received kickoff (Frida: 5s / 2.5m — see `stale_ball_*` params).
    pub fn on_whistle(&mut self, kickoff_delay_s: f32) {
        self.kickoff_team = match self.kickoff_team {
            TeamId::Home => TeamId::Away,
            TeamId::Away => TeamId::Home,
        };
        self.phase = MatchPhase::GoalPause;
        self.phase_timer = kickoff_delay_s;
        self.kickoff_circle_lock = true;
        self.kickoff_suppress_away_team_side = true;
        self.kickoff_seen_carrier = false;
        self.reset_stale_tracker(Vec2::ZERO);
    }

    pub fn reset_stale_tracker(&mut self, anchor: Vec2) {
        self.stale_anchor = anchor;
        self.stale_idle_s = 0.0;
    }
}

fn random_opening_kickoff() -> TeamId {
    use std::time::{SystemTime, UNIX_EPOCH};
    let bit = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        & 1;
    if bit == 0 {
        TeamId::Home
    } else {
        TeamId::Away
    }
}

pub fn place_kickoff(
    ball: &mut Ball,
    players: &mut [Player],
    kickoff_team: TeamId,
    rest_height: f32,
) {
    ball.pos = Vec2::ZERO;
    ball.vel = Vec2::ZERO;
    ball.height = rest_height;
    ball.vel_y = 0.0;
    ball.held = false;
    for p in players.iter_mut() {
        // Positions / facing / charge reset. Stamina intentionally persists
        // across whistle and goal kickoffs (TimePlot + Discord 2026-07-22).
        p.pos = faceoff_world(p.team, p.id, kickoff_team);
        p.vel = Vec2::ZERO;
        p.facing = kickoff_facing(p.team, p.id, kickoff_team);
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
    }
    // Kicking striker starts with the ball (avoids body-bump launch at overlap).
    if let Some(p) = players
        .iter()
        .find(|p| p.team == kickoff_team && p.id.0 == 1)
    {
        let hold = p.pos + p.facing * 1.557;
        ball.pos = hold;
        ball.held = true;
    }
}

/// Receiving team may not enter the kickoff circle during Kickoff phase only.
/// After first touch they may enter; Away chase stays suppressed until release
/// so O3 holds State0 through the interior (real path X≈5.5, Z→3).
pub fn receiving_team_circle_locked(match_state: &MatchState) -> bool {
    match_state.phase == MatchPhase::Kickoff
}

pub fn kickoff_control_allowed(
    team: TeamId,
    _player_pos: Vec2,
    match_state: &MatchState,
    _params: &SimParams,
) -> bool {
    match match_state.phase {
        MatchPhase::Play => true,
        MatchPhase::GoalPause => false,
        // Only the kicking-off team gets graph control until first touch / Play.
        MatchPhase::Kickoff => team == match_state.kickoff_team,
    }
}
