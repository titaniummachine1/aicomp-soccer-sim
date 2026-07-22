//! Headless Test1 vs Test2 tackle assert (parity probe).
//! For general batch AI eval prefer `soccer_headless` (see README / AGENTS.md).
//!
//! ```text
//! cargo run --release --bin debug_tackle_test -- [max_secs] [--graph]
//! ```
//! Default: scripted Rust brains mirroring build_test1/2.py.
//! `--graph`: load Test1.txt / Test2.txt via GraphBrain (finds graph gaps).

use std::path::PathBuf;

use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph::{load_team_graph, GraphBrain};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::probe_brains::{Test1Brain, Test2Brain};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

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

fn p1(world: &MatchWorld, team: TeamId) -> &aicomp_soccer_sim::Player {
    world
        .players
        .iter()
        .find(|p| p.team == team && p.id == PlayerId(1))
        .expect("P1")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let max_time = args
        .iter()
        .find_map(|a| a.parse::<f32>().ok())
        .unwrap_or(40.0);
    let use_graph = args.iter().any(|a| a == "--graph");

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using defaults");
        SimParams::default()
    });

    let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);

    eprintln!(
        "debug_tackle_test mode={} max={:.1}s FIXED_DT={FIXED_DT} kickoff_delay={}",
        if use_graph { "graph" } else { "scripted" },
        max_time,
        world.params.kickoff_delay_s
    );

    let mut home_s = Test1Brain::default();
    let mut away_s = Test2Brain::default();
    let mut home_g: Option<GraphBrain> = None;
    let mut away_g: Option<GraphBrain> = None;

    if use_graph {
        let dir = soccer_saves_dir();
        let h = load_team_graph(&dir.join("Test1.txt")).unwrap_or_else(|e| {
            panic!("Test1.txt load failed: {e}");
        });
        let a = load_team_graph(&dir.join("Test2.txt")).unwrap_or_else(|e| {
            panic!("Test2.txt load failed: {e}");
        });
        eprintln!(
            "loaded graphs home_nodes={} away_nodes={}",
            h.nodes.len(),
            a.nodes.len()
        );
        home_g = Some(GraphBrain::new(h));
        away_g = Some(GraphBrain::new(a));
    }

    let mut t = 0.0_f32;
    let mut prev_home_has = false;
    let mut prev_away_has = false;
    let mut saw_home_kick = false;
    let mut saw_steal = false;
    let mut steal_t = None;
    let mut min_home_stam = 1.0_f32;

    while t < max_time {
        let (home_api, away_api) = world.build_apis();
        let home_out = if let Some(g) = home_g.as_mut() {
            g.think(&home_api)
        } else {
            home_s.think(&home_api)
        };
        let away_out = if let Some(g) = away_g.as_mut() {
            g.think(&away_api)
        } else {
            away_s.think(&away_api)
        };

        let hp = p1(&world, TeamId::Home);
        let ap = p1(&world, TeamId::Away);
        let home_has = matches!(
            world.possession.carrier,
            Some((TeamId::Home, 1))
        );
        let away_has = matches!(
            world.possession.carrier,
            Some((TeamId::Away, 1))
        );
        min_home_stam = min_home_stam.min(hp.stamina);

        if prev_home_has && !home_has && !away_has && hp.shot_charge < 0.05 {
            // Likely fumbled pickup / whistle — not a deliberate short kick.
        } else if prev_home_has && !home_has && !away_has {
            saw_home_kick = true;
            eprintln!(
                "t={t:.2} HOME_KICK ch={:.2} stam={:.2} ball=({:.1},{:.1})",
                hp.shot_charge,
                hp.stamina,
                world.ball.pos.x,
                world.ball.pos.y
            );
        }
        if !prev_away_has && away_has {
            saw_steal = true;
            steal_t = Some(t);
            eprintln!(
                "t={t:.2} STEAL home_stam={:.3} away_stam={:.3} home_ch={:.2} phase={:?}",
                hp.stamina,
                ap.stamina,
                hp.shot_charge,
                world.match_state.phase
            );
        }
        if prev_away_has && !away_has {
            eprintln!("t={t:.2} AWAY_LOST_BALL");
        }

        // Sparse heartbeat every 2s.
        if ((t / 2.0).floor() as i32) != (((t + FIXED_DT) / 2.0).floor() as i32) {
            eprintln!(
                "t={t:.1} home_has={home_has} away_has={away_has} \
                 h_stam={:.2} a_stam={:.2} h_ch={:.2} \
                 T1=({:.1},{:.1}) O1=({:.1},{:.1}) ball=({:.1},{:.1}) sprint_h={} phase={:?}",
                hp.stamina,
                ap.stamina,
                hp.shot_charge,
                hp.pos.x,
                hp.pos.y,
                ap.pos.x,
                ap.pos.y,
                world.ball.pos.x,
                world.ball.pos.y,
                home_out.commands[0].sprint,
                world.match_state.phase
            );
        }

        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        prev_home_has = home_has;
        prev_away_has = away_has;
        t += FIXED_DT;

        if saw_steal && t - steal_t.unwrap_or(t) > 3.0 {
            break;
        }
    }

    eprintln!("--- SUMMARY ---");
    eprintln!("saw_home_kick={saw_home_kick} saw_steal={saw_steal} min_home_stam={min_home_stam:.3}");
    if let Some(st) = steal_t {
        eprintln!("steal_at={st:.2}s");
    }
    if !saw_steal {
        eprintln!("FAIL: no steal — fix brains/sim before Unity");
        std::process::exit(2);
    }
    eprintln!("OK: steal observed in-sim");
}
