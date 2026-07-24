//! Titanium Scenario 3 **training harness** (not a general match mode).
//!
//! Full-strength 4v1: the real AIA opponent team (all 4 players, unmodified
//! AI — the same graph the sim uses for `--away aia3` in soccer_headless)
//! attacks a normal kickoff formation; our own team has only the GK (P4)
//! active — P1-3 are parked off-pitch and frozen every tick, never touching
//! the ball. This is the hardest version of "can the GK alone keep the ball
//! out" — no teammates to share the defensive load at all.
//!
//! - **GK wins** = gains possession at any point (even one tick — same
//!   scoring convention as Scenario 1/2), or timeout.
//! - **Opponent wins** = scores.
//!
//! ```text
//! cargo run --release --bin titanium_drill3 -- --trials 10 --gk data/titanium/Titanium.txt
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::Vec2;

use aicomp_soccer_sim::batch::soccer_aia_graph_path;
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::load_team_graph;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::scenario::{evaluate_scenario1, Scenario1Outcome};
use aicomp_soccer_sim::titanium::{apply_4v1_freeze, repark_4v1_inactive, setup_4v1_harness};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

/// Build a team brain from a graph .txt. Defaults to the Titanium graph in
/// the AIComp saves dir — this repo ships no native team AI, the graph is
/// the team. Build it with the engine repo's `scripts/build_titanium.py`.
fn build_gk_brain(graph_path: &Option<PathBuf>) -> Result<Box<dyn TeamBrain>, String> {
    match graph_path {
        None => aicomp_soccer_sim::batch::build_default_titanium_brain(),
        Some(path) => {
            let g = load_team_graph(path).map_err(|e| format!("load graph {path:?}: {e}"))?;
            Ok(Box::new(RuntimeBrain::compile(g)))
        }
    }
}

fn build_opponent_brain(graph_path: &Option<PathBuf>) -> Result<Box<dyn TeamBrain>, String> {
    let path = graph_path.clone().unwrap_or_else(soccer_aia_graph_path);
    let g = load_team_graph(&path).map_err(|e| format!("load graph {path:?}: {e}"))?;
    Ok(Box::new(RuntimeBrain::compile(g)))
}

#[derive(Debug)]
struct Args {
    trials: u32,
    secs: f32,
    wall_secs: f32,
    quiet: bool,
    gk_graph: Option<PathBuf>,
    opponent_graph: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundResult {
    OpponentGoal,
    GkCatch,
    GkTimeout,
}

impl From<Scenario1Outcome> for RoundResult {
    fn from(o: Scenario1Outcome) -> Self {
        match o {
            Scenario1Outcome::AttackerGoal => Self::OpponentGoal,
            Scenario1Outcome::GkHold => Self::GkCatch,
            Scenario1Outcome::GkTimeout => Self::GkTimeout,
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut trials = 10u32;
    let mut secs = 180.0f32;
    let mut wall_secs = 60.0f32;
    let mut quiet = false;
    let mut gk_graph = None;
    let mut opponent_graph = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "titanium_drill3 — Scenario 3 (full-strength 4v1: real AIA opponent \
                     team vs our GK alone; teammates P1-3 parked off-pitch, frozen)\n\
                     GK wins: possession at any point, or timeout. Opponent wins: goal.\n\
                     --trials N (default 10)\n\
                     --secs F     soft match-clock cap (default 180)\n\
                     --wall-secs F  real-time budget / trial (default 60)\n\
                     --gk PATH    drive the GK with this graph .txt instead of \
                     the default cascade\n\
                     --opponent PATH  opponent graph .txt (default: AIA3.txt / AIA.txt \
                     from the AIComp saves dir, same as soccer_headless --away aia3)\n\
                     --quiet"
                );
                std::process::exit(0);
            }
            "--trials" => {
                i += 1;
                trials = argv
                    .get(i)
                    .ok_or("--trials needs a value")?
                    .parse()
                    .map_err(|e| format!("--trials: {e}"))?;
            }
            "--secs" => {
                i += 1;
                secs = argv
                    .get(i)
                    .ok_or("--secs needs a value")?
                    .parse()
                    .map_err(|e| format!("--secs: {e}"))?;
            }
            "--wall-secs" => {
                i += 1;
                wall_secs = argv
                    .get(i)
                    .ok_or("--wall-secs needs a value")?
                    .parse()
                    .map_err(|e| format!("--wall-secs: {e}"))?;
            }
            "--gk" => {
                i += 1;
                gk_graph = Some(PathBuf::from(argv.get(i).ok_or("--gk needs a path")?));
            }
            "--opponent" => {
                i += 1;
                opponent_graph = Some(PathBuf::from(
                    argv.get(i).ok_or("--opponent needs a path")?,
                ));
            }
            "--quiet" => quiet = true,
            other => return Err(format!("unknown arg {other}")),
        }
        i += 1;
    }
    Ok(Args {
        trials,
        secs,
        wall_secs,
        quiet,
        gk_graph,
        opponent_graph,
    })
}

