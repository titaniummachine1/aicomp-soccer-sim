//! Count loose-ball "flypasts": ball enters a player's interact radius and
//! leaves again without that team claiming possession.
//!
//! cargo run --release --bin flypast_count -- \
//!   --a "graph:C:/.../TitaniumLean.txt" --b "graph:C:/.../Titanium_challenger.txt" \
//!   --opp graph:data/titanium/Haialand-v2.txt --secs 180

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn build(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    match input {
        BrainInput::Graph(p) => {
            let g = cache
                .get_or_compile(p)
                .map_err(|e| format!("load {p:?}: {e}"))?;
            Ok(Box::new(RuntimeBrain::from_cached(g)))
        }
        other => Err(format!("flypast_count needs graph:<path>, got {other:?}")),
    }
}

#[derive(Default, Clone, Debug)]
struct FlyStats {
    in_range_ticks: u64,
    in_range_no_press_ticks: u64,
    in_range_press_no_claim_ticks: u64,
    flypast_events: u64,
    claims_while_in_range: u64,
}

struct VisitTracker {
    in_bubble: [bool; 4],
    claimed: [bool; 4],
}

impl VisitTracker {
    fn new() -> Self {
        Self {
            in_bubble: [false; 4],
            claimed: [false; 4],
        }
    }
}

fn pct(n: u64, d: u64) -> f64 {
    if d == 0 {
        0.0
    } else {
        100.0 * n as f64 / d as f64
    }
}

fn run_side(
    label: &str,
    under_test: &BrainInput,
    opp: &BrainInput,
    under_is_home: bool,
    secs: f32,
    cache: &ProgramCache,
    params: &SimParams,
) -> Result<FlyStats, String> {
    let mut brain_a = build(under_test, cache)?;
    let mut brain_b = build(opp, cache)?;
    let opening = if under_is_home {
        TeamId::Home
    } else {
        TeamId::Away
    };
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), opening);

    let team = if under_is_home {
        TeamId::Home
    } else {
        TeamId::Away
    };
    let r = params.interact_radius;
    let mut stats = FlyStats::default();
    let mut visit = VisitTracker::new();
    let ticks = (secs / FIXED_DT).round() as u32;

    for _ in 0..ticks {
        let held_before = world.ball.held;
        let we_held_before = held_before
            && world
                .possession
                .carrier
                .map(|(t, _)| t == team)
                .unwrap_or(false);

        let (home_api, away_api) = world.build_apis();
        let (home_out, away_out) = if under_is_home {
            (brain_a.think(&home_api), brain_b.think(&away_api))
        } else {
            (brain_b.think(&home_api), brain_a.think(&away_api))
        };
        let our_cmds = if under_is_home {
            &home_out.commands
        } else {
            &away_out.commands
        };

        world.step_with_commands(&home_out, &away_out, FIXED_DT);

        let loose = !world.ball.held;
        let we_hold_after = world.ball.held
            && world
                .possession
                .carrier
                .map(|(t, _)| t == team)
                .unwrap_or(false);
        let claimed_now = we_hold_after && !we_held_before;

        // Claim happens the same tick the ball stops being loose — mark open
        // visits claimed before we process bubble exits.
        if claimed_now {
            for slot in 0..4 {
                if visit.in_bubble[slot] {
                    visit.claimed[slot] = true;
                    stats.claims_while_in_range += 1;
                }
            }
        }

        let mut any_in = false;
        let mut any_press_in = false;

        for p in world.players.iter() {
            if p.team != team {
                continue;
            }
            let slot = (p.id.0 as usize).saturating_sub(1).min(3);
            let in_range = loose && (p.pos - world.ball.pos).length() <= r;
            let pressed = our_cmds[slot].interact;

            if in_range {
                any_in = true;
                if pressed {
                    any_press_in = true;
                }
            }

            if in_range && !visit.in_bubble[slot] {
                visit.in_bubble[slot] = true;
                visit.claimed[slot] = false;
            }
            if visit.in_bubble[slot] && !in_range {
                if !visit.claimed[slot] {
                    stats.flypast_events += 1;
                }
                visit.in_bubble[slot] = false;
                visit.claimed[slot] = false;
            }
        }

        if any_in {
            stats.in_range_ticks += 1;
            if !any_press_in {
                stats.in_range_no_press_ticks += 1;
            } else if !we_hold_after {
                stats.in_range_press_no_claim_ticks += 1;
            }
        }
    }

    println!(
        "\n== {label} (under_test={}, opening={opening:?}) ==",
        if under_is_home { "home" } else { "away" }
    );
    println!(
        "  score {}-{}",
        world.match_state.score_home, world.match_state.score_away
    );
    println!("  in_range_ticks              {}", stats.in_range_ticks);
    println!(
        "  in_range_no_press_ticks     {}  ({:.1}% of in-range)",
        stats.in_range_no_press_ticks,
        pct(stats.in_range_no_press_ticks, stats.in_range_ticks)
    );
    println!(
        "  in_range_press_no_claim     {}  ({:.1}% of in-range)",
        stats.in_range_press_no_claim_ticks,
        pct(stats.in_range_press_no_claim_ticks, stats.in_range_ticks)
    );
    println!(
        "  flypast_events (enter→leave without claim)  {}",
        stats.flypast_events
    );
    println!("  claims_while_in_range       {}", stats.claims_while_in_range);
    Ok(stats)
}

