//! Match replay recording and playback.
//!
//! Records per-tick world state into a compact JSON file that can be replayed
//! in the Bevy viewer or analyzed headlessly.

use serde::{Deserialize, Serialize};

use crate::ball::Ball;
use crate::match_state::{MatchPhase, MatchState};
use crate::possession::Possession;

/// One tick of world state, compact enough for ~14000 ticks per match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFrame {
    /// Ball position (x, y) and velocity (vx, vy).
    pub ball: [f32; 4],
    /// Ball height (for airborne visualization).
    pub ball_h: f32,
    /// Ball held flag.
    pub ball_held: bool,
    /// Per-player: [x, y, vx, vy, stamina, shot_charge] for 8 players.
    /// Order: Home P1-P4, Away P1-P4.
    pub players: [[f32; 6]; 8],
    /// Carrier: (team, player_id) or None.
    pub carrier: Option<(u8, u8)>,
    /// Match phase: 0=Kickoff, 1=Play, 2=GoalPause.
    pub phase: u8,
    /// Match clock in seconds.
    pub clock_s: f32,
    /// Score (home, away).
    pub score: [u32; 2],
}

/// Full replay file with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub home_label: String,
    pub away_label: String,
    pub home_score: u32,
    pub away_score: u32,
    pub duration_s: f32,
    pub frames: Vec<ReplayFrame>,
}

impl Replay {
    pub fn new(home_label: &str, away_label: &str) -> Self {
        Self {
            home_label: home_label.to_string(),
            away_label: away_label.to_string(),
            home_score: 0,
            away_score: 0,
            duration_s: 0.0,
            frames: Vec::new(),
        }
    }

    /// Record a single tick from the world state.
    pub fn record_tick(
        &mut self,
        ball: &Ball,
        players: &[crate::player::Player],
        possession: &Possession,
        match_state: &MatchState,
    ) {
        let mut player_data = [[0.0f32; 6]; 8];
        for (i, p) in players.iter().enumerate().take(8) {
            player_data[i] = [
                p.pos.x,
                p.pos.y,
                p.vel.x,
                p.vel.y,
                p.stamina,
                p.shot_charge,
            ];
        }

        let carrier = possession
            .carrier
            .map(|(t, id)| (t as u8, id));

        let phase = match match_state.phase {
            MatchPhase::Kickoff => 0u8,
            MatchPhase::Play => 1,
            MatchPhase::GoalPause => 2,
        };

        self.frames.push(ReplayFrame {
            ball: [ball.pos.x, ball.pos.y, ball.vel.x, ball.vel.y],
            ball_h: ball.height,
            ball_held: ball.held,
            players: player_data,
            carrier,
            phase,
            clock_s: match_state.clock_s,
            score: [match_state.score_home, match_state.score_away],
        });

        self.duration_s = match_state.clock_s;
        self.home_score = match_state.score_home;
        self.away_score = match_state.score_away;
    }

    /// Save to a JSON file.
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string(self)
            .map_err(|e| format!("serialize replay: {e}"))?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create replay dir: {e}"))?;
            }
        }
        std::fs::write(path, json)
            .map_err(|e| format!("write replay file: {e}"))
    }

    /// Load from a JSON file.
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("read replay file: {e}"))?;
        serde_json::from_str(&data)
            .map_err(|e| format!("parse replay file: {e}"))
    }

    /// Get a frame by index, clamped to valid range.
    pub fn frame(&self, idx: usize) -> Option<&ReplayFrame> {
        self.frames.get(idx.min(self.frames.len().saturating_sub(1)))
    }

    /// Number of recorded frames.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}
