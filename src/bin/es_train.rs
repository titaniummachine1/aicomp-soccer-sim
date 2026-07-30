use std::time::Instant;

use aicomp_soccer_sim::batch::{build_brain, BrainInput, GraphEngine, ProgramCache};
use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use aicomp_soccer_sim::api::ApiFieldMask;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::train::{NetWeights, TrainedBrain, INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

struct Candidate {
    weights: NetWeights,
    fitness: f32,
    goals_for: u32,
    goals_against: u32,
}

impl Candidate {
    fn new_random(_rng: &mut StdRng) -> Self {
        Self {
            weights: NetWeights::random(INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM),
            fitness: 0.0,
            goals_for: 0,
            goals_against: 0,
        }
    }

    fn from_weights(weights: NetWeights) -> Self {
        Self {
            weights,
            fitness: 0.0,
            goals_for: 0,
            goals_against: 0,
        }
    }

    fn save(&self, path: &str) {
        match self.weights.save_to_path(path) {
            Ok(_) => eprintln!("  saved weights to {path}"),
            Err(e) => eprintln!("  failed to save weights: {e}"),
        }
    }
}

const INGAME_MIN_PER_PHASE: f32 = 90.0;
const TIME_SCALE: f32 = 20.0;
const SECS_PER_MIN: f32 = 60.0;
/// Sim seconds for one phase = in-game minutes * 60 / time_scale
fn phase_sim_secs() -> f32 {
    INGAME_MIN_PER_PHASE * SECS_PER_MIN / TIME_SCALE
}

struct BenchedBrain<'a> {
    inner: &'a mut dyn TeamBrain,
    active: usize,
    is_home: bool,
}

impl<'a> TeamBrain for BenchedBrain<'a> {
    fn think(&mut self, api: &aicomp_soccer_sim::api::TeamApi) -> BrainOutput {
        let mut out = self.inner.think(api);
        if self.active < 4 {
            let params = SimParams::default();
            let goal_x = if self.is_home { -params.goal_line_x } else { params.goal_line_x };
            for i in self.active..4 {
                out.commands[i] = BrainCommand {
                    move_to: bevy::prelude::Vec2::new(goal_x, 0.0),
                    sprint: false,
                    interact: false,
                };
            }
        }
        out
    }

    fn kickoff_formation(&self) -> [Option<bevy::prelude::Vec2>; 4] {
        self.inner.kickoff_formation()
    }

    fn api_mask(&self) -> Option<ApiFieldMask> {
        self.inner.api_mask()
    }
}

fn run_phase(
    home_brain: &mut dyn TeamBrain,
    away_brain: &mut dyn TeamBrain,
    opening: TeamId,
    params: &SimParams,
    secs: f32,
    active_players: usize,
) -> (u32, u32, f32, f32, u32, u32) {
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), opening);
    world.set_kickoff_formation(TeamId::Home, home_brain.kickoff_formation());
    world.set_kickoff_formation(TeamId::Away, away_brain.kickoff_formation());
    if world.params.kickoff_delay_s < 1.0 {
        world.params.kickoff_delay_s = 4.9;
    }

    let max_ticks = ((secs / FIXED_DT).ceil() as u64).max(1);
    if active_players >= 4 {
        for _ in 0..max_ticks {
            world.step_brains(home_brain, away_brain, FIXED_DT);
            let sh = world.match_state.score_home;
            let sa = world.match_state.score_away;
            if sh.abs_diff(sa) >= 7 && (sh == 0 || sa == 0) { break; }
        }
    } else {
        let mut benched_home = BenchedBrain { inner: home_brain, active: active_players, is_home: true };
        let mut benched_away = BenchedBrain { inner: away_brain, active: active_players, is_home: false };
        for _ in 0..max_ticks {
            world.step_brains(&mut benched_home, &mut benched_away, FIXED_DT);
            let sh = world.match_state.score_home;
            let sa = world.match_state.score_away;
            if sh.abs_diff(sa) >= 7 && (sh == 0 || sa == 0) { break; }
        }
    }

    let ms = &world.match_state;
    (ms.score_home, ms.score_away, ms.possession_s_home, ms.possession_s_away, ms.shots_home, ms.shots_away)
}

fn run_tournament_match(
    home_brain: &mut dyn TeamBrain,
    away_brain: &mut dyn TeamBrain,
    opening: TeamId,
    params: &SimParams,
) -> (u32, u32) {
    let phase_secs = phase_sim_secs();

    let (h1, a1, _, _, _, _) = run_phase(home_brain, away_brain, opening, params, phase_secs, 4);
    if h1 != a1 { return (h1, a1); }

    let (h2, a2, _, _, _, _) = run_phase(home_brain, away_brain, opening, params, phase_secs, 4);
    let th = h1 + h2;
    let ta = a1 + a2;
    if th != ta { return (th, ta); }

    eprintln!("    tie → 3v3");
    let (h3, a3, _, _, _, _) = run_phase(home_brain, away_brain, opening, params, phase_secs, 3);
    let th = th + h3; let ta = ta + a3;
    if th != ta { return (th, ta); }

    eprintln!("    tie → 2v2");
    let (h4, a4, _, _, _, _) = run_phase(home_brain, away_brain, opening, params, phase_secs, 2);
    let th = th + h4; let ta = ta + a4;
    if th != ta { return (th, ta); }

    eprintln!("    tie → 1v1");
    let (h5, a5, ph5, pa5, sh5, sa5) = run_phase(home_brain, away_brain, opening, params, phase_secs, 1);
    let th = th + h5; let ta = ta + a5;
    if th != ta { return (th, ta); }

    eprintln!("    1v1 tie → stats: poss {:.1} vs {:.1}, shots {} vs {}", ph5, pa5, sh5, sa5);
    if ph5 != pa5 { return (th + if ph5 > pa5 { 1 } else { 0 }, ta + if ph5 > pa5 { 0 } else { 1 }); }
    if sh5 != sa5 { return (th + if sh5 > sa5 { 1 } else { 0 }, ta + if sh5 > sa5 { 0 } else { 1 }); }
    (th, ta)
}