fn add_stats(dst: &mut FlyStats, src: &FlyStats) {
    dst.in_range_ticks += src.in_range_ticks;
    dst.in_range_no_press_ticks += src.in_range_no_press_ticks;
    dst.in_range_press_no_claim_ticks += src.in_range_press_no_claim_ticks;
    dst.flypast_events += src.flypast_events;
    dst.claims_while_in_range += src.claims_while_in_range;
}

fn main() -> ExitCode {
    let mut a = BrainInput::Graph(PathBuf::from(
        r"C:\Users\Terminatort8000\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\TitaniumLean.txt",
    ));
    let mut b = BrainInput::Graph(PathBuf::from(
        r"C:\gitProjects\worldcup\titanim-socker-engine\out\Titanium_challenger.txt",
    ));
    let mut opp = BrainInput::Graph(PathBuf::from("data/titanium/Haialand-v2.txt"));
    let mut secs = 180.0f32;

    let mut argv = env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--a" => a = BrainInput::parse(&argv.next().expect("--a")).unwrap(),
            "--b" => b = BrainInput::parse(&argv.next().expect("--b")).unwrap(),
            "--opp" => opp = BrainInput::parse(&argv.next().expect("--opp")).unwrap(),
            "--secs" => secs = argv.next().unwrap().parse().unwrap(),
            "-h" | "--help" => {
                eprintln!(
                    "flypast_count — Lean vs Challenger missed-catch metrics\n\
                     --a graph:<path>   first titanium (default TitaniumLean)\n\
                     --b graph:<path>   second titanium (default Titanium_challenger)\n\
                     --opp graph:<path> common opponent (default Haialand-v2)\n\
                     --secs F           (default 180)"
                );
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }

    let params = match SimParams::load_from_disk(&default_params_path()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("params: {e}");
            return ExitCode::FAILURE;
        }
    };
    let cache = ProgramCache::new();

    println!("flypast_count  secs={secs}");
    println!("  A = {a:?}");
    println!("  B = {b:?}");
    println!("  opp = {opp:?}");

    let mut total_a = FlyStats::default();
    let mut total_b = FlyStats::default();

    for home in [true, false] {
        match run_side("A", &a, &opp, home, secs, &cache, &params) {
            Ok(s) => add_stats(&mut total_a, &s),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
        match run_side("B", &b, &opp, home, secs, &cache, &params) {
            Ok(s) => add_stats(&mut total_b, &s),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    println!("\n======== TOTALS (both openings vs same opp) ========");
    println!(
        "A (Lean)       flypast_events={}\n  in_range={}  no_press={} ({:.1}%)  press_no_claim={} ({:.1}%)  claims={}",
        total_a.flypast_events,
        total_a.in_range_ticks,
        total_a.in_range_no_press_ticks,
        pct(total_a.in_range_no_press_ticks, total_a.in_range_ticks),
        total_a.in_range_press_no_claim_ticks,
        pct(total_a.in_range_press_no_claim_ticks, total_a.in_range_ticks),
        total_a.claims_while_in_range,
    );
    println!(
        "B (Challenger) flypast_events={}\n  in_range={}  no_press={} ({:.1}%)  press_no_claim={} ({:.1}%)  claims={}",
        total_b.flypast_events,
        total_b.in_range_ticks,
        total_b.in_range_no_press_ticks,
        pct(total_b.in_range_no_press_ticks, total_b.in_range_ticks),
        total_b.in_range_press_no_claim_ticks,
        pct(total_b.in_range_press_no_claim_ticks, total_b.in_range_ticks),
        total_b.claims_while_in_range,
    );

    ExitCode::SUCCESS
}
