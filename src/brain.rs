//! Brain hook — mirrors `SoccerController(player, moveTo, sprint, interact)`.

use bevy::prelude::*;

use crate::ball::Ball;
use crate::player::{Player, PlayerId};

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

#[derive(Debug, Clone)]
pub struct WorldView<'a> {
    pub ball: &'a Ball,
    pub players: &'a [Player],
    pub dt: f32,
}

pub trait TeamBrain: Send + Sync {
    fn think(&mut self, team: TeamId, view: &WorldView<'_>) -> BrainOutput;
}

#[derive(Debug, Default)]
pub struct ChaseBallBrain;

impl TeamBrain for ChaseBallBrain {
    fn think(&mut self, team: TeamId, view: &WorldView<'_>) -> BrainOutput {
        let mut out = BrainOutput::default();
        let attack_goal = match team {
            TeamId::Home => Vec2::new(39.5, 0.0),
            TeamId::Away => Vec2::new(-39.5, 0.0),
        };

        for (i, slot) in PlayerId::ALL.iter().enumerate() {
            let Some(me) = view
                .players
                .iter()
                .find(|p| p.team == team && p.id == *slot)
            else {
                continue;
            };

            let holding = view.ball.held && (me.pos - view.ball.pos).length() < 1.0;
            let (move_to, sprint, interact) = if holding {
                let charge_ready = me.shot_charge > 0.55;
                (attack_goal, true, !charge_ready)
            } else {
                let near = (me.pos - view.ball.pos).length() < 1.5;
                (view.ball.pos, true, near)
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
