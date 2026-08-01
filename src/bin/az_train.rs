use std::time::Instant;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::io::{Read, Write};
use std::net::TcpListener;

use aicomp_soccer_sim::api::ApiFieldMask;
use aicomp_soccer_sim::batch::{build_brain, BrainInput, GraphEngine, ProgramCache};
use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamBrain, TeamId};
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::train::{
    NetGradients, NetWeights,
    INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM,
    VALUES_PER_PLAYER, COMPASS_DIRS, SOFTMAX_GROUPS,
    SPRINT_OFFSET, TACKLE_OFFSET, TEAM_OUTPUT_START,
    extract_features,
};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::deterministic::{
    evaluate_and_apply_with_action, DeterministicCache, NNActions, ShootTarget,
};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand::seq::SliceRandom;
use rayon::prelude::*;

const NORM: f32 = 20.0;
const TIME_SCALE: f32 = 20.0;
const INGAME_MATCH_MIN: f32 = 90.0;
const MATCH_SECS: f32 = INGAME_MATCH_MIN * 60.0 / TIME_SCALE;
const MAX_TRAIN_STEPS: usize = 800;

// AZ hyperparameters
const POLICY_LR: f32 = 0.001;
const VALUE_LR: f32 = 0.002;
const DIRICHLET_ALPHA: f32 = 0.25;
const DIRICHLET_WEIGHT: f32 = 0.3;
const TEMPERATURE: f32 = 1.0;
const TD_LAMBDA: f32 = 0.95;
const GAMMA: f32 = 0.99;
const MAX_GRAD_NORM: f32 = 0.5;
const LR_DECAY: f32 = 0.995;
const RESTORE_THRESHOLD: f32 = 30.0;

// MCTS constants — disabled until pure-intuition training plateaus.
// Set MCTS_ENABLED=true to switch from pure policy to AlphaZero-style search.
const MCTS_ENABLED: bool = false;
const MCTS_CANDIDATES: usize = 4;    // Number of candidate actions to evaluate per state
const MCTS_ROLLOUT_TICKS: usize = 10; // Forward simulation ticks per candidate (~0.5s game time)
const MCTS_ITERATIONS: usize = 2;    // How many search rounds to run
const MCTS_C_PUCT: f32 = 1.0;       // Exploration constant for PUCT selection
const MCTS_INTERVAL: usize = 10;    // Run MCTS every N ticks (reuses last result in between)

/// Training sample: state + policy target (MCTS visit distribution) + value target.
#[derive(Clone)]
struct AzSample {
    input: [f32; INPUT_DIM],
    /// Policy target: softmax probabilities over OUTPUT_DIM (flattened).
    /// For softmax groups, this is the one-hot of the chosen action.
    /// For sigmoid outputs (sprint/tackle/shoot/pass), this is the sigmoid target.
    policy_target: [f32; OUTPUT_DIM],
    /// Value target: TD-lambda return from this state.
    value_target: f32,
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max_logit = logits.iter().cloned().fold(f32::MIN, f32::max);
    let mut exp_vals = Vec::with_capacity(logits.len());
    let mut sum_exp = 0.0;
    for l in logits {
        let e = (l - max_logit).exp();
        exp_vals.push(e);
        sum_exp += e;
    }
    for v in &mut exp_vals {
        *v /= sum_exp.max(1e-10);
    }
    exp_vals
}

fn softmax_sample(logits: &[f32], rng: &mut StdRng) -> usize {
    let probs = softmax(logits);
    let sample = rng.gen_range(0.0..1.0);
    let mut cum = 0.0;
    for (i, &p) in probs.iter().enumerate() {
        cum += p;
        if sample <= cum {
            return i;
        }
    }
    probs.len() - 1
}

fn add_dirichlet_noise(logits: &mut [f32], alpha: f32, weight: f32, rng: &mut StdRng) {
    let n = logits.len();
    let mut noise = Vec::with_capacity(n);
    let mut sum = 0.0;
    for _ in 0..n {
        let x = rng.gen::<f32>().max(1e-10).powf(1.0 / alpha);
        noise.push(x);
        sum += x;
    }
    for (l, nx) in logits.iter_mut().zip(&noise) {
        *l = (1.0 - weight) * *l + weight * (nx / sum.max(1e-10));
    }
}

