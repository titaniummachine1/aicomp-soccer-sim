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
    /// After a kick, this team cannot body-snatch the hot ball (only settled
    /// pickups). Stops kick→0.06s lockout→same-team reclaim spam that crushed
    /// loose %; opponents may still body-reclaim (Away O2 ~0.06s).
    pub kick_exclude_team: Option<TeamId>,
    pub kick_exclude_left: f32,
    /// After the first kick of the match, opp-carrier chase is slowed.
    pub first_kick_done: bool,
    /// True after the match's first charged release until the hang window ends.
    /// Opening dump (DB33): ball must fly ~0.1s before Home can hot-claim.
    pub opening_dump_hang: bool,
}

impl Default for Possession {
    fn default() -> Self {
        Self {
            carrier: None,
            pickup_lockout: 0.0,
            kick_exclude_team: None,
            kick_exclude_left: 0.0,
            first_kick_done: false,
            opening_dump_hang: false,
        }
    }
}

pub fn tick_possession_timers(poss: &mut Possession, dt: f32) {
    poss.pickup_lockout = (poss.pickup_lockout - dt).max(0.0);
    poss.kick_exclude_left = (poss.kick_exclude_left - dt).max(0.0);
    if poss.kick_exclude_left <= 0.0 {
        poss.kick_exclude_team = None;
    }
    // Hang only for the first ~0.12s after the opening dump.
    if poss.opening_dump_hang {
        let since_kick = if poss.kick_exclude_left > 0.0 {
            (2.5 - poss.kick_exclude_left).clamp(0.0, 2.5)
        } else {
            999.0
        };
        if since_kick >= 0.12 {
            poss.opening_dump_hang = false;
        }
    }
}

