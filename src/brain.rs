//! Brain hook — reads `TeamApi` (SoccerGet*), writes SoccerController×4.

use bevy::prelude::*;

use crate::api::TeamApi;
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

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone)]
pub struct BrainOutput {
    pub commands: [BrainCommand; 4],
}

impl Default for BrainOutput {
    fn default() -> Self {
        Self {
            commands: [BrainCommand::default(); 4],
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
}

impl TeamBrain for Box<dyn TeamBrain> {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        (**self).think(api)
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
pub struct ChaseBallBrain;

impl TeamBrain for ChaseBallBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
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
                (ball, true, near || dist <= interact_r)
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
