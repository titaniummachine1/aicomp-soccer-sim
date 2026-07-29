//! Titanium Scenario 2 **training harness** (not a general match mode).
//!
//! 2v1 passing drill, scripted attacker side (deterministic, no AI decision
//! making — see `PassDrillAttackerBrain`):
//! - Attacker P1 at **center** with the ball, charges to full power, then
//!   passes toward attacker P2's wide receiving spot.
//! - Attacker P2 waits at the receiving spot, claims the pass, charges to
//!   full power, then shoots at the requested post extreme (left/right).
//! - GK (P4) already on cover, driven by the default graph cascade or an
//!   arbitrary graph file (`--gk PATH`) — the real Python deliverable.
//! - Everyone else parked off-pitch.
//! - **Attacker wins** = scores. **GK wins** = intercepts/holds the ball
//!   (at any point — during the pass OR the final shot), or timeout.
//!
//! This tests whether the GK can weigh "intercept the pass in flight" against
//! "cover the shot cone from wherever the receiver ends up" — the pass is
//! only a real option for the attacker if the GK can't already beat it to
//! the interception point.
//!
//! ```text
//! cargo run --release --bin titanium_drill2 -- --trials 10 --extreme left --gk data/titanium/Titanium.txt
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::Vec2;

use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::load_team_graph;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::scenario::{evaluate_scenario1, Scenario1Outcome};
use aicomp_soccer_sim::titanium::{
    apply_2v1_freeze, repark_2v1_inactive, setup_2v1_harness, PassDrillAttackerBrain,
    ShotExtreme,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtremeArg {
    Left,
    Right,
    Both,
}

#[derive(Debug)]
struct Args {
    trials: u32,
    secs: f32,
    wall_secs: f32,
    quiet: bool,
    gk_graph: Option<PathBuf>,
    extreme: ExtremeArg,
    receiver_wide: f32,
    receiver_forward: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundResult {
    AttackerGoal,
    GkCatch,
    GkTimeout,
}

impl From<Scenario1Outcome> for RoundResult {
    fn from(o: Scenario1Outcome) -> Self {
        match o {
            Scenario1Outcome::AttackerGoal => Self::AttackerGoal,
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
    let mut extreme = ExtremeArg::Both;
    let mut receiver_wide = 9.0f32;
    let mut receiver_forward = 8.0f32;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "titanium_drill2 — Scenario 2 (2v1 passing drill: P1 charges+passes to \
                     P2, P2 receives+shoots at extreme; only P1/P2/GK visible)\n\
                     Atk wins: goal. GK wins: catch/intercept OR timeout.\n\
                     --trials N (default 10)\n\
                     --secs F     soft match-clock cap (default 180)\n\
                     --wall-secs F  real-time budget / trial (default 60)\n\
                     --gk PATH    drive the GK with this graph .txt instead of \
                     the default cascade\n\
                     --extreme left|right|both (default both, alternates per trial)\n\
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
            "--extreme" => {
                i += 1;
                extreme = match argv.get(i).ok_or("--extreme needs a value")?.as_str() {
                    "left" => ExtremeArg::Left,
                    "right" => ExtremeArg::Right,
                    "both" => ExtremeArg::Both,
                    other => return Err(format!("--extreme: unknown value {other}")),
                };
            }
            "--receiver-wide" => {
                i += 1;
                receiver_wide = argv
                    .get(i)
                    .ok_or("--receiver-wide needs a value")?
                    .parse()
                    .map_err(|e| format!("--receiver-wide: {e}"))?;
            }
            "--receiver-forward" => {
                i += 1;
                receiver_forward = argv
                    .get(i)
                    .ok_or("--receiver-forward needs a value")?
                    .parse()
                    .map_err(|e| format!("--receiver-forward: {e}"))?;
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
        extreme,
        receiver_wide,
        receiver_forward,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_trial(
    params: SimParams,
    attack_home: bool,
    z_bias: f32,
    shot_extreme: ShotExtreme,
    receiver_wide: f32,
    receiver_forward: f32,
    secs: f32,
    wall_secs: f32,
    gk_graph: &Option<PathBuf>,
) -> Result<(RoundResult, f32, bool, Vec2), String> {
    let opening = if attack_home {
        TeamId::Home
    } else {
        TeamId::Away
    };

    let mut world = MatchWorld::new_kickoff_opening(params, opening);
    // Receiver spawns wide on the same side it will shoot toward — a
    // realistic pass-then-shoot geometry, not an arbitrary combination.
    // World z-sign for "left"/"right" is fixed regardless of home/away (the
    // sim's own team_goal_posts always uses +half_width for the left post),
    // so this doesn't need to depend on attack_home — only the forward (x)
    // offset does, and setup_2v1_harness already handles that internally.
    let receiver_wide_z = match shot_extreme {
        ShotExtreme::Left => receiver_wide,
        ShotExtreme::Right => -receiver_wide,
    };
    let receiver_pos = setup_2v1_harness(
        &mut world,
        attack_home,
        z_bias,
        receiver_wide_z,
        receiver_forward,
    );

    // P2 makes a run across to the opposite flank — a real moving target,
    // not a stationary one — and P1's pass leads that motion.
    let receiver_run_target = Vec2::new(receiver_pos.x, -receiver_wide_z);
    let mut atk_brain: Box<dyn TeamBrain> = Box::new(PassDrillAttackerBrain {
        extreme: shot_extreme,
        receiver_pos,
        receiver_run_target,
        receiver_run_speed: 8.0,
        pass_speed: 25.0,
        goal_half_width: world.params.goal_half_width,
    });
    let mut def_brain: Box<dyn TeamBrain> = build_gk_brain(gk_graph)?;

    let max_ticks = ((secs / FIXED_DT).ceil() as u64).max(1);
    let wall_budget = Duration::from_secs_f32(wall_secs.max(0.5));
    let wall_start = Instant::now();
    let start_h = world.match_state.score_home;
    let start_a = world.match_state.score_away;
    let atk_team = if attack_home { TeamId::Home } else { TeamId::Away };
    let mut saw_p2_receive = false;

    for _tick in 0..max_ticks {
        if wall_start.elapsed() >= wall_budget {
            return Ok((RoundResult::GkTimeout, world.match_state.clock_s, saw_p2_receive, world.ball.pos));
        }

        let (home_api, away_api) = world.build_apis();
        let mut home_out = if attack_home {
            atk_brain.think(&home_api)
        } else {
            def_brain.think(&home_api)
        };
        let mut away_out = if attack_home {
            def_brain.think(&away_api)
        } else {
            atk_brain.think(&away_api)
        };

        apply_2v1_freeze(&mut home_out, &mut away_out, &world, attack_home);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        repark_2v1_inactive(&mut world, attack_home);

        if world.possession.carrier == Some((atk_team, 2)) {
            saw_p2_receive = true;
        }

        if let Some(out) = evaluate_scenario1(&world, attack_home, start_h, start_a) {
            return Ok((RoundResult::from(out), world.match_state.clock_s, saw_p2_receive, world.ball.pos));
        }
    }

    Ok((RoundResult::GkTimeout, world.match_state.clock_s.max(secs), saw_p2_receive, world.ball.pos))
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

    let mut atk_goals = 0u32;
    let mut gk_catches = 0u32;
    let mut gk_timeouts = 0u32;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);

    for i in 0..args.trials {
        let attack_home = ((seed >> (i % 60)) & 1) == 0;
        let z_bias = if ((seed >> ((i + 3) % 60)) & 1) == 0 {
            1.0
        } else {
            -1.0
        };
        let extreme = match args.extreme {
            ExtremeArg::Left => ShotExtreme::Left,
            ExtremeArg::Right => ShotExtreme::Right,
            ExtremeArg::Both => {
                if i % 2 == 0 {
                    ShotExtreme::Left
                } else {
                    ShotExtreme::Right
                }
            }
        };
        let (result, t, p2_received, ball_pos) = match run_trial(
            params.clone(),
            attack_home,
            z_bias,
            extreme,
            args.receiver_wide,
            args.receiver_forward,
            args.secs,
            args.wall_secs,
            &args.gk_graph,
        ) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        match result {
            RoundResult::AttackerGoal => atk_goals += 1,
            RoundResult::GkCatch => gk_catches += 1,
            RoundResult::GkTimeout => gk_timeouts += 1,
        }
        if !args.quiet {
            let tag = match result {
                RoundResult::AttackerGoal => "atk_goal",
                RoundResult::GkCatch => "gk_catch",
                RoundResult::GkTimeout => "gk_timeout",
            };
            let ext = match extreme {
                ShotExtreme::Left => "left",
                ShotExtreme::Right => "right",
            };
            eprintln!(
                "trial {i}: attack={} z_bias={z_bias:+} extreme={ext} {tag} t={t:.2} \
                 p2_received={p2_received} ball=({:.2},{:.2}) [caught_pass={}]",
                if attack_home { "home" } else { "away" },
                ball_pos.x,
                ball_pos.y,
                !p2_received && result == RoundResult::GkCatch,
            );
        }
    }

    let gk_wins = gk_catches + gk_timeouts;
    let json = serde_json::json!({
        "ok": true,
        "scenario": 2,
        "harness": "scenario2_pass_then_shoot_2v1",
        "trials": args.trials,
        "secs": args.secs,
        "wall_secs": args.wall_secs,
        "attacker_goals": atk_goals,
        "gk_catches": gk_catches,
        "gk_timeouts": gk_timeouts,
        "gk_wins": gk_wins,
        "attacker_goal_rate": atk_goals as f32 / args.trials.max(1) as f32,
        "gk_win_rate": gk_wins as f32 / args.trials.max(1) as f32,
    });
    println!("{json}");

    let log_dir = PathBuf::from("data/titanium");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("eval_log2.jsonl");
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