fn evaluate_candidate(
    cand: &mut Candidate,
    opponents: &[BrainInput],
    params: &SimParams,
    cache: &ProgramCache,
) {
    cand.fitness = 0.0;
    cand.goals_for = 0;
    cand.goals_against = 0;

    for opp in opponents {
        for opening in [TeamId::Home, TeamId::Away] {
            let mut home: Box<dyn TeamBrain> = if opening == TeamId::Home {
                Box::new(TrainedBrain::with_weights(cand.weights.clone()))
            } else {
                match build_brain(opp, GraphEngine::Runtime, Some(cache)) {
                    Ok(b) => b,
                    Err(e) => { eprintln!("  skip opp {}: {}", opp.label(), e); continue; }
                }
            };
            let mut away: Box<dyn TeamBrain> = if opening == TeamId::Away {
                Box::new(TrainedBrain::with_weights(cand.weights.clone()))
            } else {
                match build_brain(opp, GraphEngine::Runtime, Some(cache)) {
                    Ok(b) => b,
                    Err(e) => { eprintln!("  skip opp {}: {}", opp.label(), e); continue; }
                }
            };

            let (sh, sa) = run_tournament_match(&mut *home, &mut *away, opening, params);

            let (our_score, their_score) = if opening == TeamId::Home {
                (sh, sa)
            } else {
                (sa, sh)
            };

            cand.goals_for += our_score;
            cand.goals_against += their_score;
        }
    }

    cand.fitness = cand.goals_for as f32 - cand.goals_against as f32;
}

fn discover_opponents() -> Vec<BrainInput> {
    let saves = aicomp_soccer_sim::batch::soccer_saves_dir();
    let mut opponents = Vec::new();

    let real_scripts = [
        "AIA.txt", "AIA3.txt",
        "Haialand-v2.txt", "MOB_2.txt", "Nova_v56.txt",
        "Poponeta.txt", "StarCheese.txt",
        "Titanium.txt", "Titanium_release.txt",
        "Titanium_v1.txt", "Titanium_v2.txt", "Titanium_v3.txt",
        "Titanium_v4.txt", "Titanium_v5.txt", "Titanium_v6.txt",
        "Titanium_v7.txt",
        "ZudanFC3.2_release.txt", "ZudanFC3.5.txt",
    ];

    for name in &real_scripts {
        let path = saves.join(name);
        if path.is_file() && std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 1000 {
            opponents.push(BrainInput::Graph(path));
        }
    }

    opponents
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gen_count: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let pop_size: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let noise: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.1);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);

    eprintln!("=== ES Training (Tournament Format) ===");
    eprintln!("generations: {gen_count}, population: {pop_size}, noise: {noise}, seed: {seed}");
    eprintln!("format: 90min → 180min → 3v3 90min → 2v2 90min → 1v1 90min → stats (20x time scale, {} sim secs/phase)", phase_sim_secs());
    eprintln!("fitness = total goals scored - total goals conceded");

    let params = SimParams::default();
    let cache = ProgramCache::default();
    let opponents = discover_opponents();
    eprintln!("opponents found: {} ({} matches per candidate)",
        opponents.len(), opponents.len() * 2);
    for opp in &opponents {
        eprintln!("  - {}", opp.label());
    }

    let mut rng = StdRng::seed_from_u64(seed);

    let mut population: Vec<Candidate> = (0..pop_size)
        .map(|_| Candidate::new_random(&mut rng))
        .collect();

    let weights_path = "assets/es_weights.json";
    let best_path = "assets/es_best.json";

    for gen in 0..gen_count {
        let t0 = Instant::now();

        for (i, cand) in population.iter_mut().enumerate() {
            evaluate_candidate(cand, &opponents, &params, &cache);
            eprintln!("  gen{gen} cand{i}: GF{} GA{} fit={:.1}",
                cand.goals_for, cand.goals_against, cand.fitness);
        }

        population.sort_by(|a, b| b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal));

        let best = &population[0];
        let worst = &population[pop_size - 1];
        eprintln!(
            "gen {gen}: best fit={:.1} (GF{} GA{}) | worst fit={:.1} | {}ms",
            best.fitness, best.goals_for, best.goals_against,
            worst.fitness,
            t0.elapsed().as_millis()
        );

        best.save(best_path);

        if gen % 5 == 0 {
            best.save(&format!("assets/es_gen{gen}.json"));
        }

        let elite_count = (pop_size / 4).max(2);
        let mut next_gen: Vec<Candidate> = population[..elite_count]
            .iter()
            .map(|c| Candidate::from_weights(c.weights.clone()))
            .collect();

        while next_gen.len() < pop_size {
            let parent_idx = rng.gen_range(0..elite_count);
            let parent = &population[parent_idx];
            let child_weights = parent.weights.perturb(noise, &mut rng);
            next_gen.push(Candidate::from_weights(child_weights));
        }

        population = next_gen;
    }

    population.sort_by(|a, b| b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal));
    let champion = &population[0];
    eprintln!(
        "\n=== Final Champion ===\nfit={:.1} GF{} GA{}",
        champion.fitness, champion.goals_for, champion.goals_against
    );
    champion.save(weights_path);
    champion.save(best_path);
    eprintln!("saved champion to {weights_path}");
}
