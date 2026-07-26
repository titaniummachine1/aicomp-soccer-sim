//! How far the wall clamp moves the held ball away from where the carrier's
//! anti-tackle model thinks it is.
//!
//! Titanium models the held ball as `carrier + facing * hold_offset` — a clean
//! circle of radius 1.67 m. The engine then projects that point into the
//! playable AABB (`project_hold_into_playable`), so near a wall the real ball
//! sits somewhere else entirely. Every blocked-arc calculation is done against
//! the unclamped point, so on the boundary the model is aiming at a ball
//! position that does not exist.
//!
//! This measures the gap: how often the clamp bites, how far it displaces the
//! ball, and — the part that decides whether it matters — whether any ball
//! actually lost was lost while clamped.
//!
//!     cargo run --release --bin hold_wall_clamp -- --home A.txt --away B.txt

use std::env;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn brain(input: &BrainInput, cache: &ProgramCache) -> Box<dyn TeamBrain> {
    match input {
        BrainInput::Graph(p) => Box::new(RuntimeBrain::from_cached(
            cache.get_or_compile(p).expect("compile"),
        )),
        _ => Box::new(aicomp_soccer_sim::brain::ChaseBallBrain::default()),
    }
}

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let get = |k: &str| -> Option<String> {
        argv.iter()
            .position(|a| a == k)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let home = BrainInput::parse(&get("--home").expect("--home")).expect("home");
    let away = BrainInput::parse(&get("--away").expect("--away")).expect("away");
    let secs: f32 = get("--secs")
        .unwrap_or_else(|| "180".into())
        .parse()
        .unwrap();

    let cache = ProgramCache::default();
    let (mut h, mut a) = (brain(&home, &cache), brain(&away, &cache));
    let params = SimParams::default();
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);

    let (mut held, mut clamped) = (0u32, 0u32);
    let (mut sum_err, mut max_err) = (0.0f32, 0.0f32);
    // A displacement only matters if it can change an interaction verdict, so
    // count the ones at least half an interact radius.
    let mut material = 0u32;
    let (mut losses, mut losses_while_clamped) = (0u32, 0u32);
    let mut clamped_last = false;

    for _ in 0..((secs / FIXED_DT) as u32) {
        let before = world.possession.carrier;
        let (home_out, away_out) = {
            let (ha, aa) = world.build_apis();
            (h.think(&ha), a.think(&aa))
        };
        world.step_with_commands(&home_out, &away_out, FIXED_DT);

        // Post-step: the ball has been placed from the settled facing, which is
        // the state interaction was judged against.
        let mut this_tick_clamped = false;
        if let Some((t, id)) = world.possession.carrier {
            if let Some(p) = world.players.iter().find(|p| p.team == t && p.id.0 == id) {
                held += 1;
                let modelled = p.hold_pos(params.hold_offset); // what Titanium assumes
                let actual = p.hold_pos_playable(&params); // what the engine uses
                let err = (modelled - actual).length();
                if err > 1e-6 {
                    clamped += 1;
                    this_tick_clamped = true;
                    sum_err += err;
                    max_err = max_err.max(err);
                    if err >= 0.5 * params.interact_radius {
                        material += 1;
                    }
                }
            }
        }
        // A steal, and whether the carrier was on the wall when it happened.
        if let (Some((bt, _)), Some((at, _))) = (before, world.possession.carrier) {
            if bt != at {
                losses += 1;
                if clamped_last {
                    losses_while_clamped += 1;
                }
            }
        }
        clamped_last = this_tick_clamped;
    }

    let pct = |n: u32, d: u32| if d == 0 { 0.0 } else { 100.0 * n as f32 / d as f32 };
    println!("held ticks                 {held}");
    println!(
        "  wall clamp moved the ball {clamped}  ({:.1}% of held ticks)",
        pct(clamped, held)
    );
    println!(
        "  mean displacement         {:.3} m   max {max_err:.3} m",
        if clamped == 0 { 0.0 } else { sum_err / clamped as f32 }
    );
    println!(
        "  >= half interact radius   {material}  ({:.1}% of held ticks)",
        pct(material, held)
    );
    println!("possession changes         {losses}");
    println!(
        "  while the ball was clamped {losses_while_clamped}  ({:.1}%)",
        pct(losses_while_clamped, losses)
    );
}
