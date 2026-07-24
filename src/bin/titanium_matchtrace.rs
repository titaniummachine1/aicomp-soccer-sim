//! Debug tool: dump the position history leading up to every goal in a full
//! match, so a human (or an agent) can see what actually happened without
//! guessing from the final score alone.
//!
//! Pure observability — no strategy here, just recording + printing.
//!
//! ```text
//! cargo run --release --bin titanium_matchtrace -- --home graph:data/titanium/Titanium.txt --away aia3 --seed 0 --opening away --win-goals 3
//! ```

use std::collections::VecDeque;
use std::env;
use std::process::ExitCode;

use bevy::prelude::Vec2;

use aicomp_soccer_sim::batch::{
    default_team_brain, soccer_aia_graph_path, BrainInput, ProgramCache,
};
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn build_brain(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    // Built-ins matter here as much as graphs: running a symmetric built-in
    // against itself is the control that tells you whether an asymmetry is in
    // a team's logic or in the simulator itself.
    let path = match input {
        BrainInput::Chase => return Ok(Box::new(aicomp_soccer_sim::brain::ChaseBallBrain)),
        BrainInput::Idle => return Ok(Box::new(aicomp_soccer_sim::brain::IdleBrain)),
        BrainInput::Aia => soccer_aia_graph_path(),
        BrainInput::Graph(p) => p.clone(),
        other => {
            return Err(format!(
                "titanium_matchtrace supports chase/idle/aia/graph:<path>, got {other:?}"
            ))
        }
    };
    let g = cache
        .get_or_compile(&path)
        .map_err(|e| format!("load graph {path:?}: {e}"))?;
    Ok(Box::new(RuntimeBrain::from_cached(g)))
}

#[derive(Debug)]
struct Args {
    secs: f32,
    home: BrainInput,
    away: BrainInput,
    opening: TeamId,
    seed: Option<u64>,
    win_goals: Option<u32>,
    window_s: f32,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut secs = 300.0f32;
    let mut home = default_team_brain();
    let mut away = BrainInput::Aia;
    let mut opening = TeamId::Home;
    let mut seed = None;
    let mut win_goals = Some(5u32);
    let mut window_s = 3.0f32;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                eprintln!(
                    "titanium_matchtrace — dump position history before every goal\n\
                     --secs F         soft match-clock cap (default 300)\n\
                     --home <brain>   graph:<path> or aia (default: cascade)\n\
                     --away <brain>   graph:<path> or aia (default: aia)\n\
                     --opening home|away (default home)\n\
                     --seed N\n\
                     --win-goals N    stop after either side reaches N (default 5)\n\
                     --window F       seconds of history printed before each goal (default 3.0)"
                );
                std::process::exit(0);
            }
            "--secs" => {
                i += 1;
                secs = argv.get(i).ok_or("--secs needs a value")?.parse().map_err(|e| format!("--secs: {e}"))?;
            }
            "--home" => {
                i += 1;
                home = BrainInput::parse(argv.get(i).ok_or("--home needs a value")?)?;
            }
            "--away" => {
                i += 1;
                away = BrainInput::parse(argv.get(i).ok_or("--away needs a value")?)?;
            }
            "--opening" => {
                i += 1;
                opening = match argv.get(i).ok_or("--opening needs a value")?.as_str() {
                    "home" => TeamId::Home,
                    "away" => TeamId::Away,
                    other => return Err(format!("--opening: unknown value {other}")),
                };
            }
            "--seed" => {
                i += 1;
                seed = Some(argv.get(i).ok_or("--seed needs a value")?.parse().map_err(|e| format!("--seed: {e}"))?);
            }
            "--win-goals" => {
                i += 1;
                win_goals = Some(argv.get(i).ok_or("--win-goals needs a value")?.parse().map_err(|e| format!("--win-goals: {e}"))?);
            }
            "--window" => {
                i += 1;
                window_s = argv.get(i).ok_or("--window needs a value")?.parse().map_err(|e| format!("--window: {e}"))?;
            }
            other => return Err(format!("unknown arg {other}")),
        }
        i += 1;
    }
    Ok(Args { secs, home, away, opening, seed, win_goals, window_s })
}

