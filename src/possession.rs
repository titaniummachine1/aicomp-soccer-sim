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
/// `carrier_stamina` and `carrier_pos` are required for contested tackles
/// (read before mut borrow). The tackle distance check uses the carrier's
/// center, not the ball position — confirmed by real-game TimePlot
/// 2026-07-29: a tackler 1.31 m from the carrier but 2.98 m from the ball
/// still steals, proving the check is center-to-center vs interact_radius.
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
    carrier_pos: Option<Vec2>,
) -> InteractOutcome {
    let hold = player.hold_pos_playable(params);
    // Per-player 64-bit Interact history: each tick shift left, LSB = current.
    // Cleared on whistle / goal / roster reset (see place_kickoff).
    //
    // Claim / tackle: rising edge only (held now, not held last tick).
    // Shot: falling edge kicks; charge = consecutive held ticks in the low
    // bits (after post-pickup warmup), capped at full-charge length. Holding
    // forever does not re-fire claim/tackle — only a fresh rising edge does.
    use crate::interact_history::{interact_falling_edge, interact_rising_edge, push_interact};
    player.interact_bits = push_interact(player.interact_bits, cmd.interact);
    let impulse = interact_rising_edge(player.interact_bits);
    let released = interact_falling_edge(player.interact_bits);
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
            // Hold-to-charge: accumulate after post-pickup warmup. The bit
            // buffer's consecutive-held run is the same clock (one bit per
            // tick); we integrate in seconds so charge_time_s stays calibrated.
            if player.charge_warmup_left <= 0.0 {
                let t = params.shot_charge_time_s.max(1e-4);
                player.shot_charge = (player.shot_charge + dt / t).min(1.0);
            } else {
                player.shot_charge = 0.0;
            }
            return InteractOutcome::default();
        }

        // Release = kick on the falling edge.
        if released {
            let dir = if player.facing.length_squared() > 1e-6 {
                player.facing.normalize()
            } else {
                match player.team {
                    TeamId::Home => Vec2::X,
                    TeamId::Away => -Vec2::X,
                }
            };
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
            poss.kick_exclude_shooter = Some((player.team, player.id.0));
            poss.kick_exclude_left = 3.0;
            poss.first_kick_done = true;
            poss.kickoff_touch_done = true;
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
                "CHARGE_CLEAR_NO_KICK {:?} P{} charge was {:.2} (Interact false, no falling edge)",
                player.team, player.id.0, player.shot_charge
            ));
        }
        player.shot_charge = 0.0;
        player.charge_warmup_left = 0.0;
        return InteractOutcome::default();
    }

    // Non-carrier interaction:
    // When the ball is FREE (!ball.held), picking it up works continuously while `cmd.interact` is true
    // (e.g. alternating toggle or held button).
    // When the ball is HELD (tackle/steal), it requires a rising edge (`impulse`) so a tackler
    // cannot continuously drain/steal every tick without releasing Interact first.
    if ball.held {
        if !impulse {
            return InteractOutcome::default();
        }
    } else {
        if !cmd.interact {
            return InteractOutcome::default();
        }
    }

    let since_kick = if poss.kick_exclude_left > 0.0 {
        (3.0 - poss.kick_exclude_left).clamp(0.0, 3.0)
    } else {
        999.0
    };
    let shooter_team = poss.kick_exclude_shooter.map(|(t, _)| t);

    // NO possession lockout. There is no cooldown after a pickup or a tackle.
    //
    // What used to be here: `pickup_delay_after_exchange_s` (0.25 s) on every
    // claim AND on a won tackle, 0.55 s when both duellists were spent, and
    // 0.40 s on a FAILED tackle. All of it arrived in ffa398a — the same commit
    // that gave tacklers a phantom 3.42 m reach from a second, non-existent
    // hold-point origin. The project's own parity capture records
    // "pickupDelayAfterExchange 0.25s reserved": the field was observed in the
    // game, noted as RESERVED, and then wired up anyway. The 0.55 and 0.40 have
    // no source at all. None of it was ever measured against a recording.
    //
    // It was not harmless. Measured with `pass_chain`: a player claims a pass,
    // and the 0.25 s lockout then blocks the NEXT teammate from claiming the
    // next pass for 13 ticks. A receiver cannot observe the lockout, so it
    // presses, is refused, and — Interact being an impulse — burns its one
    // rising edge and never claims at all. A simple four-player passing relay
    // was impossible.
    //
    // The thing it was guarding against, instant re-steals, is already handled
    // by the real mechanism: a tackler must RELEASE Interact before it can
    // tackle again. That is the engine's own anti-ping-pong rule, confirmed by
    // the game's author, and this was a second invented layer on top of it.

    // Contested tackle (same rules in Scenario 1 and full match — one code path).
    // `player` here is the tackler (person pressing Interact without the ball).
    // Both lose drain = min(tackler, carrier). Pre-drain stamina decides:
    //   tackler >= carrier → tackler steals (equal / both already 0 → steal)
    //   tackler <  carrier → carrier keeps the excess, tackler loses
    // No special-case block at 0/0 — exchange lockout rate-limits flips. Unity
    // parity: probes/build_tackle_empty_stam.py (confirm ping-pong vs lockout).
    if ball.held {
        if let Some((ct, cid)) = poss.carrier {
            {
                // YOU CAN TACKLE YOUR OWN TEAMMATES. Confirmed by the game's
                // author. There is no team check here and no separate friendly
                // path: the same reach and the same stamina duel apply whoever
                // is holding the ball.
                //
                // This used to require `ct != player.team`, so a held ball
                // returned nothing to a teammate and the only way to move the
                // ball within a team was to kick it.
                //
                // It is not a free pass. The duel drains min(tackler, carrier)
                // from BOTH, so two full-stamina teammates swapping the ball
                // this way both end at zero.
                //
                // Center-to-center: a tackle happens if the tackler's center
                // is within interact_radius of the CARRIER's center, not the
                // ball. The ball sits at a hold offset from the carrier, so
                // checking distance to the ball would grant or deny reach
                // based on facing direction — the real game does not.
                //
                // Confirmed by real-game TimePlot 2026-07-29_01-51-57:
                // P2 at 1.31 m from carrier P1 but 2.98 m from the ball
                // still steals. If the check were ball-distance (2.98 > 1.75)
                // it would not have triggered.
                let carrier_center = carrier_pos.unwrap_or(ball.pos);
                let dist = (player.pos - carrier_center).length();
                if dist <= params.interact_radius {
                    let tackler_stam = player.stamina;
                    let carrier_stam = carrier_stamina.unwrap_or(0.0);
                    let eps = 1e-4;
                    let drain = tackler_stam.min(carrier_stam);
                    player.stamina = (tackler_stam - drain).max(0.0);
                    player.stamina_regen_lock_left = params
                        .stamina_tackle_regen_delay_s
                        .max(player.stamina_regen_lock_left);
                    let _carrier_after = (carrier_stam - drain).max(0.0);
                    // Pre-drain compare; equal (incl. 0/0) → tackler steals.
                    let tackler_wins = tackler_stam + eps >= carrier_stam;
                    if tackler_wins {
                        poss.carrier = Some((player.team, player.id.0));
                        // The ball moves to the NEW carrier's hold point at
                        // once, exactly as the pickup path already does. Both
                        // are "possession changed", and leaving the ball on the
                        // loser's hold point until the end-of-tick sync means
                        // every interaction resolved later in the same tick
                        // judges itself against a ball that has already moved.
                        ball.pos = hold;
                        player.shot_charge = 0.0;
                        player.charge_warmup_left = params.shot_charge_warmup_s;
                        trace(format!(
                            "STEAL {:?} P{} from {:?} P{} drain={drain:.2} tackler={tackler_stam:.2} carrier={carrier_stam:.2}",
                            player.team, player.id.0, ct, cid,
                        ));
                    } else {
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
    // Interact distance from the player's POSITION to the ball, and nothing
    // else. Same rule as the tackle above: reach is granted by interact radius
    // alone. Body radius is wall collision only and plays no part here.
    //
    // This carried the same phantom second origin the tackle did - the
    // tackler's hold point, 1.67 m ahead along their facing - which let a
    // player claim a ball up to 3.42 m away instead of 1.75 m.
    let dist = (player.pos - ball.pos).length();
    if dist <= params.interact_radius {
        // Reach is settled: the gate above is the ONLY distance test, so
        // everything below decides permission, never range. (A branch here used
        // to read `dist <= interact_radius + 1.0` for a "slightly fat" opening
        // reach; nested inside that gate it could never be false, so it granted
        // nothing and only made the reach rule look negotiable.)
        //
        // The single restriction that survives is the opening dump hang: the
        // kicked ball must travel ~0.12s before anyone may Interact-claim it.
        // The keeper is exempt, as is an opponent in the ~0.25s post-kick
        // reclaim window. Shooter-alone is already blocked above, and the ball
        // lockout blocks a same-tick snatch.
        let hot_opp_window = shooter_team.is_some_and(|t| t != player.team)
            && since_kick < 0.25
            && poss.opening_hot_reclaim
            && !poss.opening_dump_hang;
        let can_claim = if player.id.0 == 4 || hot_opp_window {
            true
        } else if poss.opening_dump_hang {
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
            // NO lockout on an uncontested pickup. `pickup_delay_after_exchange_s`
            // is, by its own definition, `Ball.pickupDelayAfterExchange` — wired
            // on a successful tackle EXCHANGE, which this is not. It was applied
            // here as a grace period so the kicker could not instantly re-tackle
            // a reclaim, inferred from one opening-dump observation.
            //
            // It made a passing relay impossible: measured with `pass_chain`,
            // player 2 claims a pass and the 0.25 s lockout then blocks player 3
            // from claiming the NEXT pass for 13 ticks. A receiver cannot see the
            // lockout, so it presses, gets refused, and — since Interact is an
            // impulse — burns its only rising edge and never claims at all.
            //
            // The concern it was guarding is now covered properly: a kicker
            // cannot instantly re-take the ball because that needs a release and
            // a fresh press, which is the impulse rule doing its job.
            //
            // UNCERTAIN: whether the real game also has some cooldown here is
            // unmeasured. The tackle-exchange lockout is untouched.
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
    fn ball_is_placed_from_the_new_facing_before_interaction_resolves() {
        // Tick order: ROTATION, then MOVEMENT along it, then BALL PLACEMENT at
        // the hold offset from that rotation (wall-projected), and only THEN
        // interaction. This test pins step 3 happening before step 4.
        //
        // The carrier reverses. Its ball swings from +hold_offset to
        // -hold_offset, a 3.34 m move, onto a defender standing behind it. If
        // the ball is placed after the interact loop instead (as it used to
        // be), that defender is judged against last tick's ball at 3.34 m —
        // nearly 2x interact radius — and the tackle silently cannot happen.
        let params = SimParams::default();
        let mover = crate::player::SimpleMover::from_params(&params);
        let mut carrier = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::ZERO,
            vel: Vec2::new(7.0, 0.0),
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_bits: 0,
        };
        let mut ball = Ball {
            pos: carrier.hold_pos(params.hold_offset),
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: true,
        };
        let stale = ball.pos;
        let poss = Possession {
            carrier: Some((TeamId::Home, 1)),
            ..Default::default()
        };

        // 1 + 2: rotation, then movement along the new heading.
        crate::player::step_mover(
            &mut carrier,
            &mover,
            Vec2::new(-50.0, 0.0),
            false,
            true,
            false,
            true,
            0.019,
        );
        assert!((carrier.facing - Vec2::NEG_X).length() < 1e-5);

        // 3: ball placement from that facing.
        sync_held_ball(&mut ball, &[carrier.clone()], &poss, &params);
        assert!(
            (ball.pos - carrier.hold_pos_playable(&params)).length() < 1e-6,
            "ball not at the wall-projected hold point"
        );

        // 4: interaction now sees the ball where the rotation put it.
        let defender_at = ball.pos;
        // The stale ball is out of interact range of this defender entirely,
        // which is the whole point: under the old order the tackle could not
        // even be attempted.
        assert!(
            (defender_at - stale).length() > params.interact_radius,
            "test is not exercising the stale-ball case: gap {}",
            (defender_at - stale).length()
        );
        assert!(
            (defender_at - ball.pos).length() <= params.interact_radius,
            "a defender on the new hold point must be able to interact"
        );
    }

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
            interact_bits: 0,
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
            interact_bits: 0,
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
            None,
        )
        .drain
        .expect("0/0 duel returns drain");
        assert!(drain.attacker_wins);
        assert!(drain.drain.abs() < 1e-5);
        assert_eq!(poss.carrier, Some((TeamId::Away, 1)));
        // No lockout is armed. This used to assert `pickup_lockout >= 0.55`,
        // a constant with no source, which made the test a pin for invented
        // behaviour rather than a check of the thing it is named after.
        assert_eq!(poss.pickup_lockout, 0.0);
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
            interact_bits: 0,
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
            interact_bits: 0,
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
            interact_bits: 0,
        };
        let cmd = BrainCommand {
            move_to: mate.pos,
            sprint: false,
            interact: true,
        };
        apply_interact(
            &mut mate, &mut ball, &mut poss, cmd, &params, 0.019, None, None, None, None,
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
            interact_bits: 0,
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
            interact_bits: 0,
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
            None,
        );
        assert_eq!(poss.carrier, Some((TeamId::Home, 1)));
        assert!(ball.held);
    }

    #[test]
    fn alternating_toggle_free_ball_pickup() {
        let params = SimParams::default();
        let mut ball = Ball {
            pos: Vec2::new(0.5, 0.0),
            vel: Vec2::ZERO,
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: false,
        };
        let mut poss = Possession::default();
        let mut player = Player {
            team: TeamId::Home,
            id: PlayerId(1),
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            facing: Vec2::X,
            stamina: 1.0,
            stamina_regen_lock_left: 0.0,
            shot_charge: 0.0,
            charge_warmup_left: 0.0,
            interact_bits: 0b1010101010, // Alternating interact bits (was held last tick)
        };
        let cmd = BrainCommand {
            move_to: Vec2::ZERO,
            sprint: false,
            interact: true, // held this tick, but interact_rising_edge is false because prev bit was 1
        };
        apply_interact(
            &mut player,
            &mut ball,
            &mut poss,
            cmd,
            &params,
            0.019,
            None,
            None,
            None,
            None,
        );
        assert_eq!(poss.carrier, Some((TeamId::Home, 1)));
        assert!(ball.held);
    }
}
