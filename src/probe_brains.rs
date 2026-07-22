//! Scripted Test1 / Test2 brains — mirror worldcupteams build_test1/2.py
//! so we can debug tackle flow in-sim before Unity.

use bevy::prelude::Vec2;

use crate::api::TeamApi;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
use crate::player::PlayerId;

const BURN_BELOW: f32 = 0.65;
const AWAY_ENGAGE_STAM: f32 = 0.80;
const WAYPOINT_Z: f32 = 20.0;
const FLIP_AT: f32 = 16.0;
const SHORT_CHARGE: f32 = 0.35;

fn park_others(out: &mut BrainOutput, api: &TeamApi) {
    for slot in PlayerId::ALL.iter().skip(1) {
        let label = match slot.0 {
            2 => "Team Player 2",
            3 => "Team Player 3",
            _ => "Team Player 4",
        };
        let me = api.get_transform(label).unwrap_or(Vec2::ZERO);
        let i = (slot.0 as usize - 1).min(3);
        out.commands[i] = BrainCommand {
            move_to: me,
            sprint: false,
            interact: false,
        };
    }
}

fn sticky_wp(z: f32, prev: &mut f32) -> f32 {
    if prev.abs() < 1.0 {
        *prev = WAYPOINT_Z;
    }
    if z > FLIP_AT {
        *prev = -WAYPOINT_Z;
    } else if z < -FLIP_AT {
        *prev = WAYPOINT_Z;
    }
    *prev
}

/// Home: short kick → N/S sprint-burn → walk+charge bait.
#[derive(Debug, Default)]
pub struct Test1Brain {
    /// 0 = first kick, 1 = burn/bait, 2 = lost.
    phase: f32,
    prev_has: bool,
    prev_charge: f32,
    lap_wp: f32,
}

impl TeamBrain for Test1Brain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();
        park_others(&mut out, api);

        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let me = api.get_transform("Team Player 1").unwrap_or(Vec2::ZERO);
        let has = api.get_bool("Team Player 1 Has Ball").unwrap_or(false);
        let near = api.get_bool("Is Ball Nearby Team Player 1").unwrap_or(false);
        let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false);
        let stam = api.get_float("Team Player 1 Stamina").unwrap_or(1.0);
        let charge = if has {
            api.get_float("Teammate 1 Shot Charge")
                .or_else(|| api.get_float("Ball Carrier Shot Charge"))
                .unwrap_or(0.0)
        } else {
            0.0
        };

        let released = self.prev_has && !has && self.prev_charge >= 0.15;
        if opp_has && !has {
            self.phase = 2.0;
        } else if self.phase < 0.5 && released {
            self.phase = 1.0;
        }

        let phase0 = self.phase < 0.5;
        let phase1 = (0.5..1.5).contains(&self.phase);
        let phase2 = self.phase >= 1.5;

        let need_pickup = near && !has && !phase2;
        let short_ready = phase0 && has && charge >= SHORT_CHARGE;
        let need_burn = phase1 && has && stam > BURN_BELOW;
        let bait_hold = phase1 && has && !need_burn;

        let interact = need_pickup || (phase0 && has && !short_ready) || bait_hold;
        let sprint = need_burn;

        let wp = sticky_wp(me.y, &mut self.lap_wp);
        let lap = Vec2::new(0.0, wp);
        let kick_aim = me + Vec2::new(0.0, 8.0);

        let move_to = if phase2 {
            me
        } else if phase1 {
            lap
        } else if short_ready {
            kick_aim
        } else if has {
            me
        } else {
            ball
        };

        out.commands[0] = BrainCommand {
            move_to,
            sprint,
            interact,
        };

        self.prev_has = has;
        self.prev_charge = charge;
        out
    }
}

/// Away: wait for Home kick + Home stam < 0.80, walk-tackle, then N/S carry.
#[derive(Debug, Default)]
pub struct Test2Brain {
    seen_home: bool,
    home_kicked: bool,
    won_once: bool,
    lap_wp: f32,
}

impl TeamBrain for Test2Brain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();
        park_others(&mut out, api);

        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let me = api.get_transform("Team Player 1").unwrap_or(Vec2::ZERO);
        let has = api.get_bool("Team Player 1 Has Ball").unwrap_or(false);
        let near = api.get_bool("Is Ball Nearby Team Player 1").unwrap_or(false);
        let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false);
        let home_stam = api
            .get_float("Opponent Player 1 Stamina")
            .or_else(|| {
                if opp_has {
                    api.get_float("Ball Carrier Stamina")
                } else {
                    None
                }
            })
            .unwrap_or(1.0);

        if opp_has {
            self.seen_home = true;
        }
        if self.seen_home && !opp_has {
            self.home_kicked = true;
        }
        if has {
            self.won_once = true;
        }

        let ready = self.home_kicked && opp_has && home_stam < AWAY_ENGAGE_STAM;
        let chasing = ready && !self.won_once;
        let won = self.won_once;

        let wp = sticky_wp(me.y, &mut self.lap_wp);
        let lap = Vec2::new(0.0, wp);

        let move_to = if chasing {
            ball
        } else if won {
            lap
        } else {
            me
        };
        let interact = chasing && (near || opp_has);

        out.commands[0] = BrainCommand {
            move_to,
            sprint: false,
            interact,
        };
        out
    }
}