/// Decode NN output into BrainOutput and NNActions, with softmax sampling.
fn output_to_commands(
    output: &[f32],
    api: &aicomp_soccer_sim::api::TeamApi,
    rng: &mut StdRng,
    add_noise: bool,
) -> (BrainOutput, NNActions) {
    use aicomp_soccer_sim::player::PlayerId;
    let mut out = BrainOutput::default();
    let mut players = [bevy::prelude::Vec2::ZERO; 4];
    for (i, id) in PlayerId::ALL.iter().enumerate() {
        players[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
    }

    let mut has_ball = [false; 4];
    for i in 0..4 {
        has_ball[i] = api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
    }

    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal_left = api.get_transform("Opponent Goal Left Post").unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal_right = api.get_transform("Opponent Goal Right Post").unwrap_or(bevy::prelude::Vec2::ZERO);

    for i in 0..4 {
        let base = i * VALUES_PER_PLAYER;

        // For each softmax group, sample and pick the best group.
        let mut best_dir = bevy::prelude::Vec2::ZERO;
        let mut best_logit = f32::MIN;

        for (grp_offset, grp_size) in SOFTMAX_GROUPS {
            let mut logits: Vec<f32> = Vec::with_capacity(*grp_size);
            for j in 0..*grp_size {
                logits.push(output[base + grp_offset + j]);
            }
            if add_noise {
                add_dirichlet_noise(&mut logits, DIRICHLET_ALPHA, DIRICHLET_WEIGHT, rng);
            }
            let chosen = softmax_sample(&logits, rng);
            let chosen_logit = logits[chosen];

            if chosen_logit > best_logit {
                best_logit = chosen_logit;
                best_dir = match *grp_offset {
                    0 => COMPASS_DIRS[chosen],
                    8 => {
                        let mate = api.get_transform(&format!("Team Player {}", chosen + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = mate - players[i];
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    12 => {
                        let mate = api.get_transform(&format!("Team Player {}", chosen + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = players[i] - mate;
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    16 => {
                        let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", chosen + 1)).flatten();
                        if let Some(d) = pd { d } else { bevy::prelude::Vec2::ZERO }
                    }
                    20 => {
                        let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", chosen + 1)).flatten();
                        if let Some(d) = pd { d } else { bevy::prelude::Vec2::ZERO }
                    }
                    24 => {
                        let opp = api.get_transform(&format!("Opponent Player {}", chosen + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = opp - players[i];
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    28 => {
                        let opp = api.get_transform(&format!("Opponent Player {}", chosen + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = players[i] - opp;
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    32 => {
                        match chosen {
                            0 => { let d = opp_goal_left - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                            1 => { let d = opp_goal - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                            _ => { let d = opp_goal_right - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                        }
                    }
                    _ => bevy::prelude::Vec2::ZERO,
                };
            }
        }

        let move_to = players[i] + best_dir * NORM;
        let sprint_sig = output[base + SPRINT_OFFSET];
        let tackle_sig = output[base + TACKLE_OFFSET];

        let interact = if has_ball[i] { true } else { tackle_sig > 0.0 };

        out.commands[i] = BrainCommand {
            move_to,
            sprint: sprint_sig > 0.0,
            interact,
            shoot: false,
        };
    }

    // Team-level
    let shoot_signal = output[TEAM_OUTPUT_START];
    let pass_signal = output[TEAM_OUTPUT_START + 4];

    let mut shoot_target = ShootTarget::Center;
    let mut shoot = false;
    if shoot_signal > 0.0 {
        shoot = true;
        let mut logits = [0.0f32; 3];
        for a in 0..3 {
            logits[a] = output[TEAM_OUTPUT_START + 1 + a];
        }
        let best = if add_noise { softmax_sample(&logits, rng) } else {
            logits.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(i, _)| i).unwrap_or(1)
        };
        shoot_target = match best {
            0 => ShootTarget::Left,
            1 => ShootTarget::Center,
            _ => ShootTarget::Right,
        };
    }

    let mut pass_target = 0usize;
    let mut pass = false;
    if pass_signal > 0.0 && !shoot {
        pass = true;
        let mut logits = [0.0f32; 4];
        for a in 0..4 {
            logits[a] = output[TEAM_OUTPUT_START + 5 + a];
        }
        pass_target = if add_noise { softmax_sample(&logits, rng) } else {
            logits.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(i, _)| i).unwrap_or(0)
        };
    }

    if shoot {
        for i in 0..4 {
            if has_ball[i] { out.commands[i].shoot = true; }
        }
    }
    let actions = NNActions { shoot, shoot_target, pass, pass_target };
    (out, actions)
}

/// Build policy target from the output that was actually used.
/// For softmax groups: one-hot at the chosen index.
/// For sigmoid outputs: the raw sigmoid value.
fn build_policy_target(
    output: &[f32],
    api: &aicomp_soccer_sim::api::TeamApi,
    rng: &mut StdRng,
) -> [f32; OUTPUT_DIM] {
    let mut target = [0.0f32; OUTPUT_DIM];

    for i in 0..4 {
        let base = i * VALUES_PER_PLAYER;
        for (grp_offset, grp_size) in SOFTMAX_GROUPS {
            let mut logits: Vec<f32> = Vec::with_capacity(*grp_size);
            for j in 0..*grp_size {
                logits.push(output[base + grp_offset + j]);
            }
            let chosen = softmax_sample(&logits, rng);
            target[base + grp_offset + chosen] = 1.0;
        }
        // Sprint and tackle: use sigmoid of the output as target.
        target[base + SPRINT_OFFSET] = 1.0 / (1.0 + (-output[base + SPRINT_OFFSET]).exp());
        target[base + TACKLE_OFFSET] = 1.0 / (1.0 + (-output[base + TACKLE_OFFSET]).exp());
    }

    // Team-level
    target[TEAM_OUTPUT_START] = 1.0 / (1.0 + (-output[TEAM_OUTPUT_START]).exp());
    for a in 0..3 {
        target[TEAM_OUTPUT_START + 1 + a] = 1.0 / (1.0 + (-output[TEAM_OUTPUT_START + 1 + a]).exp());
    }
    target[TEAM_OUTPUT_START + 4] = 1.0 / (1.0 + (-output[TEAM_OUTPUT_START + 4]).exp());
    for a in 0..4 {
        target[TEAM_OUTPUT_START + 5 + a] = 1.0 / (1.0 + (-output[TEAM_OUTPUT_START + 5 + a]).exp());
    }

    target
}

/// Auto-tackle macro (same as ppo_train).
fn auto_tackle(
    nn_api: &aicomp_soccer_sim::api::TeamApi,
    nn_out: &mut BrainOutput,
    params: &SimParams,
) {
    let opp_has_ball = nn_api.get_bool("Opponent Has Ball").unwrap_or(false);
    if !opp_has_ball { return; }

    let interact_r = params.interact_radius;
    let carrier_stam = nn_api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;
    let mut carrier_pos = bevy::prelude::Vec2::ZERO;
    for i in 0..4u8 {
        if nn_api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) {
            carrier_pos = nn_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
            break;
        }
    }
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    for i in 0..4 {
        let p = nn_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
        let dist = (p - carrier_pos).length();
        if dist <= interact_r {
            let stam = nn_api.get_float(&format!("Team Player {} Stamina", i + 1)).unwrap_or(0.0) / 100.0;
            candidates.push((i, stam));
        }
    }
    if candidates.is_empty() { return; }

    let chosen = candidates.iter()
        .filter(|(_, s)| *s >= carrier_stam - 1e-4)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .or_else(|| candidates.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)))
        .map(|(i, _)| *i)
        .unwrap_or(0);

    for (i, _) in &candidates {
        nn_out.commands[*i].interact = *i == chosen;
    }
}

/// A candidate action in the MCTS search.
struct MctsCandidate {
    /// The NN output (logits) for this candidate.
    output: Vec<f32>,
    /// The BrainOutput to apply.
    brain_out: BrainOutput,
    /// The NNActions for deterministic layer.
    actions: NNActions,
    /// Policy prior probability (from softmax of the output).
    prior: f32,
    /// Number of visits during search.
    visits: usize,
    /// Total accumulated value from rollouts.
    total_value: f32,
}

impl MctsCandidate {
    fn q(&self) -> f32 {
        if self.visits == 0 { 0.0 } else { self.total_value / self.visits as f32 }
    }
    fn uct(&self, total_visits: usize) -> f32 {
        let exploration = MCTS_C_PUCT * self.prior * ((total_visits as f32).sqrt() / (1 + self.visits) as f32);
        self.q() + exploration
    }
}

/// Run MCTS search at the current state.
/// Returns (best_output, improved_policy_target, value_estimate).
fn mcts_search(
    weights: &NetWeights,
    world: &MatchWorld,
    nn_api: &aicomp_soccer_sim::api::TeamApi,
    params: &SimParams,
    rng: &mut StdRng,
    is_home_nn: bool,
    opp_vel: &[bevy::prelude::Vec2; 4],
    home_mask: Option<&ApiFieldMask>,
    away_mask: Option<&ApiFieldMask>,
) -> (Vec<f32>, [f32; OUTPUT_DIM], f32) {
    let (input, _is_home) = extract_features(nn_api, opp_vel);
    let act = weights.forward_with_activations(&input);

    // Generate K candidate actions by sampling from the policy with different random seeds.
    let mut candidates: Vec<MctsCandidate> = Vec::with_capacity(MCTS_CANDIDATES);
    let mut prior_sum = 0.0;
    for _ in 0..MCTS_CANDIDATES {
        let (brain_out, actions) = output_to_commands(&act.output, nn_api, rng, true);
        let probs = softmax(&act.output);
        // Prior = average probability of the chosen actions (simplified: use output entropy)
        let prior = 1.0 / MCTS_CANDIDATES as f32; // Uniform prior initially, refined by visits
        candidates.push(MctsCandidate {
            output: act.output.clone(),
            brain_out,
            actions,
            prior,
            visits: 0,
            total_value: 0.0,
        });
        prior_sum += prior;
    }
    // Normalize priors
    for c in &mut candidates {
        c.prior /= prior_sum.max(1e-10);
    }

    // Run MCTS iterations
    for _iteration in 0..MCTS_ITERATIONS {
        let total_visits: usize = candidates.iter().map(|c| c.visits).sum();

        // Selection: pick candidate with highest UCT score
        let best_idx = candidates.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.uct(total_visits).partial_cmp(&b.uct(total_visits)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);

        // Expansion + Rollout: clone world, apply candidate's action, simulate forward
        let mut sim_world = world.clone();
        let mut sim_det_cache = DeterministicCache::default();
        let mut sim_macro_tick = 0u32;
        let mut sim_was_loose = false;
        let mut sim_grab_hold = false;

        let mut nn_out = candidates[best_idx].brain_out.clone();
        let actions = candidates[best_idx].actions.clone();

        // Apply deterministic layer on the first tick
        evaluate_and_apply_with_action(nn_api, params, &mut nn_out, &mut sim_det_cache, &actions);
        auto_tackle(nn_api, &mut nn_out, params);

        let mut rollout_value = 0.0f32;
        let mut goal_delta = 0.0f32;

        let (us_before, them_before) = if is_home_nn {
            (sim_world.match_state.score_home, sim_world.match_state.score_away)
        } else {
            (sim_world.match_state.score_away, sim_world.match_state.score_home)
        };

        for tick in 0..MCTS_ROLLOUT_TICKS {
            // Build APIs for this sim state
            let (home_api, away_api) = sim_world.build_apis_masked(home_mask, away_mask);
            let (sim_nn_api, sim_opp_api) = if is_home_nn {
                (&home_api, &away_api)
            } else {
                (&away_api, &home_api)
            };

            // After first tick, use the NN to generate actions for continued simulation
            let (cur_out, _cur_actions) = if tick == 0 {
                (nn_out.clone(), actions.clone())
            } else {
                let mut sim_opp_vel = [bevy::prelude::Vec2::ZERO; 4];
                for i in 0..4 {
                    let opp_pos = sim_nn_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                    sim_opp_vel[i] = (opp_pos - opp_vel[i]) / FIXED_DT;
                }
                let (sim_input, _) = extract_features(sim_nn_api, &sim_opp_vel);
                let sim_act = weights.forward_with_activations(&sim_input);
                let (mut so, sa) = output_to_commands(&sim_act.output, sim_nn_api, rng, false);
                evaluate_and_apply_with_action(sim_nn_api, params, &mut so, &mut sim_det_cache, &sa);
                auto_tackle(sim_nn_api, &mut so, params);

                // Auto-pickup
                let is_loose = sim_nn_api.get_bool("Is Ball Loose").unwrap_or(false);
                sim_macro_tick += 1;
                if is_loose {
                    let interact_r = sim_nn_api.get_float("Player Interact Radius").unwrap_or(1.5);
                    let ball = sim_nn_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
                    let mut best_p = None;
                    let mut best_d = f32::MAX;
                    for i in 0..4 {
                        let has = sim_nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                        if has { continue; }
                        let p = sim_nn_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = (p - ball).length();
                        if d < best_d { best_d = d; best_p = Some(i); }
                    }
                    if let Some(idx) = best_p {
                        if best_d <= interact_r { so.commands[idx].interact = sim_macro_tick % 2 == 1; }
                    }
                }
                if sim_grab_hold {
                    sim_grab_hold = false;
                    for i in 0..4 {
                        let has = sim_nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                        if has { so.commands[i].interact = false; break; }
                    }
                }
                if sim_was_loose && !is_loose { sim_grab_hold = true; }
                sim_was_loose = is_loose;

                (so, sa)
            };

            // Opponent: also use NN to control (self-play — NN plays both sides)
            let opp_opp_vel = [bevy::prelude::Vec2::ZERO; 4];
            let (opp_input, _) = extract_features(sim_opp_api, &opp_opp_vel);
            let opp_act = weights.forward_with_activations(&opp_input);
            let (mut opp_out, opp_actions) = output_to_commands(&opp_act.output, sim_opp_api, rng, false);
            evaluate_and_apply_with_action(sim_opp_api, params, &mut opp_out, &mut sim_det_cache, &opp_actions);
            auto_tackle(sim_opp_api, &mut opp_out, params);

            let (home_out, away_out) = if is_home_nn {
                (&cur_out, &opp_out)
            } else {
                (&opp_out, &cur_out)
            };
            sim_world.step_with_commands(home_out, &away_out, FIXED_DT);

            // Check for goals
            let (us_now, them_now) = if is_home_nn {
                (sim_world.match_state.score_home, sim_world.match_state.score_away)
            } else {
                (sim_world.match_state.score_away, sim_world.match_state.score_home)
            };
            if us_now > us_before as u32 + goal_delta as u32 {
                goal_delta += 1.0;
            }
            if them_now > them_before as u32 + (goal_delta.abs() as u32) {
                goal_delta -= 1.0;
            }
        }

        // Evaluate leaf with value head
        let (home_api, away_api) = sim_world.build_apis_masked(home_mask, away_mask);
        let (leaf_api, _) = if is_home_nn { (&home_api, &away_api) } else { (&away_api, &home_api) };
        let mut leaf_opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        for i in 0..4 {
            leaf_opp_vel[i] = leaf_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
        }
        let (leaf_input, _) = extract_features(leaf_api, &leaf_opp_vel);
        let leaf_act = weights.forward_with_activations(&leaf_input);
        let leaf_value = leaf_act.value;

        // Combine rollout goal delta with value head estimate
        let combined_value = goal_delta * 0.5 + leaf_value * 0.5;

        // Backpropagation
        candidates[best_idx].visits += 1;
        candidates[best_idx].total_value += combined_value;
    }

    // Build improved policy from visit distribution
    let total_visits: f32 = candidates.iter().map(|c| c.visits as f32).sum::<f32>().max(1.0);
    let best_idx = candidates.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.visits.cmp(&b.visits))
        .map(|(i, _)| i)
        .unwrap_or(0);

    let best_output = candidates[best_idx].output.clone();
    let improved_policy = build_policy_target(&best_output, nn_api, rng);
    let value_estimate = candidates[best_idx].q();

    (best_output, improved_policy, value_estimate)
}

/// Convert an expert BrainOutput + API state into a policy target for supervised learning.
/// Reverse-engineers which softmax group was chosen based on the move_to direction.
fn expert_to_policy_target(
    expert_out: &BrainOutput,
    api: &aicomp_soccer_sim::api::TeamApi,
) -> [f32; OUTPUT_DIM] {
    use aicomp_soccer_sim::player::PlayerId;
    let mut target = [0.0f32; OUTPUT_DIM];

    let mut players = [bevy::prelude::Vec2::ZERO; 4];
    for (i, id) in PlayerId::ALL.iter().enumerate() {
        players[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
    }

    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal_left = api.get_transform("Opponent Goal Left Post").unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal_right = api.get_transform("Opponent Goal Right Post").unwrap_or(bevy::prelude::Vec2::ZERO);

    for i in 0..4 {
        let base = i * VALUES_PER_PLAYER;
        let cmd = &expert_out.commands[i];
        let desired_dir = if (cmd.move_to - players[i]).length() > 1e-3 {
            (cmd.move_to - players[i]).normalize()
        } else {
            bevy::prelude::Vec2::ZERO
        };

        // For each softmax group, find which direction best matches desired_dir
        for (grp_offset, grp_size) in SOFTMAX_GROUPS {
            let mut best_idx = 0;
            let mut best_dot = f32::MIN;

            for j in 0..*grp_size {
                let dir = match *grp_offset {
                    0 => COMPASS_DIRS[j],
                    8 => {
                        let mate = api.get_transform(&format!("Team Player {}", j + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = mate - players[i];
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    12 => {
                        let mate = api.get_transform(&format!("Team Player {}", j + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = players[i] - mate;
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    16 | 20 => {
                        let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", j + 1)).flatten();
                        if let Some(d) = pd { d } else { bevy::prelude::Vec2::ZERO }
                    }
                    24 => {
                        let opp = api.get_transform(&format!("Opponent Player {}", j + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = opp - players[i];
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    28 => {
                        let opp = api.get_transform(&format!("Opponent Player {}", j + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                        let d = players[i] - opp;
                        if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO }
                    }
                    32 => {
                        match j {
                            0 => { let d = opp_goal_left - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                            1 => { let d = opp_goal - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                            _ => { let d = opp_goal_right - players[i]; if d.length() > 1e-6 { d.normalize() } else { bevy::prelude::Vec2::ZERO } }
                        }
                    }
                    _ => bevy::prelude::Vec2::ZERO,
                };

                let dot = dir.x * desired_dir.x + dir.y * desired_dir.y;
                if dot > best_dot {
                    best_dot = dot;
                    best_idx = j;
                }
            }
            target[base + grp_offset + best_idx] = 1.0;
        }

        // Sprint and tackle: use the actual command values
        target[base + SPRINT_OFFSET] = if cmd.sprint { 1.0 } else { 0.0 };
        target[base + TACKLE_OFFSET] = if cmd.interact && !api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false) { 1.0 } else { 0.0 };
    }

    // Team-level: shoot and pass — we can't easily reverse-engineer these from BrainOutput
    // since shoot/pass are encoded as signals + softmax targets. Set them to 0 (no shot/pass)
    // unless the expert shot or passed (which we can detect from the deterministic layer).
    // For supervised learning, we'll use 0.5 (neutral) for these since we don't have the info.
    target[TEAM_OUTPUT_START] = 0.5; // shoot signal — neutral
    for a in 0..3 { target[TEAM_OUTPUT_START + 1 + a] = 0.33; } // shoot direction — uniform
    target[TEAM_OUTPUT_START + 4] = 0.5; // pass signal — neutral
    for a in 0..4 { target[TEAM_OUTPUT_START + 5 + a] = 0.25; } // pass target — uniform

    target
}

/// Run an expert vs expert match and collect supervised training samples.
/// Records states from the "student" perspective (the side we want to learn from).
fn run_expert_match(
    expert_a: &mut dyn TeamBrain,
    expert_b: &mut dyn TeamBrain,
    params: &SimParams,
    rng: &mut StdRng,
    kickoff: TeamId,
    home_mask: Option<&ApiFieldMask>,
    away_mask: Option<&ApiFieldMask>,
) -> Vec<AzSample> {
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), kickoff);
    let mut samples: Vec<AzSample> = Vec::new();
    let max_ticks = (MATCH_SECS / FIXED_DT) as usize;

    // Track which side we're learning from (alternate each match)
    let learn_home = rng.gen_bool(0.5);

    for _tick in 0..max_ticks {
        let (home_api, away_api) = world.build_apis();
        let (learn_api, other_api) = if learn_home {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        // Get expert decisions
        let home_out = expert_a.think(&home_api);
        let away_out = expert_b.think(&away_api);

        // Record sample from the learning side's perspective
        let mut opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        for i in 0..4 {
            let opp_pos = learn_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
            opp_vel[i] = opp_pos / FIXED_DT; // approximate
        }
        let (input, _) = extract_features(learn_api, &opp_vel);

        let expert_out = if learn_home { &home_out } else { &away_out };
        let policy_target = expert_to_policy_target(expert_out, learn_api);

        // Value target: goal difference at end of match (simplified)
        let (us, them) = if learn_home {
            (world.match_state.score_home, world.match_state.score_away)
        } else {
            (world.match_state.score_away, world.match_state.score_home)
        };
        let value_target = (us as f32 - them as f32) / 7.0; // normalize to [-1, 1]

        samples.push(AzSample {
            input,
            policy_target,
            value_target,
        });

        world.step_with_commands(&home_out, &away_out, FIXED_DT);

        let sh = world.match_state.score_home;
        let sa = world.match_state.score_away;
        if sh.abs_diff(sa) >= 7 && (sh == 0 || sa == 0) {
            break;
        }
    }

    // Update value targets with TD-lambda from the end of the match
    let n = samples.len();
    if n > 0 {
        let final_val = {
            let (us, them) = if learn_home {
                (world.match_state.score_home, world.match_state.score_away)
            } else {
                (world.match_state.score_away, world.match_state.score_home)
            };
            (us as f32 - them as f32) / 7.0
        };

        // Simple TD-lambda: propagate final value backwards with decay
        let mut running = final_val;
        for i in (0..n).rev() {
            running = running * GAMMA + final_val * (1.0 - GAMMA);
            samples[i].value_target = running;
        }
    }

    samples
}

/// Run a self-play match and collect training samples.
fn run_self_play_match(
    weights: &NetWeights,
    params: &SimParams,
    rng: &mut StdRng,
    kickoff: TeamId,
    home_mask: Option<&ApiFieldMask>,
    away_mask: Option<&ApiFieldMask>,
) -> (Vec<AzSample>, u32, u32) {
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), kickoff);
    let mut samples: Vec<AzSample> = Vec::new();

    // Opponent velocity tracking
    let mut prev_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_valid = false;

    let mut det_cache = DeterministicCache::default();

    // Macro state for auto-pickup and grab-hold release (NN side)
    let mut macro_tick = 0u32;
    let mut was_loose = false;
    let mut grab_hold_release_tick = false;

    // Macro state for opponent side (NN controls both teams)
    let mut opp_macro_tick = 0u32;
    let mut opp_was_loose = false;
    let mut opp_grab_hold = false;
    let mut opp_det_cache = DeterministicCache::default();

    let max_ticks = (MATCH_SECS / FIXED_DT) as usize;

    let is_home_nn = rng.gen_bool(0.5);

    // MCTS result cache (reused between MCTS ticks)
    let mut mcts_cached_output: Option<Vec<f32>> = None;
    let mut mcts_cached_policy: Option<[f32; OUTPUT_DIM]> = None;

    for _tick in 0..max_ticks {
        let (home_api, away_api) = world.build_apis_masked(home_mask, away_mask);
        let (nn_api, opp_api) = if is_home_nn {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        // Compute opponent velocity
        let mut opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        for i in 0..4 {
            let opp_pos = nn_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
            if prev_opp_pos_valid {
                opp_vel[i] = (opp_pos - prev_opp_pos[i]) / FIXED_DT;
            }
            prev_opp_pos[i] = opp_pos;
        }
        prev_opp_pos_valid = true;

        let (input, _is_home) = extract_features(nn_api, &opp_vel);

        // Forward pass — use MCTS search if enabled (every N ticks), otherwise pure intuition
        let mcts_tick = MCTS_ENABLED && (_tick % MCTS_INTERVAL == 0);

        let (nn_output, policy_target, _mcts_value) = if mcts_tick {
            // AlphaZero-style: run MCTS to get improved policy + value
            let (best_output, improved_policy, value_est) = mcts_search(
                weights, &world, nn_api, params, rng, is_home_nn, &opp_vel,
                home_mask, away_mask,
            );
            // Cache the MCTS output for reuse between MCTS ticks
            mcts_cached_output = Some(best_output);
            mcts_cached_policy = Some(improved_policy);
            (mcts_cached_output.clone().unwrap(), mcts_cached_policy.clone().unwrap(), value_est)
        } else if MCTS_ENABLED && mcts_cached_output.is_some() {
            // Reuse last MCTS result
            (mcts_cached_output.clone().unwrap(), mcts_cached_policy.clone().unwrap(), 0.0)
        } else {
            // Pure intuition: just forward pass + Dirichlet noise
            let act = weights.forward_with_activations(&input);
            let pt = build_policy_target(&act.output, nn_api, rng);
            (act.output, pt, 0.0)
        };

        // Decode with exploration noise
        let (mut nn_out, actions) = output_to_commands(&nn_output, nn_api, rng, true);

        // Apply deterministic layer
        evaluate_and_apply_with_action(nn_api, params, &mut nn_out, &mut det_cache, &actions);

        // Auto-tackle
        auto_tackle(nn_api, &mut nn_out, params);

        // Auto-pickup macro: if ball is loose, force interact on closest NN player
        // within pickup range. Pulses every other tick for rising edge.
        let is_loose = nn_api.get_bool("Is Ball Loose").unwrap_or(false);
        macro_tick += 1;
        if is_loose {
            let interact_r = nn_api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = nn_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
            let mut best_idx = None;
            let mut best_dist = f32::MAX;
            for i in 0..4 {
                let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has { continue; }
                let p = nn_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                let d = (p - ball).length();
                if d < best_dist {
                    best_dist = d;
                    best_idx = Some(i);
                }
            }
            if let Some(idx) = best_idx {
                if best_dist <= interact_r {
                    nn_out.commands[idx].interact = macro_tick % 2 == 1;
                }
            }
        }

        // Grab-hold release: after pickup, force interact=false for one tick
        if grab_hold_release_tick {
            grab_hold_release_tick = false;
            for i in 0..4 {
                let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has {
                    nn_out.commands[i].interact = false;
                    break;
                }
            }
        }
        if was_loose && !is_loose {
            grab_hold_release_tick = true;
        }
        was_loose = is_loose;

        // Opponent: also NN-controlled (true self-play — NN plays both sides)
        let mut opp_opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        for i in 0..4 {
            let opp_opp_pos = opp_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
            opp_opp_vel[i] = (opp_opp_pos - prev_opp_pos[i]) / FIXED_DT;
        }
        let (opp_input, _) = extract_features(opp_api, &opp_opp_vel);
        let opp_act = weights.forward_with_activations(&opp_input);
        let (mut opp_out, opp_actions) = output_to_commands(&opp_act.output, opp_api, rng, true);
        evaluate_and_apply_with_action(opp_api, params, &mut opp_out, &mut opp_det_cache, &opp_actions);
        auto_tackle(opp_api, &mut opp_out, params);

        // Opponent auto-pickup
        let opp_is_loose = opp_api.get_bool("Is Ball Loose").unwrap_or(false);
        opp_macro_tick += 1;
        if opp_is_loose {
            let interact_r = opp_api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = opp_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
            let mut best_p = None;
            let mut best_d = f32::MAX;
            for i in 0..4 {
                let has = opp_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has { continue; }
                let p = opp_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                let d = (p - ball).length();
                if d < best_d { best_d = d; best_p = Some(i); }
            }
            if let Some(idx) = best_p {
                if best_d <= interact_r { opp_out.commands[idx].interact = opp_macro_tick % 2 == 1; }
            }
        }
        if opp_grab_hold {
            opp_grab_hold = false;
            for i in 0..4 {
                let has = opp_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has { opp_out.commands[i].interact = false; break; }
            }
        }
        if opp_was_loose && !opp_is_loose { opp_grab_hold = true; }
        opp_was_loose = opp_is_loose;

        let (home_out, away_out) = if is_home_nn {
            (&nn_out, &opp_out)
        } else {
            (&opp_out, &nn_out)
        };

        // Record sample BEFORE stepping
        let sh_before = world.match_state.score_home;
        let sa_before = world.match_state.score_away;
        let (cur_us_before, cur_them_before) = if is_home_nn {
            (sh_before, sa_before)
        } else {
            (sa_before, sh_before)
        };

        world.step_with_commands(home_out, away_out, FIXED_DT);

        let sh_after = world.match_state.score_home;
        let sa_after = world.match_state.score_away;
        let (cur_us, cur_them) = if is_home_nn {
            (sh_after, sa_after)
        } else {
            (sa_after, sh_after)
        };

        // Compute reward for this step
        let goal_reward = if cur_us > cur_us_before { 1.0 } else if cur_them > cur_them_before { -1.0 } else { 0.0 };

        // Store sample with placeholder value (will be filled with TD-lambda later)
        samples.push(AzSample {
            input,
            policy_target,
            value_target: goal_reward, // temporary, will be replaced
        });

        // Early termination
        if sh_after.abs_diff(sa_after) >= 7 && (sh_after == 0 || sa_after == 0) {
            break;
        }
    }

    // Compute TD-lambda returns for value targets
    let n = samples.len();
    if n > 0 {
        let final_us = if is_home_nn { world.match_state.score_home } else { world.match_state.score_away };
        let final_them = if is_home_nn { world.match_state.score_away } else { world.match_state.score_home };
        let goal_diff = (final_us as f32) - (final_them as f32);
        let terminal_value = goal_diff.tanh();

        // TD-lambda: V_t = r_t + gamma * (lambda * V_{t+1} + (1-lambda) * V_{t+2} + ...)
        // Simplified: use exponential moving average of future rewards + terminal value
        let mut running_value = terminal_value;
        for i in (0..n).rev() {
            let r = samples[i].value_target; // currently holds the step reward
            running_value = r + GAMMA * TD_LAMBDA * running_value;
            samples[i].value_target = running_value.tanh();
        }
    }

    // Subsample if too many steps
    if samples.len() > MAX_TRAIN_STEPS {
        let stride = samples.len() / MAX_TRAIN_STEPS;
        samples = samples.iter().step_by(stride).take(MAX_TRAIN_STEPS).cloned().collect();
    }

    let (final_us, final_them) = if is_home_nn {
        (world.match_state.score_home, world.match_state.score_away)
    } else {
        (world.match_state.score_away, world.match_state.score_home)
    };

    (samples, final_us, final_them)
}

/// Compute policy cross-entropy + value MSE gradient for a single sample.
fn compute_sample_gradient(
    weights: &NetWeights,
    sample: &AzSample,
) -> NetGradients {
    let act = weights.forward_with_activations(&sample.input);

    // Policy gradient: cross-entropy loss for softmax groups + BCE for sigmoid outputs.
    // For softmax: dL/dlogit_i = softmax_i - target_i
    // For sigmoid (sprint/tackle/shoot/pass): dL/dout = sigmoid(out) - target
    let mut grad_out = [0.0f32; OUTPUT_DIM];

    for i in 0..4 {
        let base = i * VALUES_PER_PLAYER;
        for (grp_offset, grp_size) in SOFTMAX_GROUPS {
            let logits: Vec<f32> = (0..*grp_size).map(|j| act.output[base + grp_offset + j]).collect();
            let probs = softmax(&logits);
            for j in 0..*grp_size {
                grad_out[base + grp_offset + j] = probs[j] - sample.policy_target[base + grp_offset + j];
            }
        }
        // Sprint and tackle: BCE gradient
        let sprint_out = 1.0 / (1.0 + (-act.output[base + SPRINT_OFFSET]).exp());
        grad_out[base + SPRINT_OFFSET] = sprint_out - sample.policy_target[base + SPRINT_OFFSET];
        let tackle_out = 1.0 / (1.0 + (-act.output[base + TACKLE_OFFSET]).exp());
        grad_out[base + TACKLE_OFFSET] = tackle_out - sample.policy_target[base + TACKLE_OFFSET];
    }

    // Team-level
    let shoot_out = 1.0 / (1.0 + (-act.output[TEAM_OUTPUT_START]).exp());
    grad_out[TEAM_OUTPUT_START] = shoot_out - sample.policy_target[TEAM_OUTPUT_START];
    for a in 0..3 {
        let o = 1.0 / (1.0 + (-act.output[TEAM_OUTPUT_START + 1 + a]).exp());
        grad_out[TEAM_OUTPUT_START + 1 + a] = o - sample.policy_target[TEAM_OUTPUT_START + 1 + a];
    }
    let pass_out = 1.0 / (1.0 + (-act.output[TEAM_OUTPUT_START + 4]).exp());
    grad_out[TEAM_OUTPUT_START + 4] = pass_out - sample.policy_target[TEAM_OUTPUT_START + 4];
    for a in 0..4 {
        let o = 1.0 / (1.0 + (-act.output[TEAM_OUTPUT_START + 5 + a]).exp());
        grad_out[TEAM_OUTPUT_START + 5 + a] = o - sample.policy_target[TEAM_OUTPUT_START + 5 + a];
    }

    // Value gradient: MSE loss = 0.5 * (value - target)^2
    // dL/dvalue = (value - target)
    let grad_value = act.value - sample.value_target;

    weights.backward(&grad_out, grad_value, &act)
}

/// Supervised training step: average gradient over all samples (parallelized).
fn az_train_step(
    weights: &mut NetWeights,
    samples: &[AzSample],
    policy_lr: f32,
    value_lr: f32,
) {
    let n = samples.len();
    if n == 0 { return; }

    // Parallel gradient computation: each thread computes gradients for a chunk of samples.
    let weights_ref = &*weights;
    let partial_grads: Vec<NetGradients> = samples
        .par_chunks(256)
        .map(|chunk| {
            let mut chunk_grad = NetGradients::zeros(weights_ref);
            for sample in chunk {
                let step_grad = compute_sample_gradient(weights_ref, sample);
                chunk_grad.add(&step_grad, 1.0 / n as f32);
            }
            chunk_grad
        })
        .collect();

    // Sum partial gradients.
    let mut total_grad = NetGradients::zeros(weights);
    for pg in &partial_grads {
        total_grad.add(pg, 1.0);
    }

    // Gradient clipping
    let mut grad_norm = 0.0f32;
    for r in &total_grad.layer0.w { for v in r { grad_norm += v * v; } }
    for v in &total_grad.layer0.b { grad_norm += v * v; }
    for r in &total_grad.layer1.w { for v in r { grad_norm += v * v; } }
    for v in &total_grad.layer1.b { grad_norm += v * v; }
    for r in &total_grad.layer2.w { for v in r { grad_norm += v * v; } }
    for v in &total_grad.layer2.b { grad_norm += v * v; }
    for r in &total_grad.value_head.w { for v in r { grad_norm += v * v; } }
    for v in &total_grad.value_head.b { grad_norm += v * v; }
    grad_norm = grad_norm.sqrt().max(1e-6);
    if grad_norm > MAX_GRAD_NORM {
        let scale = MAX_GRAD_NORM / grad_norm;
        for r in &mut total_grad.layer0.w { for v in r.iter_mut() { *v *= scale; } }
        for v in &mut total_grad.layer0.b { *v *= scale; }
        for r in &mut total_grad.layer1.w { for v in r.iter_mut() { *v *= scale; } }
        for v in &mut total_grad.layer1.b { *v *= scale; }
        for r in &mut total_grad.layer2.w { for v in r.iter_mut() { *v *= scale; } }
        for v in &mut total_grad.layer2.b { *v *= scale; }
        for r in &mut total_grad.value_head.w { for v in r.iter_mut() { *v *= scale; } }
        for v in &mut total_grad.value_head.b { *v *= scale; }
    }

    // Apply with separate learning rates for policy (layer0/1/2) and value (value_head)
    for (r, gr) in weights.layer0.w.iter_mut().zip(&total_grad.layer0.w) {
        for (v, gv) in r.iter_mut().zip(gr) { *v -= policy_lr * gv; }
    }
    for (v, gv) in weights.layer0.b.iter_mut().zip(&total_grad.layer0.b) { *v -= policy_lr * gv; }
    for (r, gr) in weights.layer1.w.iter_mut().zip(&total_grad.layer1.w) {
        for (v, gv) in r.iter_mut().zip(gr) { *v -= policy_lr * gv; }
    }
    for (v, gv) in weights.layer1.b.iter_mut().zip(&total_grad.layer1.b) { *v -= policy_lr * gv; }
    for (r, gr) in weights.layer2.w.iter_mut().zip(&total_grad.layer2.w) {
        for (v, gv) in r.iter_mut().zip(gr) { *v -= policy_lr * gv; }
    }
    for (v, gv) in weights.layer2.b.iter_mut().zip(&total_grad.layer2.b) { *v -= policy_lr * gv; }
    for (r, gr) in weights.value_head.w.iter_mut().zip(&total_grad.value_head.w) {
        for (v, gv) in r.iter_mut().zip(gr) { *v -= value_lr * gv; }
    }
    for (v, gv) in weights.value_head.b.iter_mut().zip(&total_grad.value_head.b) { *v -= value_lr * gv; }
}

/// Evaluate against an opponent.
fn evaluate_match(
    weights: &NetWeights,
    opp_brain: &mut dyn TeamBrain,
    nn_side: TeamId,
    kickoff: TeamId,
    params: &SimParams,
    rng: &mut StdRng,
    home_mask: Option<&ApiFieldMask>,
    away_mask: Option<&ApiFieldMask>,
) -> (u32, u32) {
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), kickoff);
    let mut det_cache = DeterministicCache::default();
    let mut prev_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_valid = false;

    // Macro state for auto-pickup and grab-hold release
    let mut macro_tick = 0u32;
    let mut was_loose = false;
    let mut grab_hold_release_tick = false;

    let is_home_nn = nn_side == TeamId::Home;
    let max_ticks = (MATCH_SECS / FIXED_DT) as usize;

    for _tick in 0..max_ticks {
        // Build full APIs for both sides; opponent (graph brain) needs full API.
        // NN side uses masked API for faster feature extraction.
        let (home_full, away_full) = world.build_apis();
        let (home_masked, away_masked) = world.build_apis_masked(home_mask, away_mask);
        let (nn_api, opp_api) = if is_home_nn {
            (&home_masked, &away_full)
        } else {
            (&away_masked, &home_full)
        };

        let mut opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        for i in 0..4 {
            let opp_pos = nn_api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
            if prev_opp_pos_valid {
                opp_vel[i] = (opp_pos - prev_opp_pos[i]) / FIXED_DT;
            }
            prev_opp_pos[i] = opp_pos;
        }
        prev_opp_pos_valid = true;

        let (input, _is_home) = extract_features(nn_api, &opp_vel);
        let act = weights.forward_with_activations(&input);
        let (mut nn_out, actions) = output_to_commands(&act.output, nn_api, rng, false);
        evaluate_and_apply_with_action(nn_api, params, &mut nn_out, &mut det_cache, &actions);
        auto_tackle(nn_api, &mut nn_out, params);

        // Auto-pickup macro
        let is_loose = nn_api.get_bool("Is Ball Loose").unwrap_or(false);
        macro_tick += 1;
        if is_loose {
            let interact_r = nn_api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = nn_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
            let mut best_idx = None;
            let mut best_dist = f32::MAX;
            for i in 0..4 {
                let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has { continue; }
                let p = nn_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                let d = (p - ball).length();
                if d < best_dist {
                    best_dist = d;
                    best_idx = Some(i);
                }
            }
            if let Some(idx) = best_idx {
                if best_dist <= interact_r {
                    nn_out.commands[idx].interact = macro_tick % 2 == 1;
                }
            }
        }

        // Grab-hold release
        if grab_hold_release_tick {
            grab_hold_release_tick = false;
            for i in 0..4 {
                let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has {
                    nn_out.commands[i].interact = false;
                    break;
                }
            }
        }
        if was_loose && !is_loose {
            grab_hold_release_tick = true;
        }
        was_loose = is_loose;

        let opp_out = opp_brain.think(opp_api);

        let (home_out, away_out) = if is_home_nn {
            (&nn_out, &opp_out)
        } else {
            (&opp_out, &nn_out)
        };

        world.step_with_commands(home_out, &away_out, FIXED_DT);

        let sh = world.match_state.score_home;
        let sa = world.match_state.score_away;
        if sh.abs_diff(sa) >= 7 && (sh == 0 || sa == 0) {
            break;
        }
    }

    let (us, them) = if is_home_nn {
        (world.match_state.score_home, world.match_state.score_away)
    } else {
        (world.match_state.score_away, world.match_state.score_home)
    };
    (us, them)
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

fn build_api_mask() -> ApiFieldMask {
    let mut m = ApiFieldMask::none();
    m.needs_bool_set("Is Home Team");
    m.needs_bool_set("Team Has Ball");
    m.needs_bool_set("Opponent Has Ball");
    m.needs_bool_set("Is Ball Loose");
    m.needs_bool_set("Is Kickoff");
    m.needs_transform_set("Ball");
    m.needs_transform_set("Opponent Goal Center");
    m.needs_transform_set("Opponent Goal Left Post");
    m.needs_transform_set("Opponent Goal Right Post");
    m.needs_transform_set("Team Goal Center");
    m.needs_transform_set("Team Goal Left Post");
    m.needs_transform_set("Team Goal Right Post");
    m.needs_float_set("Ball Carrier Shot Charge");
    m.needs_float_set("Ball Carrier Stamina");
    m.needs_float_set("Player Interact Radius");
    m.needs_vector_set("Ball Velocity");
    for n in 1..=4u8 {
        m.needs_transform_set(&format!("Team Player {n}"));
        m.needs_transform_set(&format!("Opponent Player {n}"));
        m.needs_bool_set(&format!("Can Pass to Teammate {n}"));
        m.needs_vector_set(&format!("Perfect Pass Direction to Teammate {n}"));
        m.needs_bool_set(&format!("Team Player {n} Has Ball"));
        m.needs_bool_set(&format!("Opponent Player {n} Has Ball"));
        m.needs_bool_set(&format!("Is Team Player {n} Open"));
        m.needs_bool_set(&format!("Is Opponent Player {n} Open"));
        m.needs_float_set(&format!("Team Player {n} Stamina"));
        m.needs_float_set(&format!("Opponent Player {n} Stamina"));
        m.needs_float_set(&format!("Teammate {n} Shot Charge"));
        m.needs_float_set(&format!("Distance from Team Player {n} to Ball"));
        m.needs_float_set(&format!("Distance from Team Player {n} to Own Goal"));
        m.needs_float_set(&format!("Distance from Team Player {n} to Opponent Goal"));
        m.needs_float_set(&format!("Distance from Opponent Player {n} to Ball"));
        m.needs_bool_set(&format!("Can Opponent Intercept Pass to Teammate {n}"));
        m.needs_transform_set(&format!("Loose Ball Intercept Position for Player {n}"));
        m.needs_transform_set(&format!("Tackle Intercept Position for Player {n}"));
        for m_idx in 1..=4u8 {
            if n != m_idx {
                m.needs_vector_set(&format!("Direction of Team Player {m_idx} from Player {n}"));
                m.needs_vector_set(&format!("Direction of Opponent Player {m_idx} from Player {n}"));
                m.needs_vector_set(&format!("Perfect Intercept Direction: Opponent {m_idx} from Player {n}"));
            }
        }
        m.needs_vector_set(&format!("Receive Pass Direction from Teammate {n}"));
        m.needs_vector_set(&format!("Perfect Pass Direction to Teammate {n}"));
    }
    m.needs_float_set("Best Tackler");
    m
}

struct TrainingState {
    epoch: usize,
    total_epochs: usize,
    best_score_diff: f32,
    lr: f32,
    total_samples: usize,
    epoch_ms: u128,
    selfplay_gf: u32,
    selfplay_ga: u32,
    eval_gf: u32,
    eval_ga: u32,
    eval_diff: f32,
    per_opp: Vec<(String, u32, u32)>,
    history: Vec<(usize, f32, f32)>, // (epoch, eval_diff, selfplay_diff)
    // Live status (updates during epoch, not just after)
    live_phase: String,  // "self-play", "training", "evaluating", "idle"
    live_progress: f32,  // 0.0 to 1.0 within current phase
    live_gf: u32,        // running goal count during self-play
    live_ga: u32,
    epoch_start_secs: f64,  // when current epoch started (for elapsed time)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let matches_per_epoch: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(64);
    let mut lr: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(POLICY_LR);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);

    eprintln!("=== AlphaZero-style Training ===");
    eprintln!("epochs: {epochs}, matches/epoch: {matches_per_epoch}, lr: {lr}, seed: {seed}");
    eprintln!("match_secs: {MATCH_SECS}, gamma: {GAMMA}, td_lambda: {TD_LAMBDA}");
    eprintln!("dirichlet_alpha: {DIRICHLET_ALPHA}, dirichlet_weight: {DIRICHLET_WEIGHT}");

    let params = SimParams::default();
    let cache = Arc::new(ProgramCache::default());
    let opponents = discover_opponents();
    eprintln!("opponents found: {}", opponents.len());
    for opp in &opponents {
        if let BrainInput::Graph(p) = opp {
            eprintln!("  - {}", p.display());
        } else {
            eprintln!("  - {:?}", opp);
        }
    }

    let mask = Arc::new(build_api_mask());

    // Load or create weights.
    let weights_path = "assets/az_weights.json";
    let best_path = "assets/az_best.json";
    let mut weights = NetWeights::load_from_path(weights_path)
        .or_else(|| NetWeights::load_from_path(best_path))
        .unwrap_or_else(|| {
            eprintln!("No saved weights found, using random init");
            NetWeights::random(INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM)
        });
    let mut best_weights = NetWeights::load_from_path(best_path)
        .unwrap_or_else(|| weights.clone());
    let mut best_score_diff = f32::MIN;

    // Dashboard server
    let dashboard_state = Arc::new(Mutex::new(TrainingState {
        epoch: 0,
        total_epochs: epochs,
        best_score_diff: f32::MIN,
        lr,
        total_samples: 0,
        epoch_ms: 0,
        selfplay_gf: 0,
        selfplay_ga: 0,
        eval_gf: 0,
        eval_ga: 0,
        eval_diff: 0.0,
        per_opp: Vec::new(),
        history: Vec::new(),
        live_phase: "starting".to_string(),
        live_progress: 0.0,
        live_gf: 0,
        live_ga: 0,
        epoch_start_secs: 0.0,
    }));
    let dash_clone = Arc::clone(&dashboard_state);
    std::thread::spawn(move || {
        let listener = TcpListener::bind("127.0.0.1:7879").unwrap();
        eprintln!("dashboard: http://127.0.0.1:7879");
        for stream in listener.incoming() {
            let mut stream = match stream { Ok(s) => s, Err(_) => continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let state = dash_clone.lock().unwrap();

            let mut opp_rows = String::new();
            for (name, gf, ga) in &state.per_opp {
                let short_name = name.rsplit('\\').next().or_else(|| name.rsplit('/').next()).unwrap_or(name);
                let diff = *gf as i32 - *ga as i32;
                let color = if diff > 0 { "#4CAF50" } else if diff < 0 { "#f44336" } else { "#999" };
                opp_rows.push_str(&format!(
                    "<tr><td>{}</td><td style=\"text-align:center\">{}</td><td style=\"text-align:center\">{}</td><td style=\"text-align:center;color:{}\">{:+}</td></tr>",
                    short_name, gf, ga, color, diff
                ));
            }

            let mut history_rows = String::new();
            for (ep, ediff, spdiff) in &state.history {
                let color = if *ediff > 0.0 { "#4CAF50" } else if *ediff < 0.0 { "#f44336" } else { "#999" };
                history_rows.push_str(&format!(
                    "<tr><td>{}</td><td style=\"text-align:center;color:{}\">{:+.0}</td><td style=\"text-align:center\">{:+.0}</td></tr>",
                    ep, color, ediff, spdiff
                ));
            }

            let eval_color = if state.eval_diff > 0.0 { "#4CAF50" } else if state.eval_diff < 0.0 { "#f44336" } else { "#999" };
            let best_color = if state.best_score_diff > 0.0 { "#4CAF50" } else if state.best_score_diff < 0.0 { "#f44336" } else { "#999" };

            let phase_color = match state.live_phase.as_str() {
                "self-play" => "#0ff",
                "training" => "#ff0",
                "evaluating" => "#f80",
                _ => "#888",
            };
            let live_bar = (state.live_progress * 100.0).min(100.0);

            let html = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"2\"><title>AZ Training</title>\n<style>\nbody {{ font-family: monospace; background: #1a1a2e; color: #eee; margin: 20px; }}\nh1 {{ color: #0ff; }}\n.card {{ background: #16213e; border-radius: 8px; padding: 16px; margin: 10px 0; }}\n.stat {{ display: inline-block; margin: 0 20px 10px 0; }}\n.stat-label {{ color: #888; font-size: 12px; }}\n.stat-value {{ font-size: 24px; font-weight: bold; }}\ntable {{ border-collapse: collapse; width: 100%; }}\nth, td {{ border: 1px solid #333; padding: 6px 10px; }}\nth {{ background: #0d1b3e; color: #0ff; text-align: left; }}\n.progress {{ background: #333; border-radius: 4px; height: 20px; overflow: hidden; }}\n.progress-bar {{ background: #0ff; height: 100%; transition: width 0.5s; }}\n.live-bar {{ background: #0af; height: 100%; }}\n.live {{ font-size: 18px; color: {}; }}\n</style></head><body>\n<h1>AlphaZero Training</h1>\n<div class=\"progress\"><div class=\"progress-bar\" style=\"width: {:.1}%\"></div></div>\n<div class=\"card\">\n<div class=\"live\">STATUS: {} [{:.0}%] | Live GF:{} GA:{}</div>\n<div class=\"progress\" style=\"margin-top:8px\"><div class=\"live-bar\" style=\"width: {:.1}%\"></div></div>\n</div>\n<div class=\"card\">\n<div class=\"stat\"><div class=\"stat-label\">EPOCH</div><div class=\"stat-value\">{}</div></div>\n<div class=\"stat\"><div class=\"stat-label\">BEST DIFF</div><div class=\"stat-value\" style=\"color:{}\">{:+.0}</div></div>\n<div class=\"stat\"><div class=\"stat-label\">EVAL DIFF</div><div class=\"stat-value\" style=\"color:{}\">{:+.0}</div></div>\n<div class=\"stat\"><div class=\"stat-label\">LR</div><div class=\"stat-value\">{:.6}</div></div>\n<div class=\"stat\"><div class=\"stat-label\">EPOCH TIME</div><div class=\"stat-value\">{}ms</div></div>\n<div class=\"stat\"><div class=\"stat-label\">TOTAL SAMPLES</div><div class=\"stat-value\">{}</div></div>\n</div>\n<div class=\"card\">\n<h3>Self-Play (last epoch)</h3>\n<p>GF: {} | GA: {} | Diff: {:+}</p>\n</div>\n<div class=\"card\">\n<h3>Evaluation vs Opponents</h3>\n<table><tr><th>Opponent</th><th>GF</th><th>GA</th><th>Diff</th></tr>{}</table>\n</div>\n<div class=\"card\">\n<h3>History</h3>\n<table><tr><th>Epoch</th><th>Eval</th><th>SelfPlay</th></tr>{}</table>\n</div>\n</body></html>",
                phase_color,
                state.live_phase, live_bar, state.live_gf, state.live_ga,
                live_bar,
                if state.total_epochs > 0 { state.epoch as f64 / state.total_epochs as f64 * 100.0 } else { 0.0 },
                state.epoch,
                best_color, state.best_score_diff,
                eval_color, state.eval_diff,
                state.lr,
                state.epoch_ms,
                state.total_samples,
                state.selfplay_gf, state.selfplay_ga, state.selfplay_gf as i32 - state.selfplay_ga as i32,
                opp_rows,
                history_rows
            );
            let _ = stream.write_all(html.as_bytes());
        }
    });

    let mut rng = StdRng::seed_from_u64(seed);

    // === Phase 1: Supervised learning from expert demonstrations ===
    // Run strongest opponents against each other, record states + actions, train NN to imitate.
    const SUP_EPOCHS: usize = 15;
    const SUP_MATCHES: usize = 64;
    eprintln!("\n=== Phase 1: Supervised Learning from Expert Demos ===");
    eprintln!("sup_epochs: {SUP_EPOCHS}, sup_matches: {SUP_MATCHES}");

    // Pick strongest opponents for expert demos
    let strong_opps: Vec<BrainInput> = opponents.iter()
        .filter(|o| {
            if let BrainInput::Graph(p) = o {
                let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                name.contains("Titanium_v7") || name.contains("ZudanFC3.5") || name.contains("Nova") || name.contains("Haialand")
            } else { false }
        })
        .cloned()
        .collect();
    eprintln!("strong opponents for demos: {}", strong_opps.len());
    for opp in &strong_opps {
        if let BrainInput::Graph(p) = opp { eprintln!("  - {}", p.display()); }
    }

    if !strong_opps.is_empty() {
        for sup_epoch in 0..SUP_EPOCHS {
            let sup_start = Instant::now();

            // Update dashboard
            {
                let mut st = dashboard_state.lock().unwrap();
                st.live_phase = "supervised".to_string();
                st.live_progress = sup_epoch as f32 / SUP_EPOCHS as f32;
                st.epoch = sup_epoch;
                st.lr = lr;
            }

            // Run expert matches in parallel
            let params_clone = params.clone();
            let sup_results: Vec<Vec<AzSample>> = (0..SUP_MATCHES)
                .into_par_iter()
                .map(|i| {
                    let match_seed = seed + sup_epoch as u64 * SUP_MATCHES as u64 + i as u64;
                    let mut match_rng = StdRng::seed_from_u64(match_seed);
                    let kickoff = if i % 2 == 0 { TeamId::Home } else { TeamId::Away };

                    // Pick two random strong opponents
                    let a_idx = match_rng.gen_range(0..strong_opps.len());
                    let b_idx = match_rng.gen_range(0..strong_opps.len());
                    let mut expert_a = build_brain(&strong_opps[a_idx], GraphEngine::Runtime, Some(&cache)).unwrap();
                    let mut expert_b = build_brain(&strong_opps[b_idx], GraphEngine::Runtime, Some(&cache)).unwrap();

                    run_expert_match(
                        expert_a.as_mut(), expert_b.as_mut(),
                        &params_clone, &mut match_rng, kickoff,
                        None, None,
                    )
                })
                .collect();

            // Collect samples
            let mut all_samples: Vec<AzSample> = Vec::new();
            for samples in &sup_results {
                all_samples.extend(samples.iter().cloned());
            }

            eprintln!("  Sup epoch {} | {} matches | {}ms | {} samples",
                sup_epoch, SUP_MATCHES, sup_start.elapsed().as_millis(), all_samples.len());

            // Train on expert samples with mini-batches
            if !all_samples.is_empty() {
                all_samples.shuffle(&mut rng);
                let sup_lr = lr * 0.5; // Moderate LR for supervised mini-batch training
                const SUP_BATCH: usize = 4096;
                for chunk in all_samples.chunks(SUP_BATCH) {
                    az_train_step(&mut weights, chunk, sup_lr, sup_lr * 2.0);
                }
            }

            // Evaluate every 5 sup epochs
            if sup_epoch % 5 == 0 || sup_epoch == SUP_EPOCHS - 1 {
                let mut eval_gf = 0u32;
                let mut eval_ga = 0u32;
                let eval_opps = opponents.iter().take(4).collect::<Vec<_>>();
                for opp_input in &eval_opps {
                    let mut opp_brain = match build_brain(opp_input, GraphEngine::Runtime, Some(&cache)) {
                        Ok(b) => b, Err(_) => continue,
                    };
                    let nn_side = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
                    let kickoff = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
                    let (us, them) = evaluate_match(
                        &weights, opp_brain.as_mut(), nn_side, kickoff, &params, &mut rng,
                        Some(&mask), Some(&mask),
                    );
                    eval_gf += us;
                    eval_ga += them;
                }
                let eval_diff = eval_gf as f32 - eval_ga as f32;
                eprintln!("  Sup eval: GF{} GA{} diff={:+}", eval_gf, eval_ga, eval_diff);

                if eval_diff > best_score_diff {
                    best_score_diff = eval_diff;
                    best_weights = weights.clone();
                    best_weights.save_to_path(best_path).ok();
                    eprintln!("  >>> NEW BEST! saved to {}", best_path);
                }

                // Update dashboard
                {
                    let mut st = dashboard_state.lock().unwrap();
                    st.eval_diff = eval_diff;
                    st.eval_gf = eval_gf;
                    st.eval_ga = eval_ga;
                    st.best_score_diff = best_score_diff;
                    st.total_samples += all_samples.len();
                }
            }

            weights.save_to_path(weights_path).ok();
        }
        eprintln!("=== Phase 1 complete. Starting Phase 2: Self-play ===\n");
    }

    for epoch in 0..epochs {
        let epoch_start = Instant::now();

        // Update live status
        {
            let mut st = dashboard_state.lock().unwrap();
            st.live_phase = "self-play".to_string();
            st.live_progress = 0.0;
            st.live_gf = 0;
            st.live_ga = 0;
        }

        // Self-play: generate training data in parallel with live counters.
        let weights_clone = weights.clone();
        let params_clone = params.clone();
        let mask_clone = Arc::clone(&mask);

        // Atomic counters for real-time dashboard updates
        let live_done = Arc::new(AtomicUsize::new(0));
        let live_gf = Arc::new(AtomicU32::new(0));
        let live_ga = Arc::new(AtomicU32::new(0));

        let match_configs: Vec<(TeamId, u64)> = (0..matches_per_epoch)
            .map(|i| {
                let kickoff = if i % 2 == 0 { TeamId::Home } else { TeamId::Away };
                let match_seed = seed + (epoch as u64) * (matches_per_epoch as u64) + i as u64;
                (kickoff, match_seed)
            })
            .collect();

        let live_done_clone = Arc::clone(&live_done);
        let live_gf_clone = Arc::clone(&live_gf);
        let live_ga_clone = Arc::clone(&live_ga);
        let dash_clone2 = Arc::clone(&dashboard_state);

        let results: Vec<(Vec<AzSample>, u32, u32)> = match_configs
            .par_iter()
            .map(|&(kickoff, match_seed)| {
                let mut match_rng = StdRng::seed_from_u64(match_seed);
                let (samples, us, them) = run_self_play_match(
                    &weights_clone,
                    &params_clone,
                    &mut match_rng,
                    kickoff,
                    Some(&mask_clone),
                    Some(&mask_clone),
                );
                // Update atomic counters
                let done = live_done_clone.fetch_add(1, Ordering::Relaxed) + 1;
                let gf = live_gf_clone.fetch_add(us, Ordering::Relaxed) + us;
                let ga = live_ga_clone.fetch_add(them, Ordering::Relaxed) + them;
                // Update dashboard state
                let mut st = dash_clone2.lock().unwrap();
                st.live_progress = done as f32 / matches_per_epoch as f32;
                st.live_gf = gf;
                st.live_ga = ga;
                (samples, us, them)
            })
            .collect();

        // Collect all samples.
        let mut all_samples: Vec<AzSample> = Vec::new();
        let mut total_gf = 0u32;
        let mut total_ga = 0u32;
        for (samples, us, them) in &results {
            all_samples.extend(samples.iter().cloned());
            total_gf += us;
            total_ga += them;
        }

        let diff = total_gf as f32 - total_ga as f32;
        let elapsed = epoch_start.elapsed().as_millis();
        eprintln!("=== Epoch {} | {} matches | {}ms | {} samples ===",
            epoch, matches_per_epoch, elapsed, all_samples.len());
        eprintln!("  Self-play: GF{} GA{} diff={:+}", total_gf, total_ga, diff);

        // Update live status: training phase
        {
            let mut st = dashboard_state.lock().unwrap();
            st.live_phase = "training".to_string();
            st.live_progress = 0.0;
            st.live_gf = total_gf;
            st.live_ga = total_ga;
        }

        // Train on collected samples.
        if !all_samples.is_empty() {
            // Shuffle samples for training.
            all_samples.shuffle(&mut rng);
            az_train_step(&mut weights, &all_samples, lr, lr * 2.0);
        }

        // Evaluate against opponents.
        {
            let mut st = dashboard_state.lock().unwrap();
            st.live_phase = "evaluating".to_string();
            st.live_progress = 0.0;
        }
        let eval_opponents = opponents.iter().take(8).collect::<Vec<_>>();
        let mut eval_gf = 0u32;
        let mut eval_ga = 0u32;
        let mut per_opp_results: Vec<(String, u32, u32)> = Vec::new();
        for (ei, opp_input) in eval_opponents.iter().enumerate() {
            let mut opp_brain = match build_brain(opp_input, GraphEngine::Runtime, Some(&cache)) {
                Ok(b) => b,
                Err(e) => { eprintln!("  Failed to build opponent: {e}"); continue; }
            };
            let nn_side = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
            let kickoff = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
            let (us, them) = evaluate_match(
                &weights, opp_brain.as_mut(), nn_side, kickoff, &params, &mut rng,
                Some(&mask), Some(&mask),
            );
            eval_gf += us;
            eval_ga += them;
            let opp_name = if let BrainInput::Graph(p) = opp_input { p.display().to_string() } else { format!("{:?}", opp_input) };
            eprintln!("  vs {:30} GF{:2} GA{:2} diff={:+3}", opp_name, us, them, us as i32 - them as i32);
            per_opp_results.push((opp_name, us, them));
            // Update dashboard live during eval
            {
                let mut st = dashboard_state.lock().unwrap();
                st.live_progress = (ei + 1) as f32 / eval_opponents.len() as f32;
                st.per_opp.clear();
                st.per_opp.extend(per_opp_results.iter().cloned());
            }
        }
        let eval_diff = eval_gf as f32 - eval_ga as f32;
        eprintln!("  EVAL: GF{} GA{} diff={:+}", eval_gf, eval_ga, eval_diff);

        // Save best.
        if eval_diff > best_score_diff {
            best_score_diff = eval_diff;
            best_weights = weights.clone();
            best_weights.save_to_path(best_path).ok();
            eprintln!("  >>> NEW BEST! saved to {}", best_path);
        }

        // Save current weights.
        weights.save_to_path(weights_path).ok();

        // Anti-collapse: restore if degraded too far.
        if eval_diff < best_score_diff - RESTORE_THRESHOLD && best_score_diff > f32::MIN + 1.0 {
            eprintln!("  !!! Anti-collapse: restoring best weights");
            weights = best_weights.clone();
        }

        // Learning rate decay.
        lr *= LR_DECAY;

        // Update dashboard.
        let mut state = dashboard_state.lock().unwrap();
        state.epoch = epoch + 1;
        state.best_score_diff = best_score_diff;
        state.lr = lr;
        state.epoch_ms = elapsed;
        state.selfplay_gf = total_gf;
        state.selfplay_ga = total_ga;
        state.eval_gf = eval_gf;
        state.eval_ga = eval_ga;
        state.eval_diff = eval_diff;
        state.total_samples += all_samples.len();
        state.per_opp.clear();
        state.per_opp.extend(per_opp_results.iter().cloned());
        state.history.push((epoch + 1, eval_diff, diff));
        if state.history.len() > 50 {
            let drain_count = state.history.len() - 50;
            state.history.drain(0..drain_count);
        }
    }

    eprintln!("Training complete. Best diff: {:.1}", best_score_diff);
}
