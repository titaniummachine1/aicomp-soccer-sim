//! Brain hook — reads `TeamApi` (SoccerGet*), writes SoccerController×4.

use bevy::prelude::*;

use crate::api::{ApiFieldMask, TeamApi};
use crate::player::PlayerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamId {
    Home,
    Away,
}

impl TeamId {
    pub fn other(self) -> Self {
        match self {
            TeamId::Home => TeamId::Away,
            TeamId::Away => TeamId::Home,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrainCommand {
    pub move_to: Vec2,
    pub sprint: bool,
    pub interact: bool,
}

impl Default for BrainCommand {
    fn default() -> Self {
        Self {
            move_to: Vec2::ZERO,
            sprint: false,
            interact: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrainOutput {
    pub commands: [BrainCommand; 4],
    /// Faceoff spots this tick's evaluation produced for
    /// `ConstructSoccerProperties`, in team space (+X = attacking).
    ///
    /// Real graphs do not hand these in as constants — AIA computes them from
    /// `TeamMultiplier` and a "is it our kickoff" conditional, so they are
    /// only knowable by running the graph. `None` means the port is unwired
    /// and the engine default stands.
    pub kickoff_positions: [Option<Vec2>; 4],
}

impl Default for BrainOutput {
    fn default() -> Self {
        Self {
            commands: [BrainCommand::default(); 4],
            kickoff_positions: [None; 4],
        }
    }
}

impl BrainOutput {
    pub fn for_player(&self, id: PlayerId) -> BrainCommand {
        let i = (id.0.saturating_sub(1) as usize).min(3);
        self.commands[i]
    }
}

pub trait TeamBrain: Send + Sync {
    fn think(&mut self, api: &TeamApi) -> BrainOutput;

    /// Faceoff spots this brain's graph declares via
    /// `ConstructSoccerProperties`, in team space (+X = attacking). Slots left
    /// `None` fall back to the engine default. Hand-written brains have no
    /// graph to declare them, so the default is "no opinion".
    fn kickoff_formation(&self) -> [Option<bevy::prelude::Vec2>; 4] {
        [None; 4]
    }

    /// Returns a mask of which API fields this brain actually reads.
    /// `None` means "compute everything" (backward compatible).
    /// `Some(mask)` lets `build_apis` skip expensive computations for
    /// fields the brain never reads.
    fn api_mask(&self) -> Option<ApiFieldMask> {
        None
    }
}

impl TeamBrain for Box<dyn TeamBrain> {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        (**self).think(api)
    }

    fn kickoff_formation(&self) -> [Option<bevy::prelude::Vec2>; 4] {
        (**self).kickoff_formation()
    }

    fn api_mask(&self) -> Option<ApiFieldMask> {
        (**self).api_mask()
    }
}

/// Stand still — useful as a parked opponent in probes / headless A/B.
#[derive(Debug, Default)]
pub struct IdleBrain;

impl TeamBrain for IdleBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();
        for (i, slot) in PlayerId::ALL.iter().enumerate() {
            let me_label = match slot.0 {
                1 => "Team Player 1",
                2 => "Team Player 2",
                3 => "Team Player 3",
                _ => "Team Player 4",
            };
            let me = api.get_transform(me_label).unwrap_or(Vec2::ZERO);
            out.commands[i] = BrainCommand {
                move_to: me,
                sprint: false,
                interact: false,
            };
        }
        out
    }
}

/// Chase ball via SoccerGet* labels — proves API I/O path.
#[derive(Debug, Default)]
pub struct ChaseBallBrain {
    /// Ticks this brain has run. Per-instance on purpose: a shared counter
    /// advanced once per brain per tick, so with two chasers on the pitch one
    /// side pressed every tick (and thus stayed latched) while the other never
    /// pressed at all.
    tick: u64,
}

impl TeamBrain for ChaseBallBrain {
    fn api_mask(&self) -> Option<ApiFieldMask> {
        let mut m = ApiFieldMask::none();
        m.needs_bool_set("Team Has Ball");
        m.needs_float_set("Player Interact Radius");
        m.needs_float_set("Ball Carrier Shot Charge");
        m.needs_transform_set("Ball");
        m.needs_transform_set("Opponent Goal Center");
        for n in 1..=4u8 {
            m.needs_bool_set(&format!("Team Player {n} Has Ball"));
            m.needs_bool_set(&format!("Is Ball Nearby Team Player {n}"));
            m.needs_transform_set(&format!("Team Player {n}"));
        }
        Some(m)
    }

    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        // One press, then at least one full tick released, so the latch can
        // clear and the next press is a fresh impulse. Advanced once per
        // think() — i.e. per tick — not once per player.
        let press_tick = self.tick % 2 == 0;
        self.tick = self.tick.wrapping_add(1);
        let mut out = BrainOutput::default();
        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let opp_goal = api
            .get_transform("Opponent Goal Center")
            .unwrap_or(Vec2::new(39.5, 0.0));
        let team_has = api.get_bool("Team Has Ball").unwrap_or(false);
        let interact_r = api.get_float("Player Interact Radius").unwrap_or(1.5);

        for (i, slot) in PlayerId::ALL.iter().enumerate() {
            let has_label = match slot.0 {
                1 => "Team Player 1 Has Ball",
                2 => "Team Player 2 Has Ball",
                3 => "Team Player 3 Has Ball",
                _ => "Team Player 4 Has Ball",
            };
            let near_label = match slot.0 {
                1 => "Is Ball Nearby Team Player 1",
                2 => "Is Ball Nearby Team Player 2",
                3 => "Is Ball Nearby Team Player 3",
                _ => "Is Ball Nearby Team Player 4",
            };
            let me_label = match slot.0 {
                1 => "Team Player 1",
                2 => "Team Player 2",
                3 => "Team Player 3",
                _ => "Team Player 4",
            };

            let has_ball = api.get_bool(has_label).unwrap_or(false);
            let near = api.get_bool(near_label).unwrap_or(false);
            let me = api.get_transform(me_label).unwrap_or(Vec2::ZERO);
            let charge = api.get_float("Ball Carrier Shot Charge").unwrap_or(0.0);

            let (move_to, sprint, interact) = if has_ball {
                let charged = charge > 0.55;
                (opp_goal, true, !charged)
            } else if team_has {
                // Support: stay near ball
                (ball, true, false)
            } else {
                let dist = (me - ball).length();
                // PULSE the claim. Interact is an impulse: it fires on the
                // press and needs a release before it can fire again, so a
                // brain that simply pins it true claims once and then never
                // again. At kickoff the free-ball claim is additionally gated
                // for ~1s, so a pinned press is spent before it is even legal
                // and the ball is never picked up at all.
                let want = near || dist <= interact_r;
                (ball, true, want && press_tick)
            };

            out.commands[i] = BrainCommand {
                move_to,
                sprint,
                interact,
            };
        }
        out
    }
}
