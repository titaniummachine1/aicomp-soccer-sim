//! Fastest possible pass between two teammates, traced tick by tick.
//!
//! Setup:
//!   * Home 1 on the centre spot, holding the ball, AIMED AT THE TEAMMATE.
//!   * Home 2 as far from the ball as it can be while still able to reach it —
//!     exactly `interact_radius` from the BALL, on the far side, so the kick
//!     travels straight at them.
//!   * Home 1 holds Interact for `--press` ticks, then releases. The release is
//!     the kick, along that tick's move input.
//!   * Home 2 presses Interact from the moment the ball is released.
//!
//!     cargo run --release --bin pass_speed -- --press 20
//!     cargo run --release --bin pass_speed -- --press 1
//!
//! `--fresh-pickup` models passing immediately after receiving, which carries
//! the pickup charge warmup.

use bevy::prelude::Vec2;

use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamId};
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn main() {
    let params = SimParams::default();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let press: u32 = argv
        .iter()
        .position(|a| a == "--press")
        .and_then(|i| argv.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let fresh = argv.iter().any(|a| a == "--fresh-pickup");
    // Swap roles so the RECEIVER has the lower player index. Interaction is
    // resolved in index order, so this decides whether the receiver can claim
    // on the very tick the ball is released or has to wait for the next one.
    let swap = argv.iter().any(|a| a == "--swap");
    trace(&params, press, fresh, swap);
}

fn trace(params: &SimParams, press: u32, fresh: bool, swap: bool) {
    let (pa, rx) = if swap { (1usize, 0usize) } else { (0usize, 1usize) };
    let mut w = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);
    w.match_state.phase = MatchPhase::Play;
    w.possession.first_kick_done = true;
    w.possession.kickoff_touch_done = true;

    for (i, p) in w.players.iter_mut().enumerate() {
        p.pos = Vec2::new(0.0, 60.0 + i as f32 * 5.0);
        p.vel = Vec2::ZERO;
        p.stamina = 1.0;
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
        p.interact_bits = 0;
        p.grab_hold_active = false;
    }

    // Passer on the centre spot, facing +X — straight at the receiver.
    w.players[pa].pos = Vec2::ZERO;
    w.players[pa].facing = Vec2::X;
    if fresh {
        w.players[pa].charge_warmup_left = params.shot_charge_warmup_s;
    }
    w.possession.carrier = Some((TeamId::Home, pa as u8 + 1));
    w.ball.held = true;
    w.ball.pos = w.players[pa].pos + w.players[pa].facing * params.hold_offset;
    w.ball.vel = Vec2::ZERO;

    // Receiver: maximum reach FROM THE BALL, directly in the kick's path.
    let receiver = w.ball.pos + Vec2::X * params.interact_radius;
    w.players[rx].pos = receiver;
    w.players[rx].facing = -Vec2::X;

    println!("passer   Home {} at (0.00, 0.00)  aiming +X at the receiver", pa + 1);
    println!("ball     held   ({:.2}, {:.2})", w.ball.pos.x, w.ball.pos.y);
    println!(
        "receiver Home {} ({:.2}, {:.2})  — {:.2} m from the ball = interact radius {:.2}",
        rx + 1,
        receiver.x,
        receiver.y,
        (receiver - w.ball.pos).length(),
        params.interact_radius
    );
    println!(
        "press {press} tick(s); one tick of charge = {:.4}, kick needs > 0.05{}\n",
        FIXED_DT / params.shot_charge_time_s,
        if fresh {
            format!(", warmup {:.2} s first", params.shot_charge_warmup_s)
        } else {
            String::new()
        }
    );

    println!(" tick  P1 int  P2 int   charge   ball pos       spd    dist P2   carrier");
    let _aim = Vec2::new(50.0, 0.0);
    let mut received_on: Option<u32> = None;
    let mut launch: Option<(u32, f32, f32)> = None;

    for tick in 0..(press + 14) {
        let p1_int = tick < press;
        // PERFECT CAPTURE: press Interact only when the ball is actually free
        // AND in reach. This is the state a graph observes at the START of the
        // tick, before anything resolves — so on the release tick the ball still
        // reads as held and no real receiver would press yet.
        let p2_int = !w.ball.held
            && (w.players[rx].pos - w.ball.pos).length() <= params.interact_radius;
        let charge_before = w.players[pa].shot_charge;

        // The passer STANDS STILL and aims with its facing (+X, at the
        // receiver). Walking toward an aim point would just carry the ball to
        // the teammate, which is not a pass.
        let mut home = BrainOutput::default();
        home.commands[pa] = BrainCommand {
            move_to: w.players[pa].pos,
            sprint: false,
            interact: p1_int,
        };
        home.commands[rx] = BrainCommand {
            move_to: w.players[rx].pos,
            sprint: false,
            interact: p2_int,
        };
        w.step_with_commands(&home, &BrainOutput::default(), FIXED_DT);

        // The release tick: the carrier no longer holds the ball. Catches the
        // same-tick capture case too, where the ball is never seen loose.
        if launch.is_none() && !matches!(w.possession.carrier, Some((TeamId::Home, id)) if id == pa as u8 + 1) {
            launch = Some((tick, w.ball.vel.length(), charge_before));
        }
        let carrier = match w.possession.carrier {
            Some((t, id)) => format!("{t:?} {id}"),
            None => "loose".into(),
        };
        println!(
            " {tick:4}  {:6}  {:6}   {:6.3}   ({:5.2},{:5.2})  {:6.2}   {:6.2}   {carrier}",
            p1_int,
            p2_int,
            w.players[pa].shot_charge,
            w.ball.pos.x,
            w.ball.pos.y,
            w.ball.vel.length(),
            (w.players[rx].pos - w.ball.pos).length(),
        );
        if received_on.is_none() && matches!(w.possession.carrier, Some((TeamId::Home, id)) if id == rx as u8 + 1) {
            received_on = Some(tick);
            break;
        }
    }

    println!();
    match launch {
        Some((t, v, c)) => println!("released on tick {t} at charge {c:.3}, launch {v:.2} m/s"),
        None => println!("NEVER RELEASED — charge never passed the > 0.05 gate"),
    }
    match (launch, received_on) {
        (Some((lt, _, _)), Some(rt)) => {
            let d = rt - lt;
            println!(
                "RECEIVED on tick {rt} — {d} tick(s) after release = {:.3} s",
                d as f32 * FIXED_DT
            );
        }
        _ => println!("NOT RECEIVED."),
    }
}
