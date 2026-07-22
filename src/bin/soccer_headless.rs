//! Headless match runner — fishtest-style offline AI evaluation.
//!
//! No window. Fixed-dt `MatchWorld` only. Prints JSON (or JSONL for `--batch`)
//! to stdout so agents / scripts can aggregate runs.
//!
//! ```text
//! cargo run --release --bin soccer_headless -- --secs 30 --home chase --away idle
//! cargo run --release --bin soccer_headless -- --batch 16 --jobs 8 --home chase --away idle
//! cargo run --release --bin soccer_headless -- --help
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{
    run_batch_parallel, run_match_job, reserved_worker_threads, BatchMatchResult, BrainInput,
    GraphEngine, MatchJob, ProgramCache,
};
use aicomp_soccer_sim::brain::TeamId;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::FIXED_DT;

#[derive(Debug)]
struct Args {
    secs: f32,
    home: BrainInput,
    away: BrainInput,
    opening: TeamId,
    seed: Option<u64>,
    params: Option<PathBuf>,
    json_out: Option<PathBuf>,
    until_goal: bool,
    quiet: bool,
    /// When `Some`, run N matches and emit JSONL (even if N == 1).
    batch: Option<usize>,
    jobs: Option<usize>,
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
                            With --batch: base seed; match i uses seed+i
  --params <path>           Params JSON (default: bevy_sim_params_v05.json)
  --json <path>             Also write result JSON to this file (single match)
  --until-goal              Stop at first goal (still capped by --secs)
  --batch <N>               Run N matches in parallel; stdout is JSONL
  --jobs <N>                Thread pool size (default: logical CPUs − 1, min 1)
  --quiet                   No stderr heartbeats
  -h, --help                Show this help

BRAINS:
  chase | idle | test1 | test2 | aia | graph:<path>

NOTE:
  Headless always runs max-speed (no wall-clock wait). Graph teams always use
  the compiled RuntimeBrain (O1) — the slow reference GraphBrain is not
  available from this CLI. --batch uses a micropool that reserves one logical
  CPU for the system/UI (override with --jobs).

EXIT:
  0  finished
  1  bad args / load failure

STDOUT:
  Single match (no --batch): one JSON object (MatchResult).
  --batch: one JSON object per line (JSONL), order = job index.

EXAMPLES:
  cargo run --release --bin soccer_headless -- --secs 20 --home chase --away idle
  cargo run --release --bin soccer_headless -- --secs 40 --home test1 --away test2 --opening home
  cargo run --release --bin soccer_headless -- --home aia --away aia --until-goal --secs 60
  cargo run --release --bin soccer_headless -- --batch 16 --jobs 8 --secs 20 --home chase --away idle --quiet
"
    );
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut secs = 30.0f32;
    let mut home = BrainInput::Chase;
    let mut away = BrainInput::Chase;
    let mut opening = TeamId::Home;
    let mut seed = None;
    let mut params = None;
    let mut json_out = None;
    let mut until_goal = false;
    let mut quiet = false;
    let mut batch = None;
    let mut jobs = None;

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
                home = BrainInput::parse(argv.get(i).ok_or("--home needs a value")?)?;
            }
            "--away" => {
                i += 1;
                away = BrainInput::parse(argv.get(i).ok_or("--away needs a value")?)?;
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
            "--engine" => {
                i += 1;
                let v = argv
                    .get(i)
                    .ok_or("--engine is removed; RuntimeBrain is always used")?
                    .to_ascii_lowercase();
                match v.as_str() {
                    "runtime" | "vm" | "o0" | "o1" => {}
                    "reference" | "ref" | "graphbrain" => {
                        return Err(
                            "--engine reference is disabled; soccer_headless always uses RuntimeBrain (O1)"
                                .into(),
                        );
                    }
                    other => {
                        return Err(format!(
                            "--engine is removed (always RuntimeBrain); got '{other}'"
                        ))
                    }
                }
            }
            "--batch" => {
                i += 1;
                let n: usize = argv
                    .get(i)
                    .ok_or("--batch needs a value")?
                    .parse()
                    .map_err(|e| format!("--batch: {e}"))?;
                if n == 0 {
                    return Err("--batch must be >= 1".into());
                }
                batch = Some(n);
            }
            "--jobs" => {
                i += 1;
                let n: usize = argv
                    .get(i)
                    .ok_or("--jobs needs a value")?
                    .parse()
                    .map_err(|e| format!("--jobs: {e}"))?;
                if n == 0 {
                    return Err("--jobs must be >= 1".into());
                }
                jobs = Some(n);
            }
            "--until-goal" => until_goal = true,
            "--quiet" => quiet = true,
            other => return Err(format!("unknown arg '{other}' (see --help)")),
        }
        i += 1;
    }

    if batch.is_none() {
        if let Some(s) = seed {
            opening = if s % 2 == 0 {
                TeamId::Home
            } else {
                TeamId::Away
            };
        }
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
        batch,
        jobs,
    })
}

