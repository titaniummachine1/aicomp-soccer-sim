//! Profile what holds the simulation back.
//!
//! ```text
//! cargo run --release --bin bench_profile -- 31579
//! ```

use std::env;
use std::time::{Duration, Instant};

use aicomp_soccer_sim::brain::{ChaseBallBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn main() {
    let argv: Vec<String> = env::args().collect();
    let ticks: usize = argv
        .get(1)
        .map(|s| s.parse().unwrap_or(31579))
        .unwrap_or(31579);

    let params_path = default_params_path();
    let params = SimParams::load_from_disk(&params_path).unwrap_or(SimParams::default());

    let mut world = MatchWorld::new_kickoff_opening(params, TeamId::Home);
    let mut home = ChaseBallBrain::default();
    let mut away = ChaseBallBrain::default();

    // Warm up.
    world.step_brains(&mut home, &mut away, FIXED_DT);

    let home_mask = home.api_mask();
    let away_mask = away.api_mask();

    let mut t_apis = Duration::ZERO;
    let mut t_think = Duration::ZERO;
    let mut t_step = Duration::ZERO;
    let t_total;

    let start_all = Instant::now();
    for _ in 0..ticks {
        let t0 = Instant::now();
        let (home_api, away_api) =
            world.build_apis_masked(home_mask.as_ref(), away_mask.as_ref());
        let t1 = Instant::now();

        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        let t2 = Instant::now();

        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        let t3 = Instant::now();

        t_apis += t1 - t0;
        t_think += t2 - t1;
        t_step += t3 - t2;
    }
    t_total = start_all.elapsed();

    let pct = |d: Duration| -> f64 {
        100.0 * d.as_secs_f64() / t_total.as_secs_f64()
    };

    println!("=== Profile: {ticks} ticks ({:.1} in-game min) ===", ticks as f32 * FIXED_DT / 60.0);
    println!("total:     {:>10.3}ms  (100.0%)", t_total.as_secs_f64() * 1000.0);
    println!("build_apis:{:>10.3}ms  ({:.1}%)", t_apis.as_secs_f64() * 1000.0, pct(t_apis));
    println!("think:     {:>10.3}ms  ({:.1}%)", t_think.as_secs_f64() * 1000.0, pct(t_think));
    println!("step:      {:>10.3}ms  ({:.1}%)", t_step.as_secs_f64() * 1000.0, pct(t_step));
    println!();
    println!("per tick:");
    println!("  total:      {:>8.1}us", t_total.as_secs_f64() * 1e6 / ticks as f64);
    println!("  build_apis: {:>8.1}us", t_apis.as_secs_f64() * 1e6 / ticks as f64);
    println!("  think:      {:>8.1}us", t_think.as_secs_f64() * 1e6 / ticks as f64);
    println!("  step:       {:>8.1}us", t_step.as_secs_f64() * 1e6 / ticks as f64);
    println!();
    println!("ticks/sec: {:.0}", ticks as f64 / t_total.as_secs_f64());
    println!("score: {}-{}", world.match_state.score_home, world.match_state.score_away);
}
