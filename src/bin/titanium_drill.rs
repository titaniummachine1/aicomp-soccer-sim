//! Titanium Scenario 1 **training harness** (not a general match mode).
//!
//! Each round (roles swap Home/Away):
//! - Attacker (P1) at **center** with the ball
//! - GK (P4) already on **cone-bisector cover**
//! - Everyone else parked off-pitch / frozen
//! - **Attacker wins** = scores
//! - **GK wins** = intercepts / takes control of the ball, **or** timeout
//!
//! Timeout defaults to ~60s **wall clock** (headless is accelerated; that is the
//! “one real minute of trying” budget).
//!
//! ```text
//! cargo run --release --bin titanium_drill -- --trials 10
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::Vec2;

use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::scenario::{evaluate_scenario1, Scenario1Outcome};
use aicomp_soccer_sim::titanium::{
    apply_1v1_freeze, repark_1v1_inactive, setup_1v1_harness, TitaniumBrain,
};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

#[derive(Debug)]
struct Args {
    trials: u32,
    /// Soft game-time cap (seconds of match clock).
    secs: f32,
    /// Hard wall-clock budget per trial (real minute in accelerated headless).
    wall_secs: f32,
    quiet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundResult {
    AttackerGoal,
    /// GK intercepted / held the ball.
    GkCatch,
    /// Time expired → counts as GK win.
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

#[derive(Debug, Clone, Copy)]
struct TrialDiagnostics {
    release_at_t: f32,
    release_charge: f32,
    release_pos: Vec2,
    release_vel: Vec2,
    catch_atk: Vec2,
    catch_gk: Vec2,
    catch_ball: Vec2,
    loose_before_catch: bool,
}

fn player_pos(world: &MatchWorld, team: TeamId, id: u8) -> Vec2 {
    world
        .players
        .iter()
        .find(|p| p.team == team && p.id == PlayerId(id))
        .map(|p| p.pos)
        .unwrap_or(Vec2::ZERO)
}

fn trial_diagnostics(
    world: &MatchWorld,
    atk_team: TeamId,
    gk_team: TeamId,
    release_at_t: f32,
    release_charge: f32,
    release_pos: Vec2,
    release_vel: Vec2,
    loose_before_catch: bool,
) -> TrialDiagnostics {
    TrialDiagnostics {
        release_at_t,
        release_charge,
        release_pos,
        release_vel,
        catch_atk: player_pos(world, atk_team, 1),
        catch_gk: player_pos(world, gk_team, 4),
        catch_ball: world.ball.pos,
        loose_before_catch,
    }
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut trials = 10u32;
    let mut secs = 180.0f32; // generous sim clock; wall budget is the real stop
    let mut wall_secs = 60.0f32;
    let mut quiet = false;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "titanium_drill — Scenario 1 (atk center+ball, GK pre-set on cover; \
                     only GK + attacker visible)\n\
                     Atk wins: goal. GK wins: catch/intercept OR timeout.\n\
                     --trials N (default 10)\n\
                     --secs F     soft match-clock cap (default 180)\n\
                     --wall-secs F  real-time budget / trial (default 60)\n\
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
    })
}

