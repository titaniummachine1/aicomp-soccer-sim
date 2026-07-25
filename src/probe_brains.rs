//! Scripted Test1 / Test2 brains — mirror worldcupteams build_test1/2.py
//! so we can debug tackle flow in-sim before Unity.
//! Also `PerfectControllerBrain`: P1 keyboard only; P2–4 park in team corner.

use bevy::prelude::Vec2;

use crate::api::TeamApi;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use crate::keypress;
use crate::player::PlayerId;

const BURN_BELOW: f32 = 0.65;
const AWAY_ENGAGE_STAM: f32 = 0.80;
const WAYPOINT_Z: f32 = 20.0;
const FLIP_AT: f32 = 16.0;
const SHORT_CHARGE: f32 = 0.35;
/// WASD MoveTo step (meters) — matches basic controller ±2 floats.
const KB_STEP: f32 = 2.0;
/// Fallback park if Upper Corner Team Side is missing.
const PARK_FALLBACK_X: f32 = 38.0;
const PARK_FALLBACK_Z: f32 = 22.0;

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

fn park_corner(api: &TeamApi) -> Vec2 {
    api.get_vector3("Upper Corner Team Side")
        .flatten()
        .unwrap_or_else(|| {
            let x = match api.team {
                TeamId::Home => -PARK_FALLBACK_X,
                TeamId::Away => PARK_FALLBACK_X,
            };
            Vec2::new(x, PARK_FALLBACK_Z)
        })
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

/// Viewer keyboard: WASD MoveTo (face that way), Shift sprint, E interact.
/// P2–4 sit in Upper Corner Team Side.
#[derive(Debug, Default)]
pub struct PerfectControllerBrain;

impl TeamBrain for PerfectControllerBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();
        let park = park_corner(api);
        for i in 1..4 {
            out.commands[i] = BrainCommand {
                move_to: park,
                sprint: false,
                interact: false,
            };
        }

        let me = api.get_transform("Team Player 1").unwrap_or(Vec2::ZERO);
        // Board-relative (same as basic controller): W/S = ±Z (sim y), A/D = ±X.
        let mut d = Vec2::ZERO;
        if keypress::is_pressed("W") {
            d.y += KB_STEP;
        }
        if keypress::is_pressed("S") {
            d.y -= KB_STEP;
        }
        if keypress::is_pressed("A") {
            d.x -= KB_STEP;
        }
        if keypress::is_pressed("D") {
            d.x += KB_STEP;
        }
        out.commands[0] = BrainCommand {
            move_to: me + d,
            sprint: keypress::is_pressed("LeftShift"),
            interact: keypress::is_pressed("E"),
        };
        out
    }
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

/// Scripted kick parity driver: blank opponents, P1 fires a fixed kick sequence.
///
/// Requires [`setup_kick_routine_harness`] — starts in Play with ball at center,
/// not kickoff. Sequence: +45° full → pickup → center → −45° full → pickup →
/// center → −45° half → park.
#[derive(Debug, Default)]
pub struct KickRoutineBrain {
    phase: u8,
    prev_has: bool,
}

impl KickRoutineBrain {
    fn charge_of(api: &TeamApi, has: bool) -> f32 {
        if !has {
            return 0.0;
        }
        api.get_float("Teammate 1 Shot Charge")
            .or_else(|| api.get_float("Ball Carrier Shot Charge"))
            .unwrap_or(0.0)
    }

    fn aim_dir(deg: f32) -> Vec2 {
        let rad = deg.to_radians();
        Vec2::new(rad.cos(), rad.sin())
    }
}

impl TeamBrain for KickRoutineBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();
        let park = park_corner(api);
        for i in 1..4 {
            out.commands[i] = BrainCommand {
                move_to: park,
                sprint: false,
                interact: false,
            };
        }

        let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
        let me = api.get_transform("Team Player 1").unwrap_or(Vec2::ZERO);
        let has = api.get_bool("Team Player 1 Has Ball").unwrap_or(false);
        let near = api.get_bool("Is Ball Nearby Team Player 1").unwrap_or(false);
        let charge = Self::charge_of(api, has);
        let released = self.prev_has && !has && charge <= 0.05;
        let center = Vec2::ZERO;
        let at_center = me.distance(center) < 1.25;

        if released && matches!(self.phase, 0 | 2 | 4) {
            self.phase += 1;
        } else if has && !self.prev_has && matches!(self.phase, 1 | 3 | 5) {
            self.phase += 1;
        }

        let (aim_deg, charge_need, sprint_release) = match self.phase {
            0 => (45.0_f32, 0.97_f32, true),
            2 => (-45.0_f32, 0.97_f32, true),
            4 => (-45.0_f32, 0.50_f32, false),
            _ => (0.0, 1.0, false),
        };

        out.commands[0] = match self.phase {
            0 | 2 | 4 if has => {
                let dir = Self::aim_dir(aim_deg);
                if !at_center {
                    BrainCommand {
                        move_to: center,
                        sprint: false,
                        interact: true,
                    }
                } else if charge >= charge_need {
                    BrainCommand {
                        move_to: me + dir * 10.0,
                        sprint: sprint_release,
                        interact: false,
                    }
                } else {
                    BrainCommand {
                        move_to: me + dir * 10.0,
                        sprint: false,
                        interact: true,
                    }
                }
            }
            1 | 3 | 5 => BrainCommand {
                move_to: ball,
                sprint: false,
                interact: near && !has,
            },
            _ => BrainCommand {
                move_to: center,
                sprint: false,
                interact: false,
            },
        };

        self.prev_has = has;
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
