//! Derive a team's kickoff formation instead of guessing it.
//!
//! Hand-picking faceoff spots is guesswork, and copying another brain's spots
//! imports its shape along with them. But the brain already knows where it
//! wants to stand: give the opponent the ball on the centre spot, let it sit
//! there, and the team under test will walk into whatever defensive shape it
//! considers optimal. Record where that settles and declare THAT as the
//! kickoff formation — the team then starts in the shape it would have
//! reached anyway, without spending the opening seconds walking there.
//!
//! The opponent is frozen holding the ball at the centre so nothing evolves:
//! no carry, no shot, no restart. The only thing moving is the team being
//! measured.
//!
//! ```text
//! cargo run --release --bin settle_formation -- \
//!     --graph data/titanium/Titanium.txt --secs 20
//! ```
//!
//! Output is in the frame `ConstructSoccerProperties` expects, ready to paste.

use std::env;
use std::process::ExitCode;

use bevy::prelude::Vec2;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{BrainOutput, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

/// Pin the opponent carrier on the centre spot, ball held, match in Play.
/// Re-applied every tick: the point is a world that does not evolve, so the
/// only motion is the team positioning itself.
fn freeze_opponent_on_the_spot(world: &mut MatchWorld, opp: TeamId) {
    let hold_offset = world.params.hold_offset;
    let rest_height = world.params.ball_rest_height;
    world.match_state.phase = MatchPhase::Play;
    world.match_state.kickoff_circle_lock = false;
    world.match_state.kickoff_suppress_away_team_side = false;
    world.match_state.clock_s = 0.0;

    let face = match opp {
        TeamId::Home => Vec2::X,
        TeamId::Away => -Vec2::X,
    };
    for p in world.players.iter_mut().filter(|p| p.team == opp) {
        if p.id == PlayerId(1) {
            p.pos = Vec2::ZERO;
            p.facing = face;
        }
        p.vel = Vec2::ZERO;
        p.stamina = 1.0;
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
    }
    world.ball.pos = face * hold_offset;
    world.ball.vel = Vec2::ZERO;
    world.ball.vel_y = 0.0;
    world.ball.height = rest_height;
    world.ball.held = true;
    world.possession.carrier = Some((opp, 1));
    world.possession.first_kick_done = true;
    world.possession.kickoff_touch_done = true;
    world.possession.pickup_lockout = 0.0;
    world.match_state.reset_stale_tracker(world.ball.pos);
}

fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .expect("spawn settle thread")
        .join()
        .expect("settle thread panicked")
}

