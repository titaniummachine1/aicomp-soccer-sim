//! Headless TimePlot capture for sim ↔ Unity parity.
//!
//! Writes ApiProbe / AIA_Debug-compatible JSON under
//! `%USERPROFILE%/AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/Timeplots/`.
//!
//! Unity side: load `AIA_Debug.txt` (or AIA) in-game and export a TimePlot, then
//! drop it next to the sim file for diff.
//!
//! ```text
//! cargo run --release --bin timeplot_until_goal -- --secs 30 --home aia --away idle
//! cargo run --release --bin timeplot_until_goal -- --secs 45 --both aia --until-goal --goals 1
//! cargo run --release --bin timeplot_until_goal -- --help
//! ```

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use aicomp_soccer_sim::batch::BrainInput;
use aicomp_soccer_sim::brain::{
    ChaseBallBrain, IdleBrain, TeamBrain, TeamId,
};
use aicomp_soccer_sim::graph::load_team_graph;
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::probe_brains::{PerfectControllerBrain, Test1Brain, Test2Brain};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::{MatchPhase, TimePlotRecorder};

#[derive(Debug)]
struct Args {
    secs: f32,
    home: BrainInput,
    away: BrainInput,
    opening: TeamId,
    until_goal: bool,
    goals: u32,
    out: Option<PathBuf>,
    quiet: bool,
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

fn print_help() {
    eprintln!(
        "\
timeplot_until_goal — headless RuntimeBrain TimePlot recorder

USAGE:
  cargo run --release --bin timeplot_until_goal -- [OPTIONS]

OPTIONS:
  --secs <f32>         Max sim seconds (default: 60)
  --home <brain>       Home brain (default: aia)
  --away <brain>       Away brain (default: aia)
  --both <brain>       Set home and away
  --opening home|away  Kickoff side (default: home)
  --until-goal         Stop after --goals scored (still capped by --secs)
  --goals <n>          Goal count to stop at with --until-goal (default: 1)
  --out <path>         Output JSON (default: Saves/Soccer/Timeplots/sim_*.json)
  --quiet              Less stderr
  -h, --help           Help

BRAINS:
  chase | idle | test1 | test2 | perfect (kb|keyboard) | aia | graph:<path>

UNITY CAPTURE:
  1. In AIComp Soccer, set Home to AIA_Debug.txt (stock AIA + probes, DB33)
  2. Play / record TimePlot the same length / opening as this run
  3. Compare series names (Ball.*, T1.*, Ctrl.*, In.*, Aim.*, Sensor.*, Event.*)
     Rebuild Unity graph: python worldcupteams/probes/build_aia_debug.py

AIA GAPS (stock AIA.txt still soft / blank in sim):
  Spherecast + SoccerPlayerSensors1..4 (raw path Null; SoccerGet clear-dirs
  approximated). ConstructSoccerProperties/Country faceoff-only. See
  docs/API_GAPS.md and: python scripts/aia_gap_report.py
"
    );
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut secs = 60.0f32;
    let mut home = aicomp_soccer_sim::batch::default_team_brain();
    let mut away = aicomp_soccer_sim::batch::default_team_brain();
    let mut opening = TeamId::Home;
    let mut until_goal = false;
    let mut goals = 1u32;
    let mut out = None;
    let mut quiet = false;
    let mut i = 0usize;
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
            "--both" => {
                i += 1;
                let b = BrainInput::parse(argv.get(i).ok_or("--both needs a value")?)?;
                home = b.clone();
                away = b;
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
            "--until-goal" => until_goal = true,
            "--goals" => {
                i += 1;
                goals = argv
                    .get(i)
                    .ok_or("--goals needs a value")?
                    .parse()
                    .map_err(|e| format!("--goals: {e}"))?;
                if goals == 0 {
                    return Err("--goals must be >= 1".into());
                }
                until_goal = true;
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(argv.get(i).ok_or("--out needs a path")?));
            }
            "--quiet" => quiet = true,
            other => return Err(format!("unknown arg '{other}' (see --help)")),
        }
        i += 1;
    }
    Ok(Args {
        secs,
        home,
        away,
        opening,
        until_goal,
        goals,
        out,
        quiet,
    })
}

