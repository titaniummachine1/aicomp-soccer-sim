//! Debug tool: load a single graph (no opponent needed), step it N ticks,
//! and dump every DebugDrawDisc it submits on the final tick.
//!
//! Built for verifying probe graphs (see worldcupteams/probes/) offline —
//! DebugDrawDisc actually executes in the compiled RuntimeBrain (unlike
//! TimePlot, which compiles to a const-null stub — see
//! src/graph_vm/lower.rs), so this is the reliable way to read a probe
//! graph's published values without needing Unity.
//!
//! ```text
//! cargo run --release --bin probe_dump -- --home graph:<path> --ticks 3
//! ```

use std::env;
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{default_team_brain, soccer_aia_graph_path, BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn build_brain(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    let path = match input {
        BrainInput::Chase => return Ok(Box::new(aicomp_soccer_sim::brain::ChaseBallBrain)),
        BrainInput::Idle => return Ok(Box::new(IdleBrain)),
        BrainInput::Aia => soccer_aia_graph_path(),
        BrainInput::Graph(p) => p.clone(),
        other => return Err(format!("probe_dump supports chase/idle/aia/graph:<path>, got {other:?}")),
    };
    let g = cache.get_or_compile(&path).map_err(|e| format!("load graph {path:?}: {e}"))?;
    Ok(Box::new(RuntimeBrain::from_cached(g)))
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut home = default_team_brain();
    let mut ticks = 1u32;
    let mut ball_state: Option<(f32, f32, f32, f32)> = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                eprintln!("probe_dump -- --home graph:<path> [--ticks N]\nDumps DebugDrawDisc entries submitted by --home on the final tick.");
                return ExitCode::SUCCESS;
            }
            "--home" => {
                i += 1;
                home = match BrainInput::parse(argv.get(i).expect("--home needs a value")) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(1);
                    }
                };
            }
            "--ticks" => {
                i += 1;
                ticks = argv.get(i).expect("--ticks needs a value").parse().expect("--ticks: bad number");
            }
            "--ball" => {
                // --ball x,z,vx,vz : inject a known free-ball state, then print
                // BOTH the graph's drawn prediction and the simulator's own
                // rollout of that same state so they can be compared directly.
                i += 1;
                let v: Vec<f32> = argv
                    .get(i)
                    .expect("--ball needs x,z,vx,vz")
                    .split(',')
                    .map(|s| s.trim().parse().expect("--ball: bad number"))
                    .collect();
                assert_eq!(v.len(), 4, "--ball needs exactly x,z,vx,vz");
                ball_state = Some((v[0], v[1], v[2], v[3]));
            }
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();
    let cache = ProgramCache::default();
    let mut home_brain = match build_brain(&home, &cache) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error home: {e}");
            return ExitCode::from(1);
        }
    };
    let mut away_brain: Box<dyn TeamBrain> = Box::new(IdleBrain);

    let mut world = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);

    let mut last_discs = Vec::new();
    let mut last_lines = Vec::new();
    let mut last_plots: Vec<(String, f32)> = Vec::new();
    for tick_idx in 0..ticks {
        // Inject ONCE, then let the ball roll naturally. Re-injecting every
        // tick pins the speed constant, which any airborne detector reasonably
        // reads as "still flying" -- a test artifact, not graph behaviour.
        if tick_idx == 0 {
        if let Some((x, z, vx, vz)) = ball_state {
            world.ball.held = false;
            world.ball.pos = bevy::prelude::Vec2::new(x, z);
            world.ball.vel = bevy::prelude::Vec2::new(vx, vz);
            world.ball.height = world.params.ball_rest_height;
            world.ball.vel_y = 0.0;
        }
        }
        let (home_api, away_api) = world.build_apis();
        aicomp_soccer_sim::debug_draw::begin_frame();
        let home_out = home_brain.think(&home_api);
        let frame = aicomp_soccer_sim::debug_draw::snapshot();
        last_discs = frame.discs;
        last_lines = frame.lines;
        last_plots = frame.plots;
        aicomp_soccer_sim::debug_draw::begin_frame();
        let away_out = away_brain.think(&away_api);
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
    }

    println!("{}", aicomp_soccer_sim::graph_vm::diagnostics::banner().trim_end());
    for (name, v) in &last_plots {
        println!("PLOT	{name}	{v:.4}");
    }
    for l in &last_lines {
        let name = disc_color_name(l.rgba);
        println!(
            "LINE\t{name}\ta=({:.3},{:.3})\tb=({:.3},{:.3})",
            l.a.x, l.a.y, l.b.x, l.b.y
        );
    }
    for d in &last_discs {
        let name = disc_color_name(d.rgba);
        println!("DISC\t{name}\tx={:.4}\ty={:.4}", d.center.x, d.center.y);
    }

    if let Some((x, z, vx, vz)) = ball_state {
        // Ground truth: the simulator's own tick-by-tick rollout of the same
        // state, to compare against the graph's drawn terminal point above.
        let mut b = aicomp_soccer_sim::ball::Ball {
            pos: bevy::prelude::Vec2::new(x, z),
            vel: bevy::prelude::Vec2::new(vx, vz),
            height: params.ball_rest_height,
            vel_y: 0.0,
            held: false,
        };
        let path = aicomp_soccer_sim::predict::predict_ball_path(&b, &params, FIXED_DT, 30.0);
        let last = path.last().expect("path always has a sample");
        println!(
            "SIM\trest=({:.4},{:.4})\tt={:.3}s\tsamples={}",
            last.pos.x,
            last.pos.y,
            last.t,
            path.len()
        );
        b.pos = bevy::prelude::Vec2::new(x, z);
    }
    ExitCode::SUCCESS
}

fn disc_color_name(rgba: [f32; 4]) -> String {
    for name in ["White", "Red", "Green", "Yellow", "Orange", "Magenta", "Cyan", "Blue", "Gray", "Black"] {
        let named = aicomp_soccer_sim::debug_draw::named_rgba(name);
        if (named[0] - rgba[0]).abs() < 1e-3 && (named[1] - rgba[1]).abs() < 1e-3 && (named[2] - rgba[2]).abs() < 1e-3 {
            return name.to_string();
        }
    }
    format!("{rgba:?}")
}