fn run_trial(
    params: SimParams,
    attack_home: bool,
    z_bias: f32,
    secs: f32,
    wall_secs: f32,
) -> (RoundResult, f32, f32, bool, TrialDiagnostics) {
    let opening = if attack_home {
        TeamId::Home
    } else {
        TeamId::Away
    };
    let atk_team = opening;
    let gk_team = atk_team.other();

    let mut world = MatchWorld::new_kickoff_opening(params, opening);
    setup_1v1_harness(&mut world, attack_home, z_bias);

    let mut atk_brain = TitaniumBrain::new(false);
    let mut def_brain = TitaniumBrain::new(false);
    let max_ticks = ((secs / FIXED_DT).ceil() as u64).max(1);
    let wall_budget = Duration::from_secs_f32(wall_secs.max(0.5));
    let wall_start = Instant::now();
    let start_h = world.match_state.score_home;
    let start_a = world.match_state.score_away;
    let mut max_charge = 0.0f32;
    let mut saw_release = false;
    let mut was_held = true;
    let mut release_at_t = -1.0f32;
    let mut release_charge = -1.0f32;
    let mut release_pos = Vec2::ZERO;
    let mut release_vel = Vec2::ZERO;

    for _tick in 0..max_ticks {
        if wall_start.elapsed() >= wall_budget {
            return (
                RoundResult::GkTimeout,
                world.match_state.clock_s,
                max_charge,
                saw_release,
                trial_diagnostics(
                    &world,
                    atk_team,
                    gk_team,
                    release_at_t,
                    release_charge,
                    release_pos,
                    release_vel,
                    false,
                ),
            );
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

        apply_1v1_freeze(&mut home_out, &mut away_out, &world, attack_home);

        let attacker_charge_before = world
            .players
            .iter()
            .find(|p| p.team == atk_team && p.id == PlayerId(1))
            .map(|p| p.shot_charge)
            .unwrap_or(-1.0);
        let ball_was_held_before_step = world.ball.held;
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        repark_1v1_inactive(&mut world, attack_home);

        if let Some(p) = world
            .players
            .iter()
            .find(|p| p.team == atk_team && p.id == PlayerId(1))
        {
            max_charge = max_charge.max(p.shot_charge);
        }
        if was_held && !world.ball.held {
            saw_release = true;
            if release_at_t < 0.0 {
                release_at_t = world.match_state.clock_s;
                release_charge = attacker_charge_before;
                release_pos = world.ball.pos;
                release_vel = world.ball.vel;
            }
        }
        was_held = world.ball.held;

        if let Some(out) = evaluate_scenario1(&world, attack_home, start_h, start_a) {
            return (
                RoundResult::from(out),
                world.match_state.clock_s,
                max_charge,
                saw_release,
                trial_diagnostics(
                    &world,
                    atk_team,
                    gk_team,
                    release_at_t,
                    release_charge,
                    release_pos,
                    release_vel,
                    !ball_was_held_before_step,
                ),
            );
        }
    }

    (
        RoundResult::GkTimeout,
        world.match_state.clock_s.max(secs),
        max_charge,
        saw_release,
        trial_diagnostics(
            &world,
            atk_team,
            gk_team,
            release_at_t,
            release_charge,
            release_pos,
            release_vel,
            false,
        ),
    )
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
    let mut attack_home_n = 0u32;
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
        if attack_home {
            attack_home_n += 1;
        }
        let (result, t, max_ch, released, diag) = run_trial(
            params.clone(),
            attack_home,
            z_bias,
            args.secs,
            args.wall_secs,
        );
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
            eprintln!(
                "trial {i}: attack={} z_bias={z_bias:+} {tag} t={t:.2} max_charge={max_ch:.2} released={released} \
                 | release_at_t={:.2} charge={:.2} ball=({:.2},{:.2}) vel=({:.2},{:.2}) \
                 | catch atk=({:.2},{:.2}) gk=({:.2},{:.2}) ball=({:.2},{:.2}) loose_before={}",
                if attack_home { "home" } else { "away" },
                diag.release_at_t,
                diag.release_charge,
                diag.release_pos.x,
                diag.release_pos.y,
                diag.release_vel.x,
                diag.release_vel.y,
                diag.catch_atk.x,
                diag.catch_atk.y,
                diag.catch_gk.x,
                diag.catch_gk.y,
                diag.catch_ball.x,
                diag.catch_ball.y,
                diag.loose_before_catch,
            );
        }
    }

    let gk_wins = gk_catches + gk_timeouts;
    let json = serde_json::json!({
        "ok": true,
        "scenario": 1,
        "harness": "scenario1_center_atk_vs_gk",
        "trials": args.trials,
        "secs": args.secs,
        "wall_secs": args.wall_secs,
        "attacker_goals": atk_goals,
        "gk_catches": gk_catches,
        "gk_timeouts": gk_timeouts,
        "gk_wins": gk_wins,
        "attack_home_trials": attack_home_n,
        "attacker_goal_rate": atk_goals as f32 / args.trials.max(1) as f32,
        "gk_win_rate": gk_wins as f32 / args.trials.max(1) as f32,
    });
    println!("{json}");

    let log_dir = PathBuf::from("data/titanium");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("eval_log.jsonl");
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
