//! Possession: interact at hold spot, held ball follows BallHoldLocation, kick + delay.

use bevy::prelude::*;

use crate::ball::Ball;
use crate::brain::{BrainCommand, TeamId};
use crate::params::SimParams;
use crate::player::Player;

#[derive(Resource, Debug, Clone)]
pub struct Possession {
    pub carrier: Option<(TeamId, u8)>,
    pub pickup_lockout: f32,
}

impl Default for Possession {
    fn default() -> Self {
        Self {
            carrier: None,
            pickup_lockout: 0.0,
        }
    }
}

pub fn tick_possession_timers(poss: &mut Possession, dt: f32) {
    poss.pickup_lockout = (poss.pickup_lockout - dt).max(0.0);
}

/// Interact uses the hold/aim point (BallHoldLocation), not body center alone.
pub fn apply_interact(
    player: &mut Player,
    ball: &mut Ball,
    poss: &mut Possession,
    cmd: BrainCommand,
    params: &SimParams,
    dt: f32,
) -> bool {
    let hold = player.hold_pos(params.hold_offset);
    let is_carrier = matches!(
        poss.carrier,
        Some((t, id)) if t == player.team && id == player.id.0
    );

    if is_carrier {
        ball.held = true;
        ball.pos = hold;
        ball.vel = Vec2::ZERO;

        if cmd.interact {
            player.shot_charge = (player.shot_charge + dt / 0.8).min(1.0);
            return false;
        }

        if player.shot_charge > 0.05 {
            let dir = if player.facing.length_squared() > 1e-6 {
                player.facing
            } else {
                (cmd.move_to - player.pos).normalize_or_zero()
            };
            let dir = if dir.length_squared() < 1e-6 {
                match player.team {
                    TeamId::Home => Vec2::X,
                    TeamId::Away => -Vec2::X,
                }
            } else {
                dir.normalize()
            };
            let speed = params.kick_max_speed * player.shot_charge;
            ball.held = false;
            ball.vel = dir * speed;
            ball.pos = hold + dir * 0.15;
            player.shot_charge = 0.0;
            poss.carrier = None;
            poss.pickup_lockout = params.pickup_delay_s;
            return true;
        }
        player.shot_charge = 0.0;
        return false;
    }

    if !cmd.interact {
        return false;
    }

    // Tackle: interact near held ball / opposing carrier hold spot
    if ball.held {
        if let Some((ct, _cid)) = poss.carrier {
            if ct != player.team {
                let dist = (hold - ball.pos).length();
                if dist <= params.interact_radius {
                    poss.carrier = Some((player.team, player.id.0));
                    player.shot_charge = 0.0;
                }
            }
        }
        return false;
    }

    if poss.pickup_lockout > 0.0 {
        return false;
    }

    // Pickup: hold spot (or body) within interact radius of free ball
    let dist = (hold - ball.pos).length().min((player.pos - ball.pos).length());
    if dist <= params.interact_radius {
        poss.carrier = Some((player.team, player.id.0));
        ball.held = true;
        ball.vel = Vec2::ZERO;
        ball.pos = hold;
        player.shot_charge = 0.0;
    }
    false
}

pub fn sync_held_ball(ball: &mut Ball, players: &[Player], poss: &Possession, hold_offset: f32) {
    if let Some((team, id)) = poss.carrier {
        if let Some(p) = players.iter().find(|p| p.team == team && p.id.0 == id) {
            ball.held = true;
            ball.pos = p.hold_pos(hold_offset);
            ball.vel = Vec2::ZERO;
        }
    } else {
        ball.held = false;
    }
}