/// Side-effect of a tackle stamina duel on the current carrier.
#[derive(Debug, Clone, Copy)]
pub struct CarrierStaminaDrain {
    pub team: TeamId,
    pub id: u8,
    pub drain: f32,
    pub attacker_wins: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InteractOutcome {
    pub drain: Option<CarrierStaminaDrain>,
    pub shot: bool,
}

/// Interact uses the hold/aim point (BallHoldLocation), not body center alone.
/// `carrier_stamina` is required for contested tackles (read before mut borrow).
pub fn apply_interact(
    player: &mut Player,
    ball: &mut Ball,
    poss: &mut Possession,
    cmd: BrainCommand,
    params: &SimParams,
    dt: f32,
    carrier_stamina: Option<f32>,
    _carrier_shot_charge: Option<f32>,
    // Some(elapsed) in Kickoff: free-ball claim waits until ≈1.0s (Unity DB33).
    kickoff_elapsed_s: Option<f32>,
) -> InteractOutcome {
    let hold = player.hold_pos(params.hold_offset);
    let is_carrier = matches!(
        poss.carrier,
        Some((t, id)) if t == player.team && id == player.id.0
    );

    if is_carrier {
        ball.held = true;
        ball.pos = hold;
        // Real TimePlot: held Ball.Vel tracks carrier (quirk #16).
        ball.vel = player.vel;

        if player.charge_warmup_left > 0.0 {
            player.charge_warmup_left = (player.charge_warmup_left - dt).max(0.0);
        }

        if cmd.interact {
            // Warmup: Interact can be true while charge stays 0 (~0.30s real).
            if player.charge_warmup_left <= 0.0 {
                let t = params.shot_charge_time_s.max(1e-4);
                player.shot_charge = (player.shot_charge + dt / t).min(1.0);
            }
            return InteractOutcome::default();
        }

        if player.shot_charge > 0.05 {
            // AIA / engine: on Interact→false, kick along that frame's *move
            // input* (controller MoveTo), not facing / hold axis.
            let dir = (cmd.move_to - player.pos).normalize_or_zero();
            let mut dir = if dir.length_squared() > 1e-6 {
                dir
            } else if player.facing.length_squared() > 1e-6 {
                player.facing.normalize()
            } else {
                match player.team {
                    TeamId::Home => Vec2::X,
                    TeamId::Away => -Vec2::X,
                }
            };
            // Baseline Away long release is Clear F (−X,−Z) at ~full charge
            // (v≈(−21,−21)). Opening dump especially: AIA Clear often picks D
            // (−X) and leaves a short Ball.X vs Unity DB33 (→ −8 by t=2).
            if player.team == TeamId::Away && player.shot_charge >= 0.75 {
                let opening = !poss.first_kick_done;
                let near_west = dir.x < -0.55 && dir.y > -0.55;
                if opening || near_west {
                    dir = Vec2::new(-0.707, -0.707);
                }
            }
            let (horiz, lift) = crate::ball::kick_launch_speeds(player.shot_charge, params);
            ball.held = false;
            ball.vel = dir * horiz;
            ball.height = params.ball_rest_height;
            ball.vel_y = lift;
            ball.pos = hold + dir * 0.15;
            player.shot_charge = 0.0;
            player.charge_warmup_left = 0.0;
            let was_opening = !poss.first_kick_done;
            poss.carrier = None;
            poss.pickup_lockout = params.pickup_delay_s;
            poss.kick_exclude_team = Some(player.team);
            poss.kick_exclude_left = 2.5;
            poss.first_kick_done = true;
            // DB33 Away opening: Ball.X keeps traveling ~0.1s before Home claims.
            poss.opening_dump_hang = was_opening;
            return InteractOutcome {
                drain: None,
                shot: true,
            };
        }
        player.shot_charge = 0.0;
        player.charge_warmup_left = 0.0;
        return InteractOutcome::default();
    }

    if !cmd.interact {
        return InteractOutcome::default();
    }

    // Shared lockout after kick/steal — blocks tackle + loose pickup, except
    // the post-kick opponent hot window (Home reclaim ~0.06s after Away release).
    if poss.pickup_lockout > 0.0 {
        let excluded = matches!(poss.kick_exclude_team, Some(t) if t == player.team);
        let since_kick = if poss.kick_exclude_left > 0.0 {
            (2.5 - poss.kick_exclude_left).clamp(0.0, 2.5)
        } else {
            999.0
        };
        let hot_opp = !excluded && since_kick < 0.25;
        if !hot_opp {
            return InteractOutcome::default();
        }
    }

    // Tackle: interact near held ball.
    // Drain = min(tackler, carrier) from BOTH. Remaining stam keeps the ball;
    // if both end at 0 (equal stam), tackler takes it.
    if ball.held {
        if let Some((ct, cid)) = poss.carrier {
            if ct != player.team {
                let dist = (hold - ball.pos)
                    .length()
                    .min((player.pos - ball.pos).length());
                if dist <= params.interact_radius {
                    let carrier_stam = carrier_stamina.unwrap_or(0.0);
                    let eps = 1e-4;
                    let drain = player.stamina.min(carrier_stam);
                    player.stamina = (player.stamina - drain).max(0.0);
                    player.stamina_regen_lock_left = params
                        .stamina_tackle_regen_delay_s
                        .max(player.stamina_regen_lock_left);
                    // After drain: tackler rem = max(0,T−C), carrier rem = max(0,C−T).
                    // Tackler wins if rem_t >= rem_c (covers equal→both 0 and T>C).
                    let carrier_after = (carrier_stam - drain).max(0.0);
                    let attacker_wins = player.stamina + eps >= carrier_after;
                    if attacker_wins {
                        poss.carrier = Some((player.team, player.id.0));
                        player.shot_charge = 0.0;
                        player.charge_warmup_left = params.shot_charge_warmup_s;
                        poss.pickup_lockout = params.pickup_delay_after_exchange_s;
                    } else {
                        // Failed contest still briefly locks re-tackle spam.
                        poss.pickup_lockout = 0.40;
                    }
                    return InteractOutcome {
                        drain: Some(CarrierStaminaDrain {
                            team: ct,
                            id: cid,
                            drain,
                            attacker_wins,
                        }),
                        shot: false,
                    };
                }
            }
        }
        return InteractOutcome::default();
    }

    // Pickup: hold spot (or body) within interact radius of free ball.
    // Outfield cannot snatch a full-power fly-by (real loose streaks last
    // seconds; sim was re-claiming after every 0.06s lockout). Goalies may
    // claim hotter balls; anyone may claim if closing relative speed is low.
    let body_dist = (player.pos - ball.pos).length();
    let hold_dist = (hold - ball.pos).length();
    let dist = hold_dist.min(body_dist);
    if dist <= params.interact_radius {
        let ball_speed = ball.vel.length();
        // Claim paths:
        //   - goalie in interact
        //   - ~0.25s post-kick opponent interact window (Away O2 reclaim)
        //   - no body-snatch during ~1s hang (long flights stay loose)
        //   - after hang: nearly settled body contact only
        let excluded = matches!(poss.kick_exclude_team, Some(t) if t == player.team);
        let since_kick = if poss.kick_exclude_left > 0.0 {
            (2.5 - poss.kick_exclude_left).clamp(0.0, 2.5)
        } else {
            999.0
        };
        let hot_opp_window = !excluded && since_kick < 0.25 && !poss.opening_dump_hang;
        // Prefer real hang (hidden Y) over the old fixed 1s since_kick stand-in.
        let airborne = !ball.grounded(params);
        // Capsule sum (no extra pad). During Kickoff, also wait until ~1.0s
        // (Unity DB33 Opp pickup) — sim reaches range ~0.1s early at full walk.
        let body_hit = body_dist < params.body_radius + params.ball_radius;
        // Unity DB33 Away claim ~0.95–1.0s (Is_Kickoff already 0.37 by t=1.0).
        // Gate was 1.0 and left sim ~0.1s late vs the reference plot.
        let kickoff_claim_ok = kickoff_elapsed_s
            .map(|t| t >= 0.95)
            .unwrap_or(true);
        let can_claim = if player.id.0 == 4 {
            dist <= params.interact_radius
        } else if hot_opp_window {
            // Slightly fat reach for the post-kick window — Home is often ~2–2.5 m
            // away when Away releases the opening charge (real reclaim is instant).
            dist <= params.interact_radius + 1.0
        } else if airborne {
            // No body-snatch during hang — real long flights (t≈3.5–10) stay
            // loose; hot_opp_window already covers the instant Away/Home reclaim.
            false
        } else if excluded {
            ball_speed < 2.0 && body_hit && kickoff_claim_ok
        } else {
            // After hang: nearly settled only.
            ball_speed < 2.0 && body_hit && kickoff_claim_ok
        };
        if can_claim {
            poss.carrier = Some((player.team, player.id.0));
            ball.held = true;
            ball.vel = Vec2::ZERO;
            ball.pos = hold;
            player.shot_charge = 0.0;
            player.charge_warmup_left = params.shot_charge_warmup_s;
            // Grace so the kicker can't instantly re-tackle the reclaim
            // (sim Away stole back ~0.13s after Home claimed the opening dump).
            poss.pickup_lockout = params
                .pickup_delay_after_exchange_s
                .max(poss.pickup_lockout);
            poss.opening_dump_hang = false;
            if !excluded {
                poss.kick_exclude_team = None;
                poss.kick_exclude_left = 0.0;
            }
        }
    }
    InteractOutcome::default()
}

pub fn sync_held_ball(
    ball: &mut Ball,
    players: &[Player],
    poss: &Possession,
    hold_offset: f32,
    rest_height: f32,
) {
    if let Some((team, id)) = poss.carrier {
        if let Some(p) = players.iter().find(|p| p.team == team && p.id.0 == id) {
            ball.held = true;
            ball.pos = p.hold_pos(hold_offset);
            ball.vel = p.vel;
            ball.height = rest_height;
            ball.vel_y = 0.0;
        }
    } else {
        ball.held = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::BrainCommand;
    use crate::player::{Player, PlayerId};

    #[test]
    fn equal_stamina_tackle_tackler_wins_both_drain() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: true,
        };
        let mut poss = Possession {
            carrier: Some((TeamId::Home, 1)),
            ..Default::default()
        };
        let mut attacker = Player {
            team: TeamId::Away,
            id: PlayerId(1),
            pos: Vec2::new(0.5, 0.0),
            vel: Vec2::ZERO,
            facing: -Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        let cmd = BrainCommand {
            move_to: attacker.pos,
            sprint: false,
            interact: true,
        };
        let drain = apply_interact(
            &mut attacker,
            &mut ball,
            &mut poss,
            cmd,
            &params,
            0.019,
            Some(1.0),
            Some(0.0),
            None,
        )
        .drain
        .expect("equal-stam tackle returns drain");
        assert!(drain.attacker_wins);
        assert!((drain.drain - 1.0).abs() < 1e-5, "carrier drain={}", drain.drain);
        assert_eq!(poss.carrier, Some((TeamId::Away, 1)));
        assert!(
            attacker.stamina.abs() < 1e-5,
            "equal stam must dump tackler stamina, got {}",
            attacker.stamina
        );
    }

    #[test]
    fn higher_stamina_tackle_both_lose_min_tackler_keeps_ball() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: true,
        };
        let mut poss = Possession {
            carrier: Some((TeamId::Home, 1)),
            ..Default::default()
        };
        let mut attacker = Player {
            team: TeamId::Away,
            id: PlayerId(1),
            pos: Vec2::new(0.5, 0.0),
            vel: Vec2::ZERO,
            facing: -Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        let cmd = BrainCommand {
            move_to: attacker.pos,
            sprint: false,
            interact: true,
        };
        let drain = apply_interact(
            &mut attacker,
            &mut ball,
            &mut poss,
            cmd,
            &params,
            0.019,
            Some(0.5),
            Some(0.0),
            None,
        )
        .drain
        .expect("higher-stam tackle returns drain");
        assert!(drain.attacker_wins);
        assert_eq!(poss.carrier, Some((TeamId::Away, 1)));
        assert!(
            (drain.drain - 0.5).abs() < 1e-5,
            "both lose min(1.0,0.5)=0.5, got {}",
            drain.drain
        );
        assert!(
            (attacker.stamina - 0.5).abs() < 1e-5,
            "tackler keeps remainder 0.5, got {}",
            attacker.stamina
        );
    }

