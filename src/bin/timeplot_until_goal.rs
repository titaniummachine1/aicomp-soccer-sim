//! Headless AIA vs AIA until first goal; write AIA_Debug-compatible TimePlot JSON.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aicomp_soccer_sim::brain::TeamBrain;
use aicomp_soccer_sim::graph::{load_team_graph, GraphBrain};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::brain::TeamId;
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
    let mut home = GraphBrain::new(graph.clone());
    let mut away = GraphBrain::new(graph);

    // Match capture: home|away. Default Home (timeplot_2026-07-22_05-01-57).
    let opening = match std::env::args().nth(2).as_deref() {
        Some("away") | Some("Away") => TeamId::Away,
        _ => TeamId::Home,
    };
    let mut world = MatchWorld::new_kickoff_opening(params, opening);
    eprintln!("opening kickoff: {opening:?}");
    let mut plot = TimePlotRecorder::default();

    let max_time = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(20.0);

    let mut ticks = 0u64;
    let mut last_score = 0u32;
    loop {
        let (home_api, away_api) = world.build_apis();
        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        plot.sample_home(&world, &home_api, &home, &home_out, FIXED_DT);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        ticks += 1;

        let scored_now = world.match_state.score_home + world.match_state.score_away;
        if scored_now > last_score {
            eprintln!(
                "goal at t={:.3}s ticks={ticks} score={}-{} phase={:?} (continuing to {max_time}s)",
                plot_sim_time(&plot),
                world.match_state.score_home,
                world.match_state.score_away,
                world.match_state.phase
            );
            last_score = scored_now;
        }
        if plot_sim_time(&plot) >= max_time {
            eprintln!(
                "done t={:.3}s score={}-{}",
                plot_sim_time(&plot),
                world.match_state.score_home,
                world.match_state.score_away
            );
            break;
        }
        if ticks % 500 == 0 {
            eprintln!(
                "… t={:.1}s ball=({:.1},{:.1}) score={}-{}",
                plot_sim_time(&plot),
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
    let out = soccer_saves_dir()
        .join("Timeplots")
        .join(format!("sim_timeplot_until_goal_{stamp}.json"));
    plot.write_json(&out).expect("write timeplot");
    println!("{}", out.display());
}

fn plot_sim_time(plot: &TimePlotRecorder) -> f32 {
    // Recreate via private field access — use score clocks from a lightweight approach:
    // TimePlotRecorder doesn't expose sim_time publicly; mirror via writing path.
    // Instead track externally — fix by adding getter.
    plot.sim_time()
}
