//! Possession: interact at hold spot, held ball follows BallHoldLocation, kick + delay.

use std::cell::Cell;

use bevy::prelude::*;

use crate::ball::Ball;
use crate::brain::{BrainCommand, TeamId};
use crate::params::SimParams;
use crate::player::Player;

// When set (`Some(t)`), `apply_interact` prints which branch ran that tick.
// Used by `debug_tick_trace` — leave `None` in normal runs.
thread_local! {
    pub static TRACE_T: Cell<Option<f32>> = const { Cell::new(None) };
}

fn trace(msg: impl std::fmt::Display) {
    TRACE_T.with(|c| {
        if let Some(t) = c.get() {
            eprintln!("[t={t:.3}] {msg}");
        }
    });
}

#[derive(Resource, Debug, Clone)]
pub struct Possession {
    pub carrier: Option<(TeamId, u8)>,
    pub pickup_lockout: f32,
    /// Retained for opening-kick timing/sensor parity. It does not block the
    /// shooter from interacting with a free ball in range.
    pub kick_exclude_shooter: Option<(TeamId, u8)>,
    pub kick_exclude_left: f32,
    /// True after the **match's** first charged release. Gates opening-only
    /// scripts (Away Clear-F dump, Home 0.45 press). Kept across goal/whistle.
    pub first_kick_done: bool,
    /// True after the **current kickoff's** first release. Resets each
    /// goal/whistle. Gates receiving-team no-tackle until that touch.
    pub kickoff_touch_done: bool,
    /// Opening dump (DB33): ball must fly ~0.1s before Home can hot-claim.
    pub opening_dump_hang: bool,
    /// Ball-side flag: only the opening dump allows the fat opponent hot-reclaim
    /// window. Mid-game kicks keep the ball lockout (no same-tick snatch).
    pub opening_hot_reclaim: bool,
}

impl Default for Possession {
    fn default() -> Self {
        Self {
            carrier: None,
            pickup_lockout: 0.0,
            kick_exclude_shooter: None,
            kick_exclude_left: 0.0,
            first_kick_done: false,
            kickoff_touch_done: false,
            opening_dump_hang: false,
            opening_hot_reclaim: false,
        }
    }
}

/// Clear post-kick contest state for a fresh kickoff (goal or whistle).
/// Stamina stays on players. Match-level `first_kick_done` is **kept** so
/// opening press/dump scripts do not re-arm every round (that made Home
/// kickoffs unwinnable — Away ran ~16.8s goal loops to 10–1).
pub fn reset_possession_for_kickoff(poss: &mut Possession) {
    poss.carrier = None;
    poss.pickup_lockout = 0.0;
    poss.kick_exclude_shooter = None;
    poss.kick_exclude_left = 0.0;
    poss.kickoff_touch_done = false;
    poss.opening_dump_hang = false;
    poss.opening_hot_reclaim = false;
}

pub fn tick_possession_timers(poss: &mut Possession, dt: f32) {
    poss.pickup_lockout = (poss.pickup_lockout - dt).max(0.0);
    poss.kick_exclude_left = (poss.kick_exclude_left - dt).max(0.0);
    if poss.kick_exclude_left <= 0.0 {
        poss.kick_exclude_shooter = None;
        poss.opening_hot_reclaim = false;
    }
    // Hang only for the first ~0.12s after the opening dump.
    if poss.opening_dump_hang {
        let since_kick = if poss.kick_exclude_left > 0.0 {
            (3.0 - poss.kick_exclude_left).clamp(0.0, 3.0)
        } else {
            999.0
        };
        if since_kick >= 0.12 {
            poss.opening_dump_hang = false;
        }
    }
    // Opening hot reclaim dies with the 0.25s post-kick window.
    if poss.opening_hot_reclaim {
        let since_kick = if poss.kick_exclude_left > 0.0 {
            (3.0 - poss.kick_exclude_left).clamp(0.0, 3.0)
        } else {
            999.0
        };
        if since_kick >= 0.25 {
            poss.opening_hot_reclaim = false;
        }
    }
}