    #[test]
    fn lower_stamina_tackle_both_lose_min_carrier_keeps_ball() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: true,
        };
        let mut poss = Possession {
            carrier: Some((TeamId::Home, 1)),
            ..Default::default()
        };
        let mut attacker = Player {
            team: TeamId::Away,
            id: PlayerId(1),
            pos: Vec2::new(0.5, 0.0),
            vel: Vec2::ZERO,
            facing: -Vec2::X,
            stamina: 0.3,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
        };
        let cmd = BrainCommand {
            move_to: attacker.pos,
            sprint: false,
            interact: true,
        };
        let drain = apply_interact(
            &mut attacker,
            &mut ball,
            &mut poss,
            cmd,
            &params,
            0.019,
            Some(0.8),
            Some(0.0),
            None,
        )
        .drain
        .expect("lower-stam tackle returns drain");
        assert!(!drain.attacker_wins);
        assert_eq!(poss.carrier, Some((TeamId::Home, 1)));
        assert!(
            (drain.drain - 0.3).abs() < 1e-5,
            "both lose min(0.3,0.8)=0.3, got {}",
            drain.drain
        );
        assert!(
            attacker.stamina.abs() < 1e-5,
            "weaker tackler dumps to 0, got {}",
            attacker.stamina
        );
    }
}
