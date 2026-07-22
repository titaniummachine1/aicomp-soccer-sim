//! Headless match runner — fishtest-style offline AI evaluation.
//!
//! No window. Fixed-dt `MatchWorld` only. Prints one JSON result line to stdout
//! (and a short summary on stderr) so agents / scripts can aggregate runs.
//!
//! ```text
//! cargo run --release --bin soccer_headless -- --secs 30 --home chase --away idle
//! cargo run --release --bin soccer_headless -- --help
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aicomp_soccer_sim::brain::{ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph::{load_team_graph, GraphBrain};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::probe_brains::{Test1Brain, Test2Brain};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use serde::Serialize;

#[derive(Debug, Clone)]
enum BrainSpec {
    Chase,
    Idle,
    Test1,
    Test2,
    Aia,
    Graph(PathBuf),
}

impl BrainSpec {
    fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix("graph:") {
            return Ok(Self::Graph(PathBuf::from(rest)));
        }
        match t.to_ascii_lowercase().as_str() {
            "chase" => Ok(Self::Chase),
            "idle" | "park" => Ok(Self::Idle),
            "test1" => Ok(Self::Test1),
            "test2" => Ok(Self::Test2),
            "aia" => Ok(Self::Aia),
            other => Err(format!(
                "unknown brain '{other}' (chase|idle|test1|test2|aia|graph:<path>)"
            )),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Chase => "chase".into(),
            Self::Idle => "idle".into(),
            Self::Test1 => "test1".into(),
            Self::Test2 => "test2".into(),
            Self::Aia => "aia".into(),
            Self::Graph(p) => format!("graph:{}", p.display()),
        }
    }

    fn build(&self) -> Result<Box<dyn TeamBrain>, String> {
        Ok(match self {
            Self::Chase => Box::new(ChaseBallBrain),
            Self::Idle => Box::new(IdleBrain),
            Self::Test1 => Box::new(Test1Brain::default()),
            Self::Test2 => Box::new(Test2Brain::default()),
            Self::Aia => {
                let path = soccer_saves_dir().join("AIA.txt");
                let g = load_team_graph(&path).map_err(|e| format!("load AIA {path:?}: {e}"))?;
                Box::new(GraphBrain::new(g))
            }
            Self::Graph(path) => {
                let g = load_team_graph(path).map_err(|e| format!("load graph {path:?}: {e}"))?;
                Box::new(GraphBrain::new(g))
            }
        })
    }
}

#[derive(Debug)]
struct Args {
    secs: f32,
    home: BrainSpec,
    away: BrainSpec,
    opening: TeamId,
    seed: Option<u64>,
    params: Option<PathBuf>,
    json_out: Option<PathBuf>,
    until_goal: bool,
    quiet: bool,
}

fn print_help() {
    eprintln!(
        "\
soccer_headless — offline AIComp Soccer match (no window)

USAGE:
  cargo run --release --bin soccer_headless -- [OPTIONS]

OPTIONS:
  --secs <f32>              Sim seconds to run (default: 30)
  --home <brain>            Home brain (default: chase)
  --away <brain>            Away brain (default: chase)
  --opening home|away       Opening kickoff side (default: home)
  --seed <u64>              Deterministic opening: even=home, odd=away
  --params <path>           Params JSON (default: bevy_sim_params_v05.json)
  --json <path>             Also write result JSON to this file
  --until-goal              Stop at first goal (still capped by --secs)
  --quiet                   No stderr heartbeats
  -h, --help                Show this help

BRAINS:
  chase | idle | test1 | test2 | aia | graph:<path>

EXIT:
  0  finished
  1  bad args / load failure

STDOUT:
  One JSON object (MatchResult) — aggregate this for batch / fishtest-style runs.

EXAMPLES:
  cargo run --release --bin soccer_headless -- --secs 20 --home chase --away idle
  cargo run --release --bin soccer_headless -- --secs 40 --home test1 --away test2 --opening home
  cargo run --release --bin soccer_headless -- --home aia --away aia --until-goal --secs 60
"
    );
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut secs = 30.0f32;
    let mut home = BrainSpec::Chase;
    let mut away = BrainSpec::Chase;
    let mut opening = TeamId::Home;
    let mut seed = None;
    let mut params = None;
    let mut json_out = None;
    let mut until_goal = false;
    let mut quiet = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--secs" => {
                i += 1;
                secs = argv
                    .get(i)
                    .ok_or("--secs needs a value")?
                    .parse()
                    .map_err(|e| format!("--secs: {e}"))?;
            }
            "--home" => {
                i += 1;
                home = BrainSpec::parse(argv.get(i).ok_or("--home needs a value")?)?;
            }
            "--away" => {
                i += 1;
                away = BrainSpec::parse(argv.get(i).ok_or("--away needs a value")?)?;
            }
            "--opening" => {
                i += 1;
                opening = match argv
                    .get(i)
                    .ok_or("--opening needs home|away")?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "home" => TeamId::Home,
                    "away" => TeamId::Away,
                    other => return Err(format!("--opening: expected home|away, got {other}")),
                };
            }
            "--seed" => {
                i += 1;
                seed = Some(
                    argv.get(i)
                        .ok_or("--seed needs a value")?
                        .parse()
                        .map_err(|e| format!("--seed: {e}"))?,
                );
            }
            "--params" => {
                i += 1;
                params = Some(PathBuf::from(argv.get(i).ok_or("--params needs a path")?));
            }
            "--json" => {
                i += 1;
                json_out = Some(PathBuf::from(argv.get(i).ok_or("--json needs a path")?));
            }
            "--until-goal" => until_goal = true,
            "--quiet" => quiet = true,
            other => return Err(format!("unknown arg '{other}' (see --help)")),
        }
        i += 1;
    }

    if let Some(s) = seed {
        opening = if s % 2 == 0 {
            TeamId::Home
        } else {
            TeamId::Away
        };
    }

    Ok(Args {
        secs,
        home,
        away,
        opening,
        seed,
        params,
        json_out,
        until_goal,
        quiet,
    })
}

