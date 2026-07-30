//! Micro-benchmark: run N ticks of self-play and print wall-clock time.
//!
//! ```text
//! cargo run --release --bin bench_sim -- --ticks 3158
//! ```

use std::env;
use std::time::Instant;

use aicomp_soccer_sim::brain::{ChaseBallBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn main() {
    let argv: Vec<String> = env::args().collect();
    let ticks: usize = argv
        .get(1)
        .map(|s| s.parse().unwrap_or(3158))
        .unwrap_or(3158);

    let params_path = default_params_path();
    let params = SimParams::load_from_disk(&params_path).unwrap_or(SimParams::default());

    let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
    let mut home = ChaseBallBrain::default();
    let mut away = ChaseBallBrain::default();

    // Warm up (1 tick to ensure everything is allocated).
    world.step_brains(&mut home, &mut away, FIXED_DT);

    let start = Instant::now();
    for _ in 0..ticks {
        world.step_brains(&mut home, &mut away, FIXED_DT);
    }
    let elapsed = start.elapsed();

    let total_sim_s = ticks as f32 * FIXED_DT;
    let wall_s = elapsed.as_secs_f64();
    let ticks_per_s = ticks as f64 / wall_s;
    let sim_speed = total_sim_s as f64 / wall_s;

    println!("ticks:         {ticks}");
    println!("sim time:      {total_sim_s:.1}s ({:.1} in-game min)", total_sim_s / 60.0);
    println!("wall time:     {wall_s:.6}s");
    println!("ticks/sec:     {ticks_per_s:.0}");
    println!("sim_speed:     {sim_speed:.1}x (sim seconds per wall second)");
    println!("in-game min/s: {:.2}", sim_speed / 60.0);
    println!("FIXED_DT:      {FIXED_DT}s");
    println!("match clock_s: {:.3}s", world.match_state.clock_s);
    println!("score:         {}-{}", world.match_state.score_home, world.match_state.score_away);
}