/// Side-effect of a tackle stamina duel on the current carrier.
#[derive(Debug, Clone, Copy)]
pub struct CarrierStaminaDrain {
    pub team: TeamId,
    pub id: u8,
    pub drain: f32,
    /// True when the tackler (Interact without ball) wins and takes possession.
    pub attacker_wins: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InteractOutcome {
    pub drain: Option<CarrierStaminaDrain>,
    pub shot: bool,
}

/// Interact uses the nearer of BallHoldLocation and the player's body center.
/// Being inside the body radius is therefore still a valid interaction; the
/// ball is never physically pushed by that overlap.
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
    // Retained for callers; the claim path no longer gates on it.
    _kickoff_elapsed_s: Option<f32>,
) -> InteractOutcome {
    let hold = player.hold_pos_playable(params);
    // Interact is an IMPULSE. Holding it charges a shot (handled in the carrier
    // branch below, which is genuinely level-triggered), but a claim or a
    // tackle fires only on the press. Level-triggering them let a graph that
    // simply pinned Interact true re-attempt a steal every single tick, which
    // the real game does not allow.
    // Interact is an IMPULSE for claims and tackles: one only counts if the
    // PREVIOUS tick was not pressing. Holding it down produces exactly one, the
    // same anti-bunnyhop latch games use for jump. Charging a shot is
    // unaffected — that is level-triggered and lives in the carrier branch.
    let impulse = if params.interact_is_impulse {
        cmd.interact && !player.interact_held
    } else {
        cmd.interact
    };
    player.interact_held = cmd.interact;
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
            let charge = player.shot_charge;
            let (horiz, lift) = crate::ball::kick_launch_speeds(charge, params);
            ball.held = false;
            ball.vel = dir * horiz;
            ball.height = params.ball_rest_height;
            ball.vel_y = lift;
            ball.pos = hold + dir * 0.15;
            player.shot_charge = 0.0;
            player.charge_warmup_left = 0.0;
            let was_opening = !poss.first_kick_done;
            poss.carrier = None;
            // A kicked ball remains interactable immediately. There is no
            // artificial post-kick cooldown; tackle exchange lockouts below
            // are separate and only apply after a contested steal.
            poss.kick_exclude_shooter = Some((player.team, player.id.0));
            poss.kick_exclude_left = 3.0;
            poss.first_kick_done = true;
            poss.kickoff_touch_done = true;
            // DB33 Away opening: Ball.X keeps traveling ~0.1s before Home claims.
            poss.opening_dump_hang = was_opening;
            poss.opening_hot_reclaim = was_opening;
            trace(format!(
                "KICK {:?} P{} charge={charge:.2} horiz={horiz:.1} dir=({:.2},{:.2}) ball→({:.1},{:.1})",
                player.team,
                player.id.0,
                dir.x,
                dir.y,
                ball.pos.x,
                ball.pos.y
            ));
            return InteractOutcome {
                drain: None,
                shot: true,
            };
        }
        if player.shot_charge > 0.0 {
            trace(format!(
                "CHARGE_CLEAR_NO_KICK {:?} P{} charge was {:.2} (Interact false, charge≤0.05)",
                player.team, player.id.0, player.shot_charge
            ));
        }
        player.shot_charge = 0.0;
        player.charge_warmup_left = 0.0;
        return InteractOutcome::default();
    }

    if !impulse {
        return InteractOutcome::default();
    }

    let since_kick = if poss.kick_exclude_left > 0.0 {
        (3.0 - poss.kick_exclude_left).clamp(0.0, 3.0)
    } else {
        999.0
    };
    let shooter_team = poss.kick_exclude_shooter.map(|(t, _)| t);

    // Tackle/pickup exchange lockout. Opening dump alone may bypass via
    // hot_opp (opponent reclaim after hang).
    if poss.pickup_lockout > 0.0 {
        let hot_opp = shooter_team.is_some_and(|t| t != player.team)
            && since_kick < 0.25
            && poss.opening_hot_reclaim
            && !poss.opening_dump_hang;
        if !hot_opp {
            return InteractOutcome::default();
        }
    }

    // Contested tackle (same rules in Scenario 1 and full match — one code path).
    // `player` here is the tackler (person pressing Interact without the ball).
    // Both lose drain = min(tackler, carrier). Pre-drain stamina decides:
    //   tackler >= carrier → tackler steals (equal / both already 0 → steal)
    //   tackler <  carrier → carrier keeps the excess, tackler loses
    // No special-case block at 0/0 — exchange lockout rate-limits flips. Unity
    // parity: probes/build_tackle_empty_stam.py (confirm ping-pong vs lockout).
    if ball.held {
        if let Some((ct, cid)) = poss.carrier {
            if ct != player.team {
                let dist = (hold - ball.pos)
                    .length()
                    .min((player.pos - ball.pos).length());
                if dist <= params.interact_radius {
                    let tackler_stam = player.stamina;
                    let carrier_stam = carrier_stamina.unwrap_or(0.0);
                    let eps = 1e-4;
                    let drain = tackler_stam.min(carrier_stam);
                    player.stamina = (tackler_stam - drain).max(0.0);
                    player.stamina_regen_lock_left = params
                        .stamina_tackle_regen_delay_s
                        .max(player.stamina_regen_lock_left);
                    let carrier_after = (carrier_stam - drain).max(0.0);
                    // Pre-drain compare; equal (incl. 0/0) → tackler steals.
                    let tackler_wins = tackler_stam + eps >= carrier_stam;
                    if tackler_wins {
                        poss.carrier = Some((player.team, player.id.0));
                        player.shot_charge = 0.0;
                        player.charge_warmup_left = params.shot_charge_warmup_s;
                        let both_spent = player.stamina <= eps && carrier_after <= eps;
                        poss.pickup_lockout = if both_spent {
                            0.55
                        } else {
                            params.pickup_delay_after_exchange_s
                        };
                        trace(format!(
                            "STEAL {:?} P{} from {:?} P{} drain={drain:.2} tackler={tackler_stam:.2} carrier={carrier_stam:.2}",
                            player.team, player.id.0, ct, cid,
                        ));
                    } else {
                        poss.pickup_lockout = 0.40;
                        trace(format!(
                            "TACKLE_FAIL {:?} P{} on {:?} P{} drain={drain:.2} tackler={tackler_stam:.2} carrier={carrier_stam:.2}",
                            player.team, player.id.0, ct, cid,
                        ));
                    }
                    return InteractOutcome {
                        drain: Some(CarrierStaminaDrain {
                            team: ct,
                            id: cid,
                            drain,
                            attacker_wins: tackler_wins,
                        }),
                        shot: false,
                    };
                }
            }
        }
        return InteractOutcome::default();
    }

    // Pickup: XZ interact only — ball height is bounce physics, not a claim gate
    // (Unity catch works mid-air; game is planar for Interact).
    let body_dist = (player.pos - ball.pos).length();
    let hold_dist = (hold - ball.pos).length();
    let dist = hold_dist.min(body_dist);
    if dist <= params.interact_radius {
        // Claim paths:
        //   - goalie in interact (always)
        //   - opening dump only: ~0.25s post-kick opponent hot window (fat)
        //   - else: Interact + XZ in radius (mid-air OK). Shooter alone is
        //     already blocked above; ball lockout blocked same-tick snatch.
        let hot_opp_window = shooter_team.is_some_and(|t| t != player.team)
            && since_kick < 0.25
            && poss.opening_hot_reclaim
            && !poss.opening_dump_hang;
        let can_claim = if player.id.0 == 4 {
            dist <= params.interact_radius
        } else if hot_opp_window {
            // Slightly fat reach for the opening post-kick window — Home is
            // often ~2–2.5 m away when Away releases (real reclaim is instant).
            dist <= params.interact_radius + 1.0
        } else if poss.opening_dump_hang {
            // Opening dump must travel ~0.12s before anyone Interact-claims.
            false
        } else {
            // Mid-air OK: Interact is XZ-only; Y is bounce sim only.
            //
            // NO KICKOFF GATE. This used to require `elapsed >= 0.95s` (exactly
            // 50 ticks) before anyone but a slot-4 keeper could claim a free
            // ball during a kickoff, so the kicking striker stood ON the ball
            // for 50 of the kickoff's 53 ticks unable to pick it up. Nothing
            // supported it: a real-game recording
            // (timeplot_2026-07-26_14-45-28) shows a player spawned on the
            // centre spot carrying by t=0.09, and 50 vs 53 was two constants
            // describing one event. The rule is simply: pick the ball up and
            // play starts; otherwise the kickoff ends on its own after
            // KICKOFF_TICKS.
            true
        };
        if can_claim {
            let speed_before = ball.vel.length();
            let height_before = ball.height;
            poss.carrier = Some((player.team, player.id.0));
            ball.held = true;
            ball.vel = Vec2::ZERO;
            ball.vel_y = 0.0;
            ball.height = params.ball_rest_height;
            ball.pos = hold;
            player.shot_charge = 0.0;
            player.charge_warmup_left = params.shot_charge_warmup_s;
            // Grace so the kicker can't instantly re-tackle the reclaim
            // (sim Away stole back ~0.13s after Home claimed the opening dump).
            poss.pickup_lockout = params
                .pickup_delay_after_exchange_s
                .max(poss.pickup_lockout);
            poss.opening_dump_hang = false;
            poss.opening_hot_reclaim = false;
            poss.kick_exclude_shooter = None;
            poss.kick_exclude_left = 0.0;
            trace(format!(
                "PICKUP {:?} P{} dist={dist:.2} was_speed={speed_before:.1} was_height={height_before:.2}",
                player.team, player.id.0
            ));
        }
    }
    InteractOutcome::default()
}

