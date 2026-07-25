//! Tick-by-tick parity between two graphs that are supposed to be equivalent.
//!
//! Comparing match SCORES is a weak test: two graphs can diverge for many
//! ticks and still land on the same scoreline, or agree perfectly and differ
//! by a goal through chaos. This runs BOTH graphs against the SAME world
//! state every tick and diffs the controller outputs directly, so any
//! behavioural difference shows up on the exact tick it first occurs.
//!
//! Built for verifying build-time optimisations (`--strip-debug`, node
//! deduplication, dead-variable pruning): an optimiser that changes play is a
//! bug, and this proves it either way instead of inferring from scorelines.
//!
//! ```text
//! cargo run --release --bin parity_check -- \
//!     --a graph:out/ti_debug.txt --b graph:out/ti_release.txt \
//!     --away graph:Poponeta.txt --secs 180
//! ```

use std::env;
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{default_team_brain, soccer_aia_graph_path, BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{BrainOutput, IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn build(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    let path = match input {
        BrainInput::Chase => return Ok(Box::new(aicomp_soccer_sim::brain::ChaseBallBrain)),
        BrainInput::Idle => return Ok(Box::new(IdleBrain)),
        BrainInput::Aia => soccer_aia_graph_path(),
        BrainInput::Graph(p) => p.clone(),
        other => return Err(format!("parity_check needs graph:<path>, got {other:?}")),
    };
    let g = cache
        .get_or_compile(&path)
        .map_err(|e| format!("load {path:?}: {e}"))?;
    Ok(Box::new(RuntimeBrain::from_cached(g)))
}

/// Largest per-field difference between two outputs, in metres / bool flips.
fn diff(a: &BrainOutput, b: &BrainOutput) -> Option<(usize, String)> {
    for i in 0..4 {
        let (x, y) = (&a.commands[i], &b.commands[i]);
        let d = x.move_to.distance(y.move_to);
        if d > 1e-4 {
            return Some((
                i,
                format!(
                    "move_to ({:.4},{:.4}) vs ({:.4},{:.4})  delta {:.4} m",
                    x.move_to.x, x.move_to.y, y.move_to.x, y.move_to.y, d
                ),
            ));
        }
        if x.sprint != y.sprint {
            return Some((i, format!("sprint {} vs {}", x.sprint, y.sprint)));
        }
        if x.interact != y.interact {
            return Some((i, format!("interact {} vs {}", x.interact, y.interact)));
        }
    }
    None
}

fn main() -> ExitCode {
    // Deeply-nested graphs (ZudanFC3) blow the default stack during lowering,
    // same reason soccer_headless runs on a big-stack thread.
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .expect("spawn parity thread")
        .join()
        .expect("parity thread panicked")
}

fn run() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let (mut a_in, mut b_in) = (default_team_brain(), default_team_brain());
    let mut away = BrainInput::Idle;
    let mut secs = 180.0f32;
    let mut seed_lo = 0u64;
    let mut seeds = 1u64;
    let mut i = 0;
    while i < argv.len() {
        let mut take = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i).cloned().unwrap_or_default()
        };
        match argv[i].as_str() {
            "--a" => a_in = BrainInput::parse(&take(&mut i)).expect("--a"),
            "--b" => b_in = BrainInput::parse(&take(&mut i)).expect("--b"),
            "--away" => away = BrainInput::parse(&take(&mut i)).expect("--away"),
            "--secs" => secs = take(&mut i).parse().expect("--secs"),
            "--seed" => seed_lo = take(&mut i).parse().expect("--seed"),
            "--seeds" => seeds = take(&mut i).parse().expect("--seeds"),
            "-h" | "--help" => {
                eprintln!("parity_check --a graph:<x> --b graph:<y> [--away <brain>] [--secs N] [--seed N] [--seeds N]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();
    let cache = ProgramCache::default();
    let mut worst_ok = 0.0f32;
    let mut any_diff = false;

    for seed in seed_lo..seed_lo + seeds {
        let mut a = match build(&a_in, &cache) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error --a: {e}");
                return ExitCode::from(1);
            }
        };
        let mut b = match build(&b_in, &cache) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error --b: {e}");
                return ExitCode::from(1);
            }
        };
        let mut opp = match build(&away, &cache) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error --away: {e}");
                return ExitCode::from(1);
            }
        };

        let opening = if seed % 2 == 0 { TeamId::Home } else { TeamId::Away };
        let mut world = MatchWorld::new_kickoff_opening(params.clone(), opening);
        let ticks = (secs / FIXED_DT) as u32;
        let mut diverged_at: Option<(u32, usize, String)> = None;

        for t in 0..ticks {
            let (home_api, away_api) = world.build_apis();
            // Both candidates see the SAME world state this tick.
            aicomp_soccer_sim::debug_draw::begin_frame();
            let out_a = a.think(&home_api);
            aicomp_soccer_sim::debug_draw::begin_frame();
            let out_b = b.think(&home_api);
            aicomp_soccer_sim::debug_draw::begin_frame();
            let out_away = opp.think(&away_api);

            if diverged_at.is_none() {
                if let Some((slot, what)) = diff(&out_a, &out_b) {
                    diverged_at = Some((t, slot, what));
                }
            }
            // A drives the world, so the trajectory stays well-defined even
            // after a divergence — we still want to know how often it recurs.
            world.step_with_commands(&out_a, &out_away, FIXED_DT);
        }

        match diverged_at {
            None => {
                println!("seed {seed}: IDENTICAL across {ticks} ticks");
                worst_ok += 1.0;
            }
            Some((t, slot, what)) => {
                any_diff = true;
                println!(
                    "seed {seed}: DIVERGED at tick {t} ({:.2}s), player {}: {what}",
                    t as f32 * FIXED_DT,
                    slot + 1
                );
            }
        }
    }

    let bad = aicomp_soccer_sim::graph_vm::diagnostics::unsound();
    if !bad.is_empty() {
        // Equivalence proven over Null inputs is not equivalence.
        println!("
{}", aicomp_soccer_sim::graph_vm::diagnostics::banner().trim_end());
        println!("REFUSING to certify parity: results are unsound.");
        return ExitCode::from(3);
    }
    if any_diff {
        println!("\nRESULT: NOT equivalent — the two graphs play differently.");
        ExitCode::from(1)
    } else {
        println!("\nRESULT: equivalent on {} seed(s).", worst_ok as u32);
        ExitCode::SUCCESS
    }
}