fn run_trial(
    params: SimParams,
    opening: TeamId,
    gk_team: TeamId,
    secs: f32,
    wall_secs: f32,
    gk_graph: &Option<PathBuf>,
    opponent_graph: &Option<PathBuf>,
) -> Result<(RoundResult, f32, Vec2, Vec2), String> {
    let opp_team = gk_team.other();
    let mut world = MatchWorld::new_kickoff_opening(params, opening);

    // Shared setup (also used by the viewer's Scenario 3): parks the GK
    // team's P1-3 off-pitch, puts the GK on cover, gives the opponent team a
    // fresh kickoff formation, and resets stamina/charge for everyone.
    setup_4v1_harness(&mut world, gk_team == TeamId::Home);

    let mut gk_brain: Box<dyn TeamBrain> = build_gk_brain(gk_graph)?;
    let mut opp_brain: Box<dyn TeamBrain> = build_opponent_brain(opponent_graph)?;

    let max_ticks = ((secs / FIXED_DT).ceil() as u64).max(1);
    let wall_budget = Duration::from_secs_f32(wall_secs.max(0.5));
    let wall_start = Instant::now();
    let start_h = world.match_state.score_home;
    let start_a = world.match_state.score_away;
    // `evaluate_scenario1` labels outcomes by "attacker"/"gk" relative to
    // `attack_home` — here the "attacker" role is always the opponent team.
    let attack_home = opp_team == TeamId::Home;

    let gk_pos = |world: &MatchWorld| {
        world
            .players
            .iter()
            .find(|p| p.team == gk_team && p.id == PlayerId(4))
            .map(|p| p.pos)
            .unwrap_or(Vec2::ZERO)
    };

    for _tick in 0..max_ticks {
        if wall_start.elapsed() >= wall_budget {
            return Ok((
                RoundResult::GkTimeout,
                world.match_state.clock_s,
                world.ball.pos,
                gk_pos(&world),
            ));
        }

        let (home_api, away_api) = world.build_apis();
        let mut home_out = if gk_team == TeamId::Home {
            gk_brain.think(&home_api)
        } else {
            opp_brain.think(&home_api)
        };
        let mut away_out = if gk_team == TeamId::Away {
            gk_brain.think(&away_api)
        } else {
            opp_brain.think(&away_api)
        };

        apply_4v1_freeze(&mut home_out, &mut away_out, &world, gk_team == TeamId::Home);

        world.step_with_commands(&home_out, &away_out, FIXED_DT);

        // Re-park every tick too — goal-mouth clamps or stray physics could
        // otherwise pull a "frozen" player back onto the playable AABB edge.
        repark_4v1_inactive(&mut world, gk_team == TeamId::Home);

        if let Some(out) = evaluate_scenario1(&world, attack_home, start_h, start_a) {
            return Ok((
                RoundResult::from(out),
                world.match_state.clock_s,
                world.ball.pos,
                gk_pos(&world),
            ));
        }
    }

    Ok((
        RoundResult::GkTimeout,
        world.match_state.clock_s.max(secs),
        world.ball.pos,
        gk_pos(&world),
    ))
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();

    let mut opp_goals = 0u32;
    let mut gk_catches = 0u32;
    let mut gk_timeouts = 0u32;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);

    for i in 0..args.trials {
        // Alternate which side our GK defends and who kicks off, same
        // parity convention as soccer_headless (even=home opening).
        let gk_team = if i % 2 == 0 { TeamId::Home } else { TeamId::Away };
        let opening = if ((seed >> (i % 60)) & 1) == 0 {
            TeamId::Home
        } else {
            TeamId::Away
        };
        let (result, t, ball_pos, gk_p) = match run_trial(
            params.clone(),
            opening,
            gk_team,
            args.secs,
            args.wall_secs,
            &args.gk_graph,
            &args.opponent_graph,
        ) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        match result {
            RoundResult::OpponentGoal => opp_goals += 1,
            RoundResult::GkCatch => gk_catches += 1,
            RoundResult::GkTimeout => gk_timeouts += 1,
        }
        if !args.quiet {
            let tag = match result {
                RoundResult::OpponentGoal => "opponent_goal",
                RoundResult::GkCatch => "gk_catch",
                RoundResult::GkTimeout => "gk_timeout",
            };
            eprintln!(
                "trial {i}: gk_team={gk_team:?} opening={opening:?} {tag} t={t:.2} \
                 ball=({:.2},{:.2}) gk=({:.2},{:.2})",
                ball_pos.x, ball_pos.y, gk_p.x, gk_p.y,
            );
        }
    }

    let gk_wins = gk_catches + gk_timeouts;
    let json = serde_json::json!({
        "ok": true,
        "scenario": 3,
        "harness": "scenario3_full_4v1",
        "trials": args.trials,
        "secs": args.secs,
        "wall_secs": args.wall_secs,
        "opponent_goals": opp_goals,
        "gk_catches": gk_catches,
        "gk_timeouts": gk_timeouts,
        "gk_wins": gk_wins,
        "opponent_goal_rate": opp_goals as f32 / args.trials.max(1) as f32,
        "gk_win_rate": gk_wins as f32 / args.trials.max(1) as f32,
    });
    println!("{json}");

    let log_dir = PathBuf::from("data/titanium");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("eval_log3.jsonl");
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{json}");
    }

    ExitCode::SUCCESS
}