fn build_brain(input: &BrainInput) -> Result<Box<dyn TeamBrain>, String> {
    Ok(match input {
        BrainInput::Chase => Box::new(ChaseBallBrain),
        BrainInput::Idle => Box::new(IdleBrain),
        BrainInput::Test1 => Box::new(Test1Brain::default()),
        BrainInput::Test2 => Box::new(Test2Brain::default()),
        BrainInput::Perfect => Box::new(PerfectControllerBrain),
        BrainInput::Aia => {
            let path = aicomp_soccer_sim::batch::soccer_aia_graph_path();
            let g = load_team_graph(&path).map_err(|e| format!("load AIA: {e}"))?;
            Box::new(RuntimeBrain::compile(g))
        }
        BrainInput::Titanium => Box::new(aicomp_soccer_sim::titanium::TitaniumBrain::default()),
        #[cfg(feature = "nn_train")]
        BrainInput::Trained => Box::new(aicomp_soccer_sim::train::TrainedBrain::default()),
        BrainInput::Graph(path) => {
            let g = load_team_graph(path).map_err(|e| format!("load {path:?}: {e}"))?;
            Box::new(RuntimeBrain::compile(g))
        }
    })
}

fn print_aia_gap_banner(saves: &Path) {
    eprintln!("parity gaps (AIA uses these; sim still soft/blank):");
    eprintln!("  - Spherecast + SoccerPlayerSensors1..4 -> Null (raw Sensor.* not in sim plot)");
    eprintln!("  - SoccerGet clear-dir / opp-goal-dir -> geometric approx (not live casts)");
    eprintln!("  - ConstructSoccerProperties / Country -> faceoff only");
    eprintln!("  - early Ball/T1 path (first 2s RMSE) still open");
    eprintln!("  run: python scripts/aia_gap_report.py");
    eprintln!("  delays: kickoff_delay_s forced ~4.9s (cosmetic GoalPause only); viewer may use 0 — kickoff walk-in/hold/first-kick still run");
    if saves.join("AIA_Debug.txt").is_file() {
        eprintln!(
            "Unity: load AIA_Debug.txt for matching series names ({})",
            saves.join("AIA_Debug.txt").display()
        );
    }
    if !saves.join("AIA3.txt").is_file() && !saves.join("AIA.txt").is_file() {
        eprintln!(
            "warn: missing AIA3.txt / AIA.txt under {}",
            saves.display()
        );
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("try: cargo run --release --bin timeplot_until_goal -- --help");
            return ExitCode::from(1);
        }
    };

    let mut params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });
    // Parity runs must keep Unity post-goal / whistle freeze (~4.9s).
    // Viewer may zero this for demo; never do that here.
    const UNITY_KICKOFF_DELAY_S: f32 = 4.9;
    if (params.kickoff_delay_s - UNITY_KICKOFF_DELAY_S).abs() > 0.05 {
        eprintln!(
            "parity: kickoff_delay_s={:.3} -> {UNITY_KICKOFF_DELAY_S} (Unity GoalPause)",
            params.kickoff_delay_s
        );
        params.kickoff_delay_s = UNITY_KICKOFF_DELAY_S;
    }
    let saves = soccer_saves_dir();
    if !args.quiet {
        print_aia_gap_banner(&saves);
    }

    let mut home = match build_brain(&args.home) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error home: {e}");
            return ExitCode::from(1);
        }
    };
    let mut away = match build_brain(&args.away) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error away: {e}");
            return ExitCode::from(1);
        }
    };

    let mut world = MatchWorld::new_kickoff_opening(params, args.opening);
    if !args.quiet {
        eprintln!(
            "timeplot home={} away={} opening={:?} secs={} until_goal={} goals={} FIXED_DT={FIXED_DT} kickoff_delay_s={:.2} engine=RuntimeBrain",
            args.home.label(),
            args.away.label(),
            args.opening,
            args.secs,
            args.until_goal,
            args.goals,
            world.params.kickoff_delay_s
        );
    }

    let mut plot = TimePlotRecorder::default();
    let mut ticks = 0u64;
    let start_score = world.match_state.score_home + world.match_state.score_away;
    let mut last_score = start_score;
    let mut last_phase = world.match_state.phase;
    let mut whistle_n = 0u32;

    loop {
        let (home_api, away_api) = world.build_apis();
        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        plot.sample_home(&world, &home_api, &home_out, FIXED_DT);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        ticks += 1;

        let phase = world.match_state.phase;
        if !args.quiet
            && last_phase != MatchPhase::GoalPause
            && phase == MatchPhase::GoalPause
        {
            let scored_now = world.match_state.score_home + world.match_state.score_away;
            if scored_now == last_score {
                whistle_n += 1;
                eprintln!(
                    "whistle #{whistle_n} at t={:.3}s ticks={ticks} score={}-{} next_ko={:?} first_kick_done={} exclude={:?}/{:.2}",
                    plot.sim_time(),
                    world.match_state.score_home,
                    world.match_state.score_away,
                    world.match_state.kickoff_team,
                    world.possession.first_kick_done,
                    world.possession.kick_exclude_shooter,
                    world.possession.kick_exclude_left,
                );
            }
        }
        if !args.quiet
            && last_phase == MatchPhase::GoalPause
            && phase == MatchPhase::Kickoff
        {
            eprintln!(
                "kickoff resume t={:.3}s score={}-{} ko={:?} first_kick_done={} exclude={:?} carrier={:?}",
                plot.sim_time(),
                world.match_state.score_home,
                world.match_state.score_away,
                world.match_state.kickoff_team,
                world.possession.first_kick_done,
                world.possession.kick_exclude_shooter,
                world.possession.carrier,
            );
        }
        last_phase = phase;

        let scored_now = world.match_state.score_home + world.match_state.score_away;
        if scored_now > last_score && !args.quiet {
            eprintln!(
                "goal at t={:.3}s ticks={ticks} score={}-{} next_ko={:?} first_kick_done={} exclude={:?}",
                plot.sim_time(),
                world.match_state.score_home,
                world.match_state.score_away,
                world.match_state.kickoff_team,
                world.possession.first_kick_done,
                world.possession.kick_exclude_shooter,
            );
            last_score = scored_now;
        }
        if args.until_goal && scored_now.saturating_sub(start_score) >= args.goals {
            break;
        }
        if plot.sim_time() >= args.secs {
            break;
        }
        if !args.quiet && ticks % 1000 == 0 {
            eprintln!(
                "… t={:.1}s ball=({:.1},{:.1}) score={}-{}",
                plot.sim_time(),
                world.ball.pos.x,
                world.ball.pos.y,
                world.match_state.score_home,
                world.match_state.score_away
            );
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out = args.out.unwrap_or_else(|| {
        let dir = saves.join("Timeplots");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!(
            "sim_{}_vs_{}_{stamp}.json",
            args.home.label().replace(':', "_"),
            args.away.label().replace(':', "_"),
        ))
    });
    if let Err(e) = plot.write_json(&out) {
        eprintln!("error writing {}: {e}", out.display());
        return ExitCode::from(1);
    }
    eprintln!(
        "wrote {}  t={:.3}s ticks={ticks} score={}-{}",
        out.display(),
        plot.sim_time(),
        world.match_state.score_home,
        world.match_state.score_away
    );
    println!("{}", out.display());
    ExitCode::SUCCESS
}