pub fn sync_held_ball(ball: &mut Ball, players: &[Player], poss: &Possession, params: &SimParams) {
    if let Some((team, id)) = poss.carrier {
        if let Some(p) = players.iter().find(|p| p.team == team && p.id.0 == id) {
            ball.held = true;
            // Project hold into playable AABB (sidelines / solid endlines).
            // Free yaw: facing is unchanged; offset may compress at walls.
            ball.pos = p.hold_pos_playable(params);
            ball.vel = p.vel;
            ball.height = params.ball_rest_height;
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
            interact_held: false,
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
        assert!(
            (drain.drain - 1.0).abs() < 1e-5,
            "carrier drain={}",
            drain.drain
        );
        assert_eq!(poss.carrier, Some((TeamId::Away, 1)));
        assert!(
            attacker.stamina.abs() < 1e-5,
            "equal stam must dump tackler stamina, got {}",
            attacker.stamina
        );
    }

    #[test]
    fn both_empty_stamina_equal_still_steals() {
        // 0/0 is equal pre-drain → tackler wins (no special-case block).
        // Exchange lockout still applies (both_spent → 0.55s).
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
            stamina: 0.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_held: false,
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
            Some(0.0),
            Some(0.0),
            None,
        )
        .drain
        .expect("0/0 duel returns drain");
        assert!(drain.attacker_wins);
        assert!(drain.drain.abs() < 1e-5);
        assert_eq!(poss.carrier, Some((TeamId::Away, 1)));
        assert!(poss.pickup_lockout >= 0.55);
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
            interact_held: false,
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
            interact_held: false,
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

    #[test]
    fn midair_interact_claims_xz_ignores_height() {
        // Height is bounce-only; Interact must catch lofted balls in XZ range.
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::new(1.0, 0.0),
            vel: Vec2::new(20.0, 0.0),
            height: params.ball_rest_height + 2.5,
            vel_y: 4.0,
            held: false,
        };
        let mut poss = Possession {
            kick_exclude_shooter: Some((TeamId::Home, 1)),
            kick_exclude_left: 2.0,
            first_kick_done: true,
            ..Default::default()
        };
        let mut mate = Player {
            team: TeamId::Home,
            id: PlayerId(2),
            pos: Vec2::new(1.2, 0.1),
            vel: Vec2::ZERO,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_held: false,
        };
        let cmd = BrainCommand {
            move_to: mate.pos,
            sprint: false,
            interact: true,
        };
        apply_interact(
            &mut mate, &mut ball, &mut poss, cmd, &params, 0.019, None, None, None,
        );
        assert_eq!(poss.carrier, Some((TeamId::Home, 2)));
        assert!(ball.held);
        assert!(ball.grounded(&params));
    }

    #[test]
    fn pickup_works_inside_body_radius_when_hold_point_is_outside() {
        let mut params = SimParams::default();
        params.hold_offset = 0.8;
        let mut ball = Ball {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: false,
        };
        let mut poss = Possession::default();
        let mut player = Player {
            team: TeamId::Home,
            id: PlayerId(4),
            pos: Vec2::new(params.interact_radius - 0.05, 0.0),
            vel: Vec2::ZERO,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_held: false,
        };
        let move_to = player.pos;

        apply_interact(
            &mut player,
            &mut ball,
            &mut poss,
            BrainCommand {
                move_to,
                sprint: false,
                interact: true,
            },
            &params,
            0.019,
            None,
            None,
            None,
        );

        assert_eq!(poss.carrier, Some((TeamId::Home, 4)));
        assert!(ball.held);
    }

    #[test]
    fn shooter_can_repickup_while_exclude_timer_is_active() {
        // The timer remains for opening-kick timing, but it must not block
        // normal Interact pickup by the player who just kicked.
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::new(0.5, 0.0),
            vel: Vec2::new(15.0, 0.0),
            height: params.ball_rest_height + 1.0,
            vel_y: 2.0,
            held: false,
        };
        let mut poss = Possession {
            kick_exclude_shooter: Some((TeamId::Home, 1)),
            kick_exclude_left: 2.5,
            first_kick_done: true,
            ..Default::default()
        };
        let mut shooter = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_held: false,
        };
        let cmd = BrainCommand {
            move_to: Vec2::X,
            sprint: false,
            interact: true,
        };
        apply_interact(
            &mut shooter,
            &mut ball,
            &mut poss,
            cmd,
            &params,
            0.019,
            None,
            None,
            None,
        );
        assert_eq!(poss.carrier, Some((TeamId::Home, 1)));
        assert!(ball.held);
    }
}
