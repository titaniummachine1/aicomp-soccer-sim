//! Headless AIA vs AIA TimePlot capture.
//!
//! ```text
//! cargo run --release --bin timeplot_until_goal -- [max_secs] [home|away] [goal_count]
//! ```
//! Defaults: max_secs=600, opening=home, stop after 5 total goals (or max_secs).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph::load_team_graph;
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::TimePlotRecorder;

fn soccer_saves_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("Unicorn One")
        .join("AIComp")
        .join("Saves")
        .join("Soccer")
}

fn main() {
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });

    let aia_path = soccer_saves_dir().join("AIA.txt");
    let graph = load_team_graph(&aia_path).unwrap_or_else(|e| {
        panic!("failed to load AIA from {aia_path:?}: {e}");
    });
    let cached = RuntimeBrain::compile_cached(graph);
    let mut home = RuntimeBrain::from_cached(cached.clone());
    let mut away = RuntimeBrain::from_cached(cached);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let max_time = args
        .first()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(600.0);
    let opening = match args.get(1).map(|s| s.as_str()) {
        Some("away") | Some("Away") => TeamId::Away,
        _ => TeamId::Home,
    };
    let goal_target = args
        .get(2)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(5);

    let mut world = MatchWorld::new_kickoff_opening(params, opening);
    eprintln!(
        "AIA vs AIA RuntimeBrain O1 opening={opening:?} stop_at_goals={goal_target} max_secs={max_time} FIXED_DT={FIXED_DT}"
    );
    let mut plot = TimePlotRecorder::default();

    let mut ticks = 0u64;
    let mut last_score = 0u32;
    loop {
        let (home_api, away_api) = world.build_apis();
        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        plot.sample_home(&world, &home_api, &home_out, FIXED_DT);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        ticks += 1;

        let scored_now = world.match_state.score_home + world.match_state.score_away;
        if scored_now > last_score {
            eprintln!(
                "goal at t={:.3}s ticks={ticks} score={}-{} phase={:?}",
                plot.sim_time(),
                world.match_state.score_home,
                world.match_state.score_away,
                world.match_state.phase
            );
            last_score = scored_now;
        }
        if scored_now >= goal_target {
            eprintln!(
                "done (goals) t={:.3}s score={}-{} goals={}",
                plot.sim_time(),
                world.match_state.score_home,
                world.match_state.score_away,
                scored_now
            );
            break;
        }
        if plot.sim_time() >= max_time {
            eprintln!(
                "done (time cap) t={:.3}s score={}-{} goals={}",
                plot.sim_time(),
                world.match_state.score_home,
                world.match_state.score_away,
                scored_now
            );
            break;
        }
        if ticks % 1000 == 0 {
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
    let out_dir = soccer_saves_dir().join("Timeplots");
    let _ = std::fs::create_dir_all(&out_dir);
    let total_goals = world.match_state.score_home + world.match_state.score_away;
    let out = out_dir.join(format!("sim_aia_vs_aia_{total_goals}goals_{stamp}.json"));
    plot.write_json(&out).expect("write timeplot");
    eprintln!(
        "final score {}-{} t={:.3}s ticks={ticks}",
        world.match_state.score_home,
        world.match_state.score_away,
        plot.sim_time()
    );
    println!("{}", out.display());
}