fn run() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut graph = String::new();
    let mut secs = 20.0f32;
    let mut side = TeamId::Home;
    // Default is a REAL kickoff: opponent kicks off, ball free on the centre
    // spot, everyone on their declared marks. `--hold-ball` instead pins the
    // opponent holding it, which reads as "they have possession" and makes the
    // whole team collapse onto its own goal line — a true answer to a question
    // nobody asked at a kickoff.
    let mut hold_ball = false;
    let mut i = 0;
    while i < argv.len() {
        let mut take = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i).cloned().unwrap_or_default()
        };
        match argv[i].as_str() {
            "--graph" => graph = take(&mut i),
            "--secs" => secs = take(&mut i).parse().expect("--secs"),
            "--hold-ball" => hold_ball = true,
            "--side" => {
                side = match take(&mut i).as_str() {
                    "away" => TeamId::Away,
                    _ => TeamId::Home,
                }
            }
            "-h" | "--help" => {
                eprintln!(
                    "settle_formation --graph <path> [--secs N] [--side home|away] [--hold-ball]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    if graph.is_empty() {
        eprintln!("--graph is required");
        return ExitCode::from(2);
    }

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();
    let cache = ProgramCache::default();
    let path = match BrainInput::parse(&format!("graph:{graph}")) {
        Ok(BrainInput::Graph(p)) => p,
        _ => {
            eprintln!("bad --graph {graph}");
            return ExitCode::from(2);
        }
    };
    let compiled = match cache.get_or_compile(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("load {path:?}: {e}");
            return ExitCode::from(1);
        }
    };
    let mut brain = RuntimeBrain::from_cached(compiled);

    let opp = side.other();
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), opp);
    // Spawn on whatever the graph declares, exactly as a match would.
    let opening_out = {
        let (h, a) = world.build_apis();
        let api = if side == TeamId::Home { h } else { a };
        brain.think(&api)
    };
    let mut blank = BrainOutput::default();
    blank.kickoff_positions = opening_out.kickoff_positions;
    if side == TeamId::Home {
        world.observe_brain_outputs(&blank, &BrainOutput::default());
    } else {
        world.observe_brain_outputs(&BrainOutput::default(), &blank);
    }
    if hold_ball {
        freeze_opponent_on_the_spot(&mut world, opp);
    }

    let ticks = (secs / FIXED_DT) as u32;
    let idle = BrainOutput::default();
    let mut prev = [Vec2::ZERO; 4];
    // What the controllers ASK for on the opening tick, before anyone has
    // walked anywhere. This — not where they end up — is the formation the
    // brain wants: holding the ball still at the centre forever is not a real
    // situation, and letting it run collapses everyone onto the goal line.
    let mut wanted = [Vec2::ZERO; 4];
    println!("settling {} as {:?} for {:.0}s\n", path.display(), side, secs);
    println!("   t     P1                P2                P3                P4        max move/s");
    for t in 0..ticks {
        let (home_api, away_api) = world.build_apis();
        let api = if side == TeamId::Home {
            home_api
        } else {
            away_api
        };
        let out = brain.think(&api);
        if t == 0 {
            for k in 0..4 {
                wanted[k] = out.commands[k].move_to;
            }
        }
        let (h, a) = if side == TeamId::Home {
            (out, idle.clone())
        } else {
            (idle.clone(), out)
        };
        world.step_with_commands(&h, &a, FIXED_DT);
        if hold_ball {
            // The opponent must not react, drift, or be shoved off the spot.
            freeze_opponent_on_the_spot(&mut world, opp);
        }

        let every = (1.0 / FIXED_DT) as u32;
        if t % every == every - 1 {
            let mut now = [Vec2::ZERO; 4];
            for p in world.players.iter().filter(|p| p.team == side) {
                now[p.id.0 as usize - 1] = p.pos;
            }
            let moved = (0..4)
                .map(|k| now[k].distance(prev[k]))
                .fold(0.0f32, f32::max);
            let cells: Vec<String> = now
                .iter()
                .map(|v| format!("({:6.2},{:6.2})", v.x, v.y))
                .collect();
            println!(
                "{:5.0}s  {}   {:.2} m",
                (t + 1) as f32 * FIXED_DT,
                cells.join("  "),
                moved
            );
            prev = now;
        }
    }

    let mut settled = [Vec2::ZERO; 4];
    for p in world.players.iter().filter(|p| p.team == side) {
        settled[p.id.0 as usize - 1] = p.pos;
    }

    // Declared spots are world absolute and mirror with the team multiplier
    // (-1 for Home, +1 for Away), so undo that to get the value to declare.
    let tm = match side {
        TeamId::Home => -1.0,
        TeamId::Away => 1.0,
    };
    println!("\n-- paste into InitializeSoccer, via spot(x, z) * TeamMultiplier --");
    for k in 0..4 {
        let d = settled[k] * tm;
        let own_half_ok = match side {
            TeamId::Home => settled[k].x <= 0.0,
            TeamId::Away => settled[k].x >= 0.0,
        };
        let mut note = String::new();
        if !own_half_ok {
            note.push_str("  [OPPONENT HALF - would be clamped to x=0]");
        }
        if wanted[k].length() < params.kickoff_circle_r {
            note.push_str("  [inside circle - pushed out when receiving]");
        }
        println!("    spot({:6.2}, {:6.2}),   // P{}{}", d.x, d.y, k + 1, note);
    }
    ExitCode::SUCCESS
}