fn load_params(path: Option<&PathBuf>) -> SimParams {
    let params_path = path.cloned().unwrap_or_else(default_params_path);
    SimParams::load_from_disk(&params_path).unwrap_or_else(|e| {
        eprintln!(
            "warn: params load failed ({e}); using fallbacks ({})",
            params_path.display()
        );
        SimParams::default()
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

fn run_single(args: &Args, params: SimParams) -> Result<BatchMatchResult, String> {
    let job = MatchJob {
        secs: args.secs,
        home: args.home.clone(),
        away: args.away.clone(),
        opening: args.opening,
        seed: args.seed,
        until_goal: args.until_goal,
        engine: GraphEngine::Runtime,
        params,
        job_index: None,
    };
    let cache = ProgramCache::new();
    run_match_job(&job, Some(&cache), args.quiet)
}

fn run_batch(args: &Args, params: SimParams) -> Vec<Result<BatchMatchResult, String>> {
    let n = args.batch.expect("batch present");
    let base_seed = args.seed.unwrap_or(0);
    let jobs: Vec<MatchJob> = (0..n)
        .map(|i| {
            let seed = base_seed.wrapping_add(i as u64);
            MatchJob {
                secs: args.secs,
                home: args.home.clone(),
                away: args.away.clone(),
                opening: if seed % 2 == 0 {
                    TeamId::Home
                } else {
                    TeamId::Away
                },
                seed: Some(seed),
                until_goal: args.until_goal,
                engine: GraphEngine::Runtime,
                params: params.clone(),
                job_index: Some(i),
            }
        })
        .collect();

    if !args.quiet {
        let pool_n = args.jobs.unwrap_or_else(reserved_worker_threads);
        eprintln!(
            "soccer_headless batch={n} jobs={pool_n} home={} away={} secs={} until_goal={} FIXED_DT={FIXED_DT} (max-speed, JSONL)",
            args.home.label(),
            args.away.label(),
            args.secs,
            args.until_goal
        );
    }

    run_batch_parallel(jobs, args.jobs)
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
    let params = load_params(args.params.as_ref());

    if args.batch.is_some() {
        let results = run_batch(&args, params);
        let mut any_err = false;
        for r in results {
            match r {
                Ok(result) => {
                    let json = serde_json::to_string(&result).expect("serialize BatchMatchResult");
                    println!("{json}");
                    if !result.ok {
                        any_err = true;
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    let fail = serde_json::json!({ "ok": false, "error": e });
                    println!("{fail}");
                    any_err = true;
                }
            }
        }
        if any_err {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        }
    } else {
        match run_single(&args, params) {
            Ok(result) => {
                // Omit batch-only fields for single-match parity with historical MatchResult.
                let json = serde_json::to_string(&serde_json::json!({
                    "ok": result.ok,
                    "fixed_dt": result.fixed_dt,
                    "secs_requested": result.secs_requested,
                    "clock_s": result.clock_s,
                    "ticks": result.ticks,
                    "opening": result.opening,
                    "seed": result.seed,
                    "home": result.home,
                    "away": result.away,
                    "score_home": result.score_home,
                    "score_away": result.score_away,
                    "phase": result.phase,
                    "until_goal": result.until_goal,
                    "goal_stopped": result.goal_stopped,
                }))
                .expect("serialize MatchResult");
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
}