#[derive(Clone, Copy)]
struct Sample {
    t: f32,
    ball: Vec2,
    ball_held: bool,
    carrier: Option<(TeamId, u8)>,
    home_pos: [Vec2; 4],
    away_pos: [Vec2; 4],
}

fn fmt_v(v: Vec2) -> String {
    format!("({:6.1},{:5.1})", v.x, v.y)
}

fn print_sample(s: &Sample) {
    let carrier = match s.carrier {
        Some((t, id)) => format!("{t:?}P{id}"),
        None => "-".into(),
    };
    eprintln!(
        "  t={:6.2} ball={}{} carrier={:<8} H1={} H2={} H3={} H4={} | A1={} A2={} A3={} A4={}",
        s.t,
        fmt_v(s.ball),
        if s.ball_held { "*" } else { " " },
        carrier,
        fmt_v(s.home_pos[0]),
        fmt_v(s.home_pos[1]),
        fmt_v(s.home_pos[2]),
        fmt_v(s.home_pos[3]),
        fmt_v(s.away_pos[0]),
        fmt_v(s.away_pos[1]),
        fmt_v(s.away_pos[2]),
        fmt_v(s.away_pos[3]),
    );
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
    let cache = ProgramCache::new();
    let mut home_brain = match build_brain(&args.home, &cache) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error home: {e}");
            return ExitCode::from(1);
        }
    };
    let mut away_brain = match build_brain(&args.away, &cache) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error away: {e}");
            return ExitCode::from(1);
        }
    };

    let opening = if let Some(s) = args.seed {
        if s % 2 == 0 { TeamId::Home } else { TeamId::Away }
    } else {
        args.opening
    };
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), opening);
    if world.params.kickoff_delay_s < 1.0 {
        world.params.kickoff_delay_s = 4.9;
    }

    eprintln!(
        "titanium_matchtrace home={} away={} opening={:?} secs={} win_goals={:?}",
        args.home.label(),
        args.away.label(),
        opening,
        args.secs,
        args.win_goals,
    );

    let window_ticks = ((args.window_s / FIXED_DT).ceil() as usize).max(1);
    let mut history: VecDeque<Sample> = VecDeque::with_capacity(window_ticks + 1);
    let max_ticks = ((args.secs / FIXED_DT).ceil() as u64).max(1);
    let mut last_total = world.match_state.score_home + world.match_state.score_away;

    for _tick in 0..max_ticks {
        let (home_api, away_api) = world.build_apis();
        let home_out = home_brain.think(&home_api);
        let away_out = away_brain.think(&away_api);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);

        let pos = |team: TeamId, id: u8| -> Vec2 {
            world
                .players
                .iter()
                .find(|p| p.team == team && p.id.0 == id)
                .map(|p| p.pos)
                .unwrap_or(Vec2::ZERO)
        };
        let sample = Sample {
            t: world.match_state.clock_s,
            ball: world.ball.pos,
            ball_held: world.ball.held,
            carrier: world.possession.carrier,
            home_pos: [pos(TeamId::Home, 1), pos(TeamId::Home, 2), pos(TeamId::Home, 3), pos(TeamId::Home, 4)],
            away_pos: [pos(TeamId::Away, 1), pos(TeamId::Away, 2), pos(TeamId::Away, 3), pos(TeamId::Away, 4)],
        };
        if history.len() == window_ticks {
            history.pop_front();
        }
        history.push_back(sample);

        let sh = world.match_state.score_home;
        let sa = world.match_state.score_away;
        let total = sh + sa;
        if total > last_total {
            eprintln!(
                "=== GOAL t={:.2}s score={sh}-{sa} (last {:.1}s of positions) ===",
                sample.t, args.window_s
            );
            for s in history.iter() {
                print_sample(s);
            }
            eprintln!();
            last_total = total;
        }
        if let Some(win) = args.win_goals {
            if sh >= win || sa >= win {
                break;
            }
        }
    }

    eprintln!(
        "final: {}-{} at t={:.1}s",
        world.match_state.score_home, world.match_state.score_away, world.match_state.clock_s
    );
    ExitCode::SUCCESS
}