fn soccer_saves_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("Unicorn One")
        .join("AIComp")
        .join("Saves")
        .join("Soccer")
}

#[derive(Serialize)]
struct MatchResult {
    ok: bool,
    fixed_dt: f32,
    secs_requested: f32,
    clock_s: f32,
    ticks: u64,
    opening: &'static str,
    seed: Option<u64>,
    home: String,
    away: String,
    score_home: u32,
    score_away: u32,
    phase: String,
    until_goal: bool,
    goal_stopped: bool,
}

fn opening_str(t: TeamId) -> &'static str {
    match t {
        TeamId::Home => "home",
        TeamId::Away => "away",
    }
}

fn phase_str(world: &MatchWorld) -> String {
    format!("{:?}", world.match_state.phase)
}

fn run(args: Args) -> Result<MatchResult, String> {
    let params_path = args
        .params
        .clone()
        .unwrap_or_else(default_params_path);
    let params = SimParams::load_from_disk(&params_path).unwrap_or_else(|e| {
        eprintln!("warn: params load failed ({e}); using fallbacks ({})", params_path.display());
        SimParams::default()
    });

    let mut home = args.home.build()?;
    let mut away = args.away.build()?;
    let mut world = MatchWorld::new_kickoff_opening(params, args.opening);

    if !args.quiet {
        eprintln!(
            "soccer_headless home={} away={} opening={} secs={} until_goal={} FIXED_DT={FIXED_DT}",
            args.home.label(),
            args.away.label(),
            opening_str(args.opening),
            args.secs,
            args.until_goal
        );
    }

    let start_score = world.match_state.score_home + world.match_state.score_away;
    let mut ticks = 0u64;
    let mut goal_stopped = false;
    // Use wall sim time (ticks*dt), not match clock_s — clock pauses in Kickoff/GoalPause.
    let max_ticks = ((args.secs / FIXED_DT).ceil() as u64).max(1);

    while ticks < max_ticks {
        world.step_brains(&mut *home, &mut *away, FIXED_DT);
        ticks += 1;
        if args.until_goal {
            let scored = world.match_state.score_home + world.match_state.score_away;
            if scored > start_score {
                goal_stopped = true;
                break;
            }
        }
        if !args.quiet && ticks % 500 == 0 {
            eprintln!(
                "  t={:.1}s score={}-{} phase={:?}",
                ticks as f32 * FIXED_DT,
                world.match_state.score_home,
                world.match_state.score_away,
                world.match_state.phase
            );
        }
    }

    Ok(MatchResult {
        ok: true,
        fixed_dt: FIXED_DT,
        secs_requested: args.secs,
        clock_s: ticks as f32 * FIXED_DT,
        ticks,
        opening: opening_str(args.opening),
        seed: args.seed,
        home: args.home.label(),
        away: args.away.label(),
        score_home: world.match_state.score_home,
        score_away: world.match_state.score_away,
        phase: phase_str(&world),
        until_goal: args.until_goal,
        goal_stopped,
    })
}

fn write_json(path: &Path, json: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    fs::write(path, json).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("try: cargo run --release --bin soccer_headless -- --help");
            return ExitCode::from(1);
        }
    };
    let json_path = args.json_out.clone();
    match run(args) {
        Ok(result) => {
            let json = serde_json::to_string(&result).expect("serialize MatchResult");
            println!("{json}");
            if let Some(path) = json_path {
                if let Err(e) = write_json(&path, &json) {
                    eprintln!("error writing --json: {e}");
                    return ExitCode::from(1);
                }
            }
            if !result.ok {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            let fail = serde_json::json!({ "ok": false, "error": e });
            println!("{fail}");
            ExitCode::from(1)
        }
    }
}
