//! Relay the ball through all four players as fast as the engine allows.
//!
//! Four Home players in a line, each exactly at maximum reach from the ball the
//! player behind them holds — `hold_offset + interact_radius` apart — all facing
//! along the chain.
//!
//! Every player runs ONE rule, decided from the state visible at the START of a
//! tick, which is all a real graph gets:
//!
//!     KICK relay:    interact = !carrying && ball_is_free && ball_in_reach
//!     TACKLE relay:  interact = !carrying && ball_in_reach
//!
//! The kick relay waits for the ball to come loose. Pressing claims it; the
//! next tick you are carrying, the rule goes false, and releasing IS the kick —
//! so the claim press doubles as the charge press and no tick is spent charging.
//!
//! The tackle relay does not wait: you can tackle a teammate, so the next player
//! takes the ball straight off the carrier. It costs stamina — the duel drains
//! min(tackler, carrier) from BOTH — which is what the summary reports.
//!
//!     cargo run --release --bin pass_chain
//!     cargo run --release --bin pass_chain -- --tackle

use bevy::prelude::Vec2;

use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamId};
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn main() {
    let params = SimParams::default();
    let tackle = std::env::args().any(|a| a == "--tackle");
    let mut w = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);
    w.match_state.phase = MatchPhase::Play;
    w.possession.first_kick_done = true;
    w.possession.kickoff_touch_done = true;

    // 1 cm inside maximum reach. At EXACTLY hold_offset + interact_radius the
    // distance lands on 1.7500001 in f32 and `dist <= interact_radius` refuses,
    // so a held ball is untouchable while a just-kicked one (which hops forward
    // 0.15 m on release) is not.
    let step = params.hold_offset + params.interact_radius - 0.01;
    for p in w.players.iter_mut() {
        p.vel = Vec2::ZERO;
        p.stamina = 1.0;
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
        p.interact_bits = 0;
        p.grab_hold_active = false;
        p.facing = Vec2::X;
        p.pos = if p.team == TeamId::Away {
            Vec2::new(0.0, 60.0)
        } else {
            Vec2::new((p.id.0 as f32 - 1.0) * step, 0.0)
        };
    }
    // Player 1 starts holding, and starts PRESSING — so its release next tick
    // is the first kick, exactly like every other player in the relay.
    w.possession.carrier = Some((TeamId::Home, 1));
    w.players[0].interact_bits = 1;
    w.ball.held = true;
    w.ball.pos = w.players[0].pos + Vec2::X * params.hold_offset;
    w.ball.vel = Vec2::ZERO;

    println!(
        "4 players {step:.2} m apart (hold {:.2} + reach {:.2}), facing +X",
        params.hold_offset, params.interact_radius
    );
    println!("rule: interact = !carrying && ball free && ball in reach\n");
    println!(" tick   interact     ball pos      spd    carrier");

    let mut owner_at: Vec<(u32, u8)> = vec![];
    let mut prev_owner = 1u8;

    for tick in 0..40u32 {
        let ball_free = !w.ball.held;
        let ball_pos = w.ball.pos;
        let carrier = w.possession.carrier;
        let mut home = BrainOutput::default();
        let mut pressing = String::new();
        for (slot, p) in w.players.iter().filter(|p| p.team == TeamId::Home).enumerate() {
            let carrying = matches!(carrier, Some((TeamId::Home, id)) if id == p.id.0);
            let in_reach = (p.pos - ball_pos).length() <= params.interact_radius;
            let interact = if tackle {
                // TACKLE relay. The carrier KEEPS PRESSING, so it charges
                // forever and never releases — the ball is never kicked, and
                // the only way it can move is a teammate taking it. Otherwise
                // the carrier's release would fire first (carrier resolves
                // first) and the "tackler" would just pick up a free ball,
                // which is the kick relay wearing a disguise.
                carrying || in_reach
            } else {
                !carrying && in_reach && ball_free
            };
            if interact {
                pressing.push_str(&format!("{} ", p.id.0));
            }
            home.commands[slot] = BrainCommand { move_to: p.pos, sprint: false, interact };
        }
        w.step_with_commands(&home, &BrainOutput::default(), FIXED_DT);

        let owner = match w.possession.carrier {
            Some((TeamId::Home, id)) => format!("Home {id}"),
            _ => "loose".into(),
        };
        println!(
            " {tick:4}   {:8}   ({:5.2},{:5.2})  {:6.2}   {owner}",
            if pressing.is_empty() { "-".into() } else { pressing },
            w.ball.pos.x,
            w.ball.pos.y,
            w.ball.vel.length(),
        );
        if let Some((TeamId::Home, id)) = w.possession.carrier {
            if id != prev_owner {
                owner_at.push((tick, id));
                prev_owner = id;
                if id == 4 {
                    break;
                }
            }
        }
    }

    println!("\n  hop    arrives on tick    ticks since previous");
    let mut prev_t = 0u32;
    for (t, id) in &owner_at {
        println!("  -> {id}          {t:4}                 {:4}", t - prev_t);
        prev_t = *t;
    }
    if let Some((t, _)) = owner_at.last() {
        println!(
            "\n  ball crossed all 4 players in {} ticks = {:.3} s ({:.2} m of pitch)",
            t + 1,
            (t + 1) as f32 * FIXED_DT,
            3.0 * step
        );
    }
    // The trade-off. A tackle duel drains min(tackler, carrier) from BOTH, so
    // moving the ball by tackling costs stamina that kicking does not.
    print!("  stamina left: ");
    for p in w.players.iter().filter(|p| p.team == TeamId::Home) {
        print!(" P{} {:.2}", p.id.0, p.stamina);
    }
    println!();
}
