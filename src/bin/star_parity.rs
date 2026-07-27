//! Tick-exact parity between the STAR CHEESE design and its node-graph port.
//!
//! Runs one match and, every tick, asks BOTH for commands against the SAME
//! world state:
//!
//!   * `star_cheese_ref::decide` — the reference implementation
//!   * `StarCheese.txt` — the graph built by build_star_cheese.py
//!
//! The world is stepped with ONE of them (the graph, by default) so the two
//! see identical inputs right up to the moment they disagree. On the first
//! disagreement it dumps the full state — ball, carrier, every player, and both
//! sets of commands — and stops.
//!
//! This exists because a scoreline does not say WHICH tick went wrong. Blind
//! iteration on 0-23 results was changing three things at once and learning
//! nothing; this names the tick and the inputs.
//!
//!     cargo run --release --bin star_parity -- --graph <StarCheese.txt> \
//!         --away graph:<opponent.txt> --secs 180
//!
//! `--timeplot <file.csv>` records, every tick, what the graph decided next to
//! what the reference wanted and what the world actually did — so a divergence
//! can be read as a sequence rather than a single frame.

use std::env;
use std::fmt::Write as _;

use bevy::prelude::Vec2;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{BrainOutput, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::star_cheese_ref::decide;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn brain(input: &BrainInput, cache: &ProgramCache) -> Box<dyn TeamBrain> {
    match input {
        BrainInput::Graph(p) => Box::new(RuntimeBrain::from_cached(
            cache.get_or_compile(p).expect("compile"),
        )),
        _ => Box::new(aicomp_soccer_sim::brain::ChaseBallBrain::default()),
    }
}

/// Commands differ enough to matter. Positions are compared with a tolerance
/// because the reference and the graph compute the same target through
/// different float orderings; interact is exact, since it is a decision.
fn differs(a: &BrainOutput, b: &BrainOutput, tol: f32) -> Option<(usize, String)> {
    for s in 0..4 {
        let (x, y) = (a.commands[s], b.commands[s]);
        if x.interact != y.interact {
            return Some((s, format!("interact {} vs {}", x.interact, y.interact)));
        }
        let d = (x.move_to - y.move_to).length();
        if d > tol {
            return Some((
                s,
                format!(
                    "move_to ({:.2},{:.2}) vs ({:.2},{:.2})  |delta| {d:.2}",
                    x.move_to.x, x.move_to.y, y.move_to.x, y.move_to.y
                ),
            ));
        }
    }
    None
}

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let get = |k: &str| -> Option<String> {
        argv.iter()
            .position(|a| a == k)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let graph_path = get("--graph").expect("--graph <StarCheese.txt>");
    let away_in =
        BrainInput::parse(&get("--away").unwrap_or_else(|| "chase".into())).expect("away");
    let secs: f32 = get("--secs").unwrap_or_else(|| "180".into()).parse().unwrap();
    let tol: f32 = get("--tol").unwrap_or_else(|| "0.05".into()).parse().unwrap();
    let plot = get("--timeplot");
    // Step the world with the REFERENCE instead, to see where the graph would
    // have gone had the design been driving.
    let drive_ref = argv.iter().any(|a| a == "--drive-ref");

    let cache = ProgramCache::default();
    let mut home = brain(
        &BrainInput::parse(&format!("graph:{graph_path}")).expect("home graph"),
        &cache,
    );
    let mut away = brain(&away_in, &cache);
    let params = SimParams::default();
    let mut w = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);

    let mut pressed = [false; 4];
    let mut first: Option<u32> = None;
    let mut disagreements = 0u32;
    // Why one side wins and the other collapses, measured rather than argued:
    // relays are the mechanism, and stamina is what the mechanism costs.
    let mut relays = 0u32;      // Home -> Home possession changes (teammate tackles)
    let mut steals_lost = 0u32; // Home -> Away
    let mut steals_won = 0u32;  // Away -> Home: did we EVER tackle an opponent?
    let mut opp_held_ticks = 0u32;
    let mut in_reach_ticks = 0u32; // ticks a Home player was within reach of an enemy-held ball
    let mut stam_sum = 0.0f64;
    let mut stam_n = 0u32;
    let mut home_ticks = 0u32;
    let mut spawn_logged = false;
    let mut csv = String::new();
    if plot.is_some() {
        csv.push_str("tick,t,ball_x,ball_z,ball_vx,ball_vz,carrier,");
        for s in 1..=4 {
            let _ = write!(csv, "g{s}_mx,g{s}_mz,g{s}_int,r{s}_mx,r{s}_mz,r{s}_int,p{s}_x,p{s}_z,");
        }
        csv.push_str("agree\n");
    }

    let ticks = (secs / FIXED_DT) as u32;
    for tick in 0..ticks {
        let (home_api, away_api) = w.build_apis();
        let g_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        // The reference sees the SAME world, and its own press history.
        let mut probe = pressed;
        let r_out = decide(&w, &params, &mut probe, false, false);

        let diff = differs(&g_out, &r_out, tol);
        if diff.is_some() {
            disagreements += 1;
        }

        if let Some((slot, why)) = &diff {
            if first.is_none() {
                first = Some(tick);
                println!("FIRST DIVERGENCE at tick {tick}  (t = {:.3}s)", tick as f32 * FIXED_DT);
                println!("  player {}: {why}\n", slot + 1);
                println!("  INPUTS");
                println!(
                    "    ball      ({:.2},{:.2})  vel ({:.2},{:.2})  held {}",
                    w.ball.pos.x, w.ball.pos.y, w.ball.vel.x, w.ball.vel.y, w.ball.held
                );
                println!("    carrier   {:?}", w.possession.carrier);
                println!("    phase     {:?}", w.match_state.phase);
                for p in w.players.iter() {
                    println!(
                        "    {:?} {}  pos ({:7.2},{:7.2})  facing ({:5.2},{:5.2})  stam {:.2}  held {}",
                        p.team, p.id.0, p.pos.x, p.pos.y, p.facing.x, p.facing.y,
                        p.stamina, p.interact_bits & 1
                    );
                }
                println!("\n  COMMANDS      graph                    reference");
                for s in 0..4 {
                    let (g, r) = (g_out.commands[s], r_out.commands[s]);
                    let mark = if g.interact != r.interact
                        || (g.move_to - r.move_to).length() > tol
                    {
                        " <-- differs"
                    } else {
                        ""
                    };
                    println!(
                        "    P{}  move ({:7.2},{:7.2}) int {:5}   ({:7.2},{:7.2}) int {:5}{mark}",
                        s + 1, g.move_to.x, g.move_to.y, g.interact,
                        r.move_to.x, r.move_to.y, r.interact
                    );
                }
                println!();
            }
        }

        if let Some(_) = &plot {
            let carrier = match w.possession.carrier {
                Some((t, id)) => format!("{t:?}{id}"),
                None => "loose".into(),
            };
            let _ = write!(
                csv, "{tick},{:.3},{:.3},{:.3},{:.3},{:.3},{carrier},",
                tick as f32 * FIXED_DT, w.ball.pos.x, w.ball.pos.y, w.ball.vel.x, w.ball.vel.y
            );
            for s in 0..4 {
                let (g, r) = (g_out.commands[s], r_out.commands[s]);
                let p = w.players.iter().find(|p| p.team == TeamId::Home && p.id.0 as usize == s + 1);
                let (px, pz) = p.map(|p| (p.pos.x, p.pos.y)).unwrap_or((0.0, 0.0));
                let _ = write!(
                    csv, "{:.3},{:.3},{},{:.3},{:.3},{},{:.3},{:.3},",
                    g.move_to.x, g.move_to.y, g.interact as u8,
                    r.move_to.x, r.move_to.y, r.interact as u8, px, pz
                );
            }
            let _ = writeln!(csv, "{}", diff.is_none() as u8);
        }

        if !spawn_logged {
            // What the graph DECLARES vs where players actually stand. A
            // declared spot of (0,0) for every slot re-places the whole team on
            // the centre spot, where they tackle each other to zero stamina.
            println!("DECLARED kickoff_formation: {:?}", g_out.kickoff_positions);
            println!("SPAWN (formation the graph declared, whoever drives):");
            for p in w.players.iter().filter(|p| p.team == TeamId::Home) {
                println!("    Home {}  ({:7.2},{:7.2})", p.id.0, p.pos.x, p.pos.y);
            }
            println!();
            spawn_logged = true;
        }
        let before_c = w.possession.carrier;

        // Keep the press history aligned with whichever side is driving.
        let driving = if drive_ref { &r_out } else { &g_out };
        for s in 0..4 {
            pressed[s] = driving.commands[s].interact;
        }
        w.step_with_commands(driving, &away_out, FIXED_DT);

        match (before_c, w.possession.carrier) {
            (Some((TeamId::Home, a)), Some((TeamId::Home, b))) if a != b => relays += 1,
            (Some((TeamId::Home, _)), Some((TeamId::Away, _))) => steals_lost += 1,
            (Some((TeamId::Away, _)), Some((TeamId::Home, _))) => steals_won += 1,
            _ => {}
        }
        if matches!(w.possession.carrier, Some((TeamId::Home, _))) {
            home_ticks += 1;
        }
        if matches!(w.possession.carrier, Some((TeamId::Away, _))) {
            opp_held_ticks += 1;
            // Chances actually presented: was one of ours close enough to tackle?
            if w.players.iter().any(|p| {
                p.team == TeamId::Home && (p.pos - w.ball.pos).length() <= w.params.interact_radius
            }) {
                in_reach_ticks += 1;
            }
        }
        for p in w.players.iter().filter(|p| p.team == TeamId::Home) {
            stam_sum += p.stamina as f64;
            stam_n += 1;
        }
    }

    if let Some(path) = plot {
        std::fs::write(&path, csv).expect("write timeplot");
        println!("timeplot -> {path}");
    }

    println!(
        "
  driver            {}",
        if drive_ref { "REFERENCE (Rust design)" } else { "GRAPH (StarCheese.txt)" }
    );
    println!("  teammate relays   {relays}");
    println!("  balls lost        {steals_lost}");
    println!("  balls WON off opponents  {steals_won}");
    println!(
        "  chances to tackle        {in_reach_ticks} ticks in reach of an enemy-held ball (of {opp_held_ticks} they held it)"
    );
    println!("  possession        {:.1}%", 100.0 * home_ticks as f32 / ticks as f32);
    println!(
        "  mean Home stamina {:.3}   <- the cost of relaying: a tackle drains min(both) from BOTH",
        if stam_n == 0 { 0.0 } else { stam_sum / stam_n as f64 }
    );
    println!(
        "score {}-{}   disagreeing ticks {disagreements}/{ticks}  ({:.1}%)",
        w.match_state.score_home,
        w.match_state.score_away,
        100.0 * disagreements as f32 / ticks as f32
    );
    match first {
        Some(t) => println!("first divergence: tick {t}"),
        None => println!("EXACT PARITY — graph and reference agreed on every tick."),
    }
    let _ = Vec2::ZERO;
}
