//! Print the kickoff a pair of graphs actually produces.
//!
//! Faceoff spots are declared per-graph (`ConstructSoccerProperties`), so
//! "where does everyone stand at the whistle" is a property of the two brains
//! playing, not a constant. This shows the declared spots, the world spots
//! they resolve to, and which of them the circle push moved.
//!
//! ```text
//! cargo run --release --bin kickoff_dump -- \
//!     --home graph:data/titanium/Titanium.txt --away aia --opening home
//! ```

use std::env;
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{default_team_brain, soccer_aia_graph_path, BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::MatchWorld;

fn build(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    let path = match input {
        BrainInput::Chase => return Ok(Box::new(ChaseBallBrain::default())),
        BrainInput::Idle => return Ok(Box::new(IdleBrain)),
        BrainInput::Aia => soccer_aia_graph_path(),
        BrainInput::Graph(p) => p.clone(),
        other => return Err(format!("kickoff_dump needs graph:<path>, got {other:?}")),
    };
    let g = cache
        .get_or_compile(&path)
        .map_err(|e| format!("load {path:?}: {e}"))?;
    Ok(Box::new(RuntimeBrain::from_cached(g)))
}

fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .expect("spawn kickoff thread")
        .join()
        .expect("kickoff thread panicked")
}

fn run() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let (mut home_in, mut away_in) = (default_team_brain(), BrainInput::Aia);
    let mut opening = TeamId::Home;
    let mut i = 0;
    while i < argv.len() {
        let mut take = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i).cloned().unwrap_or_default()
        };
        match argv[i].as_str() {
            "--home" => home_in = BrainInput::parse(&take(&mut i)).expect("--home"),
            "--away" => away_in = BrainInput::parse(&take(&mut i)).expect("--away"),
            "--opening" => {
                opening = match take(&mut i).as_str() {
                    "away" => TeamId::Away,
                    _ => TeamId::Home,
                }
            }
            "-h" | "--help" => {
                eprintln!("kickoff_dump [--home <brain>] [--away <brain>] [--opening home|away]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();
    let cache = ProgramCache::default();
    let home = match build(&home_in, &cache) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error --home: {e}");
            return ExitCode::from(1);
        }
    };
    let away = match build(&away_in, &cache) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error --away: {e}");
            return ExitCode::from(1);
        }
    };

    let mut world = MatchWorld::new_kickoff_opening(params.clone(), opening);
    world.set_kickoff_formation(TeamId::Home, home.kickoff_formation());
    world.set_kickoff_formation(TeamId::Away, away.kickoff_formation());
    // Graphs that COMPUTE their faceoff spots only report them after a tick,
    // so run one and let the world adopt the result.
    let (mut home, mut away) = (home, away);
    let (home_api, away_api) = world.build_apis();
    let home_out = home.think(&home_api);
    let away_out = away.think(&away_api);
    world.observe_brain_outputs(&home_out, &away_out);

    println!(
        "kickoff: {:?} takes it   circle r={:.2}   pitch {:.1} x {:.1}",
        opening, params.kickoff_circle_r, params.x_max, params.z_max
    );
    for team in [TeamId::Home, TeamId::Away] {
        let role = if team == opening { "kicking" } else { "receiving" };
        println!("\n{team:?} ({role})");
        let declared = world.kickoff_formations.get(team);
        for p in world.players.iter().filter(|p| p.team == team) {
            let slot = p.id.0 as usize - 1;
            let d = match declared[slot] {
                Some(v) => format!("declared ({:6.2},{:6.2})", v.x, v.y),
                None => "engine default        ".to_string(),
            };
            let r = p.pos.length();
            let mark = if r < params.kickoff_circle_r - 1e-3 {
                "  INSIDE circle"
            } else if (r - params.kickoff_circle_r).abs() < 1e-3 {
                "  pushed to edge"
            } else {
                ""
            };
            println!(
                "  P{}  {d}  ->  world ({:6.2},{:6.2})   {:5.2} m from spot{mark}",
                p.id.0, p.pos.x, p.pos.y, r
            );
        }
    }
    ExitCode::SUCCESS
}
