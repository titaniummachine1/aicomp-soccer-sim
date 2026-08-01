use std::time::Instant;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io::{Read, Write};
use std::net::TcpListener;

use aicomp_soccer_sim::api::ApiFieldMask;
use aicomp_soccer_sim::replay::Replay;
use aicomp_soccer_sim::batch::{build_brain, BrainInput, GraphEngine, ProgramCache};
use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::train::{
    Activations, NetGradients, NetWeights, TrainedBrain,
    INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM,
    VALUES_PER_PLAYER, COMPASS_DIRS, SOFTMAX_GROUPS,
    SPRINT_OFFSET, TACKLE_OFFSET, TEAM_OUTPUT_START,
    extract_features,
};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::possession::reset_possession_for_kickoff;
use aicomp_soccer_sim::deterministic::{
    evaluate_and_apply, evaluate_and_apply_with_action, DeterministicCache, NNActions, ShootTarget,
};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand::seq::SliceRandom;
use rayon::prelude::*;

const NORM: f32 = 20.0;
const ACTION_SIGMA: f32 = 0.3;
const GAMMA: f32 = 0.99;
const TIME_SCALE: f32 = 20.0;
const INGAME_MATCH_MIN: f32 = 90.0;
/// 90 in-game minutes at 20x = 270 sim seconds
const MATCH_SECS: f32 = INGAME_MATCH_MIN * 60.0 / TIME_SCALE;

// PPO hyperparameters
const PPO_CLIP_EPS: f32 = 0.2;
const PPO_EPOCHS: usize = 1;
const GAE_LAMBDA: f32 = 0.95;
const MAX_GRAD_NORM: f32 = 0.5;
const REWARD_SCALE: f32 = 0.01;
const LR_DECAY: f32 = 0.995;
const BASELINE_DECAY: f32 = 0.95;
const SIGMA_MIN: f32 = 0.08;
const SIGMA_DECAY: f32 = 0.99;
const RESTORE_THRESHOLD: f32 = 30.0;
const MAX_TRAIN_STEPS: usize = 800;


#[derive(Clone)]
struct Step {
    activations: Activations,
    action_noise: [f32; OUTPUT_DIM],
    reward: f32,
    old_log_prob: f32,
}

fn extract_features_raw(
    api: &aicomp_soccer_sim::api::TeamApi,
    opp_vel: &[bevy::prelude::Vec2; 4],
) -> [f32; INPUT_DIM] {
    let (input, _) = extract_features(api, opp_vel);
    input
}

fn compute_reward(
    api: &aicomp_soccer_sim::api::TeamApi,
    prev_score_us: u32,
    prev_score_them: u32,
    cur_score_us: u32,
    cur_score_them: u32,
    tackled: bool,
    pass_completed: bool,
    ball_intercepted: bool,
    whistle: bool,
    ball_lost: bool,
    aimbot_active: bool,
    shot_taken: bool,
    ball_recovered: bool,
) -> f32 {
    let mut r = 0.0;

    // Goal scored/conceded — strongest signals.
    if cur_score_us > prev_score_us {
        r += 15.0;
    }
    if cur_score_them > prev_score_them {
        r -= 8.0;
    }

    // Tackle success.
    if tackled {
        r += 1.0;
    }

    // Pass completion.
    if pass_completed {
        r += 0.5;
    }

    // Ball intercepted by opponent.
    if ball_intercepted {
        r -= 0.5;
    }

    // Ball lost.
    if ball_lost {
        r -= 2.0;
    }

    // Ball recovered from loose (not tackle, just picking up loose ball).
    if ball_recovered {
        r += 0.3;
    }

    // Shot taken — small positive reward to encourage shooting.
    if shot_taken {
        r += 0.1;
    }

    // Aimbot active means guaranteed scoring opportunity — reward taking it.
    if aimbot_active {
        r += 1.0;
    }

    // Stale-ball whistle penalty.
    if whistle {
        r -= 0.2;
    }

    // Tiny time pressure to encourage action.
    r -= 0.00001;

    let ball = api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
    let is_home = api.get_bool("Is Home Team").unwrap_or(true);
    let field_x = 40.0;

    // Tug-of-war: ball field position mapped to [-1, +1].
    let ball_advancement = if is_home {
        (ball.x / field_x).clamp(-1.0, 1.0)
    } else {
        (-ball.x / field_x).clamp(-1.0, 1.0)
    };
    r += ball_advancement * 0.001;

    let team_has_ball = api.get_bool("Team Has Ball").unwrap_or(false);
    let opp_has_ball = api.get_bool("Opponent Has Ball").unwrap_or(false);

    // Ball near own goal under pressure — big penalty.
    let ball_near_own_goal = if is_home {
        ball.x < -25.0
    } else {
        ball.x > 25.0
    };
    if ball_near_own_goal && !team_has_ball {
        r -= 0.005;
    }

    // Reward for ball in opponent's third with possession.
    let ball_in_opp_third = if is_home {
        ball.x > 20.0
    } else {
        ball.x < -20.0
    };
    if ball_in_opp_third && team_has_ball {
        r += 0.002;
    }

    // Penalty for wasteful sprinting when not near ball.
    let ball_vel = api.get_vector3("Ball Velocity").flatten().unwrap_or(bevy::prelude::Vec2::ZERO);
    let _ = ball_vel;

    r
}

/// Decode NN output + noise into BrainOutput and NNActions.
/// Output layout (157 dims):
/// Per player (4 × 37 = 148): 8 softmax direction groups (35 logits) + sprint(1) + tackle(1)
/// Team-level (9): shoot_signal(1) + shoot_target(3) + pass_signal(1) + pass_target(4)
fn output_to_commands(
    output: &[f32],
    noise: &[f32; OUTPUT_DIM],
    api: &aicomp_soccer_sim::api::TeamApi,
    rng: &mut StdRng,
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

    // Per-player: decode softmax direction groups + sprint + tackle.
    for i in 0..4 {
        let base = i * VALUES_PER_PLAYER;

        // For each softmax group, add noise to logits and sample.
        // Pick the group with the highest sampled logit across all groups.
        let mut best_dir = bevy::prelude::Vec2::ZERO;
        let mut best_logit = f32::MIN;

        for (grp_offset, grp_size) in SOFTMAX_GROUPS {
            // Sample from this group's softmax.
            let mut logits: Vec<f32> = Vec::with_capacity(*grp_size);
            for j in 0..*grp_size {
                logits.push(output[base + grp_offset + j] + noise[base + grp_offset + j]);
            }
            let max_logit = logits.iter().cloned().fold(f32::MIN, f32::max);
            let mut exp_vals: Vec<f32> = Vec::with_capacity(*grp_size);
            let mut sum_exp = 0.0;
            for l in &logits {
                let e = (l - max_logit).exp();
                exp_vals.push(e);
                sum_exp += e;
            }
            let sample = rng.gen_range(0.0..sum_exp.max(1e-10));
            let mut cum = 0.0;
            let mut chosen = 0usize;
            for j in 0..*grp_size {
                cum += exp_vals[j];
                if sample <= cum {
                    chosen = j;
                    break;
                }
            }
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
        let sprint_sig = output[base + SPRINT_OFFSET] + noise[base + SPRINT_OFFSET];
        let tackle_sig = output[base + TACKLE_OFFSET] + noise[base + TACKLE_OFFSET];

        let interact = if has_ball[i] {
            true
        } else {
            tackle_sig > 0.0
        };

        out.commands[i] = BrainCommand {
            move_to,
            sprint: sprint_sig > 0.0,
            interact,
            shoot: false,
        };
    }

    // Team-level: shoot_signal(1) + shoot_target(3) + pass_signal(1) + pass_target(4)
    let shoot_signal = output[TEAM_OUTPUT_START] + noise[TEAM_OUTPUT_START];
    let pass_signal = output[TEAM_OUTPUT_START + 4] + noise[TEAM_OUTPUT_START + 4];

    let mut shoot_target = ShootTarget::Center;
    let mut shoot = false;
    if shoot_signal > 0.0 {
        shoot = true;
        let mut logits = [0.0f32; 3];
        for a in 0..3 {
            logits[a] = output[TEAM_OUTPUT_START + 1 + a] + noise[TEAM_OUTPUT_START + 1 + a];
        }
        let max_logit = logits.iter().cloned().fold(f32::MIN, f32::max);
        let mut exp_vals = [0.0f32; 3];
        let mut sum_exp = 0.0;
        for a in 0..3 {
            exp_vals[a] = (logits[a] - max_logit).exp();
            sum_exp += exp_vals[a];
        }
        let sample = rng.gen_range(0.0..sum_exp.max(1e-10));
        let mut cum = 0.0;
        let mut best = 0usize;
        for a in 0..3 {
            cum += exp_vals[a];
            if sample <= cum {
                best = a;
                break;
            }
        }
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
            logits[a] = output[TEAM_OUTPUT_START + 5 + a] + noise[TEAM_OUTPUT_START + 5 + a];
        }
        let max_logit = logits.iter().cloned().fold(f32::MIN, f32::max);
        let mut exp_vals = [0.0f32; 4];
        let mut sum_exp = 0.0;
        for a in 0..4 {
            exp_vals[a] = (logits[a] - max_logit).exp();
            sum_exp += exp_vals[a];
        }
        let sample = rng.gen_range(0.0..sum_exp.max(1e-10));
        let mut cum = 0.0;
        let mut best = 0usize;
        for a in 0..4 {
            cum += exp_vals[a];
            if sample <= cum {
                best = a;
                break;
            }
        }
        pass_target = best;
    }

    if shoot {
        for i in 0..4 {
            if has_ball[i] {
                out.commands[i].shoot = true;
            }
        }
    }
    let actions = NNActions {
        shoot,
        shoot_target,
        pass,
        pass_target,
    };

    (out, actions)
}

fn log_prob_grad(_mean: &[f32], noise: &[f32; OUTPUT_DIM]) -> [f32; OUTPUT_DIM] {
    let mut grad = [0.0f32; OUTPUT_DIM];
    for i in 0..OUTPUT_DIM {
        grad[i] = noise[i] / (ACTION_SIGMA * ACTION_SIGMA);
    }
    grad
}

/// Auto-tackle macro: when opponent has the ball and an NN player is within
/// tackle range, force that player to interact (tackle). If multiple players
/// are in range, pick the one with lowest stamina that can still win the
/// tackle (stamina >= carrier stamina). Only one player tackles at a time.
fn auto_tackle(
    nn_api: &aicomp_soccer_sim::api::TeamApi,
    nn_out: &mut BrainOutput,
    params: &SimParams,
) {
    let opp_has_ball = nn_api.get_bool("Opponent Has Ball").unwrap_or(false);
    if !opp_has_ball {
        return;
    }

    let interact_r = params.interact_radius;
    let carrier_stam = nn_api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;

    // Find the opponent carrier position.
    let mut carrier_pos = bevy::prelude::Vec2::ZERO;
    for i in 0..4u8 {
        if nn_api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) {
            carrier_pos = nn_api.get_transform(&format!("Opponent Player {}", i + 1))
                .unwrap_or(bevy::prelude::Vec2::ZERO);
            break;
        }
    }

    // Find all NN players within tackle range (regardless of NN's interact choice).
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    for i in 0..4 {
        let p = nn_api.get_transform(&format!("Team Player {}", i + 1))
            .unwrap_or(bevy::prelude::Vec2::ZERO);
        let dist = (p - carrier_pos).length();
        if dist <= interact_r {
            let stam = nn_api.get_float(&format!("Team Player {} Stamina", i + 1))
                .unwrap_or(0.0) / 100.0;
            candidates.push((i, stam));
        }
    }

    if candidates.is_empty() {
        return;
    }

    // Among candidates who can win (stamina >= carrier), pick lowest stamina.
    // If nobody can win, pick the closest one (highest stamina) to at least try.
    let chosen = candidates
        .iter()
        .filter(|(_, s)| *s >= carrier_stam - 1e-4)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .or_else(|| {
            candidates.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(i, _)| *i)
        .unwrap_or(0);

    // Force chosen to interact, clear all others.
    for (i, _) in &candidates {
        nn_out.commands[*i].interact = *i == chosen;
    }
}

/// Tackle coordinator macro: when multiple NN players try to tackle the same
/// carrier on the same tick, pick the one with the lowest stamina that can
/// still successfully win the tackle (stamina >= carrier stamina), and force
/// all others to not interact this tick. This avoids wasting stamina on
/// simultaneous tackle attempts — borrowed from Titanium's strategy.
fn coordinate_tackles(
    nn_api: &aicomp_soccer_sim::api::TeamApi,
    nn_out: &mut BrainOutput,
    params: &SimParams,
) {
    let opp_has_ball = nn_api.get_bool("Opponent Has Ball").unwrap_or(false);
    if !opp_has_ball {
        return;
    }

    let interact_r = params.interact_radius;
    let carrier_stam = nn_api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;

    // Find the opponent carrier position.
    let mut carrier_pos = bevy::prelude::Vec2::ZERO;
    for i in 0..4u8 {
        if nn_api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) {
            carrier_pos = nn_api.get_transform(&format!("Opponent Player {}", i + 1))
                .unwrap_or(bevy::prelude::Vec2::ZERO);
            break;
        }
    }

    // Find all NN players who are trying to interact AND are within tackle range.
    let mut tacklers: Vec<(usize, f32)> = Vec::new();
    for i in 0..4 {
        if !nn_out.commands[i].interact {
            continue;
        }
        let p = nn_api.get_transform(&format!("Team Player {}", i + 1))
            .unwrap_or(bevy::prelude::Vec2::ZERO);
        let dist = (p - carrier_pos).length();
        if dist <= interact_r {
            let stam = nn_api.get_float(&format!("Team Player {} Stamina", i + 1))
                .unwrap_or(0.0) / 100.0;
            tacklers.push((i, stam));
        }
    }

    if tacklers.len() <= 1 {
        return;
    }

    // Only activate tiebreaker if at least one tackler can win.
    // If nobody can win, leave everything as-is — the NN does what it wants.
    let can_win: Vec<&(usize, f32)> = tacklers
        .iter()
        .filter(|(_, s)| *s >= carrier_stam - 1e-4)
        .collect();
    if can_win.is_empty() {
        return;
    }

    // Among tacklers who can win, pick the one with the LOWEST stamina
    // to minimize waste. Force all others to not interact this tick.
    let chosen = can_win
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| *i)
        .unwrap_or(0);

    for (i, _) in &tacklers {
        if *i != chosen {
            nn_out.commands[*i].interact = false;
        }
    }
}

fn run_trajectory(
    weights: &NetWeights,
    opp_brain: &mut dyn TeamBrain,
    nn_side: TeamId,
    kickoff: TeamId,
    params: &SimParams,
    rng: &mut StdRng,
    mut replay: Option<&mut Replay>,
) -> (Vec<Step>, u32, u32) {
    let mut steps = Vec::new();
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), kickoff);
    let is_home_nn = nn_side == TeamId::Home;
    let nn_brain = TrainedBrain::with_weights(weights.clone()).with_team(nn_side);
    if is_home_nn {
        world.set_kickoff_formation(TeamId::Home, nn_brain.kickoff_formation());
        world.set_kickoff_formation(TeamId::Away, opp_brain.kickoff_formation());
    } else {
        world.set_kickoff_formation(TeamId::Home, opp_brain.kickoff_formation());
        world.set_kickoff_formation(TeamId::Away, nn_brain.kickoff_formation());
    }
    if world.params.kickoff_delay_s < 1.0 {
        world.params.kickoff_delay_s = 4.9;
    }

    let max_ticks = ((MATCH_SECS / FIXED_DT).ceil() as u64).max(1);
    let mut prev_us = 0u32;
    let mut prev_them = 0u32;
    let mut macro_tick: u32 = 0;
    let mut was_loose = false;
    let mut grab_hold_release_tick = false;
    let mut opp_had_ball = false;
    let mut prev_nn_had_ball = false;
    let mut prev_carrier_slot: Option<u8> = None;
    let mut prev_phase = MatchPhase::Kickoff;

    let home_mask = nn_brain.api_mask();
    let away_mask = opp_brain.api_mask();

    let mut det_cache_nn = DeterministicCache::default();
    let mut det_cache_opp = DeterministicCache::default();
    let mut prev_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_valid = false;

    for _ in 0..max_ticks {
        let (home_api, away_api) = world.build_apis_masked(home_mask.as_ref(), away_mask.as_ref());
        let (nn_api, opp_api) = if is_home_nn {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        // Compute opponent velocities from position deltas.
        let mut cur_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
        for (i, id) in aicomp_soccer_sim::player::PlayerId::ALL.iter().enumerate() {
            cur_opp_pos[i] = nn_api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
        }
        let mut opp_vel = [bevy::prelude::Vec2::ZERO; 4];
        if prev_opp_pos_valid {
            for i in 0..4 {
                opp_vel[i] = (cur_opp_pos[i] - prev_opp_pos[i]) / FIXED_DT;
            }
        }
        prev_opp_pos = cur_opp_pos;
        prev_opp_pos_valid = true;

        let input = extract_features_raw(nn_api, &opp_vel);
        let act = weights.forward_with_activations(&input);

        let mut noise = [0.0f32; OUTPUT_DIM];
        for i in 0..OUTPUT_DIM {
            noise[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
        }

        let (mut nn_out, nn_actions) = output_to_commands(&act.output, &noise, nn_api, rng);
        let mut opp_out = opp_brain.think(opp_api);

        // NN side: guaranteed score detection (always on) + NN-invoked aimbot.
        let det_out = evaluate_and_apply_with_action(nn_api, params, &mut nn_out, &mut det_cache_nn, &nn_actions);
        let aimbot_active = det_out.can_score || det_out.can_walk_in || det_out.ball_going_in;
        // Opponent side: just guaranteed score detection.
        evaluate_and_apply(opp_api, params, &mut opp_out, &mut det_cache_opp);

        // Auto-pickup macro: if ball is loose, force interact on the closest
        // NN player within pickup range. Pulses every other tick to ensure a
        // rising edge in the interact_bits history — the possession system
        // requires a rising edge (false→true) to trigger pickup.
        // Odd ticks: force interact=true on closest player to ball.
        // Even ticks: force interact=false on that same player to reset the
        // latch, so the next odd tick creates a fresh rising edge.
        let is_loose = nn_api.get_bool("Is Ball Loose").unwrap_or(false);
        macro_tick += 1;
        if is_loose {
            let interact_r = nn_api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = nn_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);

            // Find closest NN player to the ball who doesn't already have it.
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
                    // Pulse: odd ticks press, even ticks release.
                    nn_out.commands[idx].interact = macro_tick % 2 == 1;
                }
            }
        }

        // Grab-hold release: when the ball transitions from loose to held
        // (pickup just happened), the carrier has grab_hold_active=true which
        // blocks charging until interact is released. The det layer always
        // sets interact=true (charge), so grab_hold never clears. Force
        // interact=false on the carrier for one tick to clear it.
        if grab_hold_release_tick {
            grab_hold_release_tick = false;
            // Find the carrier and force interact=false for this tick.
            for i in 0..4 {
                let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                if has {
                    nn_out.commands[i].interact = false;
                    break;
                }
            }
        }
        // Detect loose→held transition for next tick.
        if was_loose && !is_loose {
            grab_hold_release_tick = true;
        }
        was_loose = is_loose;

        // Auto-tackle: force closest player to tackle when in range.
        auto_tackle(nn_api, &mut nn_out, params);
        // Tackle coordinator: when multiple NN players try to tackle the same
        // carrier, pick the best one and suppress the rest.
        coordinate_tackles(nn_api, &mut nn_out, params);

        let (home_out, away_out) = if is_home_nn {
            (&nn_out, &opp_out)
        } else {
            (&opp_out, &nn_out)
        };
        world.step_with_commands(home_out, away_out, FIXED_DT);
        if let Some(r) = replay.as_deref_mut() {
            r.record_tick(&world.ball, &world.players, &world.possession, &world.match_state);
        }
        let sh_after = world.match_state.score_home;
        let sa_after = world.match_state.score_away;

        let (cur_us, cur_them) = if is_home_nn {
            (sh_after, sa_after)
        } else {
            (sa_after, sh_after)
        };

        // Detect tackle: opponent had ball last tick, now NN team has it.
        let nn_has_ball = nn_api.get_bool("Team Has Ball").unwrap_or(false);
        let opp_has_ball = nn_api.get_bool("Opponent Has Ball").unwrap_or(false);
        let tackled = opp_had_ball && nn_has_ball;
        let ball_lost = prev_nn_had_ball && opp_has_ball && !nn_has_ball;
        opp_had_ball = opp_has_ball;
        prev_nn_had_ball = nn_has_ball;

        // Detect pass completion: NN carrier changed from one teammate to another.
        let cur_carrier_slot: Option<u8> = (0..4u8).find(|&i| {
            nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false)
        });
        let pass_completed = match (prev_carrier_slot, cur_carrier_slot) {
            (Some(prev), Some(cur)) if prev != cur && nn_has_ball => true,
            _ => false,
        };

        // Detect ball intercepted: NN had ball, released it (loose), then opponent got it.
        let ball_intercepted = was_loose && opp_has_ball && !nn_has_ball;

        // Detect ball recovered from loose: was loose, now NN has it (not tackle).
        let ball_recovered = was_loose && nn_has_ball && !opp_has_ball;

        // Detect shot taken: any NN player with shoot flag set.
        let shot_taken = nn_out.commands.iter().any(|c| c.shoot);

        // Detect whistle: phase transition to GoalPause without a score change.
        let cur_phase = world.match_state.phase;
        let whistle = cur_phase == MatchPhase::GoalPause
            && prev_phase != MatchPhase::GoalPause
            && cur_us == prev_us
            && cur_them == prev_them;
        prev_phase = cur_phase;

        let reward = compute_reward(
            nn_api, prev_us, prev_them, cur_us, cur_them,
            tackled, pass_completed, ball_intercepted, whistle,
            ball_lost, aimbot_active, shot_taken, ball_recovered,
        );
        prev_us = cur_us;
        prev_them = cur_them;
        prev_carrier_slot = cur_carrier_slot;

        let old_log_prob = weights.log_prob(&act.output, &noise, ACTION_SIGMA);
        steps.push(Step {
            activations: act,
            action_noise: noise,
            reward,
            old_log_prob,
        });

        if sh_after.abs_diff(sa_after) >= 7 && (sh_after == 0 || sa_after == 0) {
            break;
        }
    }

    // Tiebreaker: if tied, play extra periods with progressive player removal.
    // 3v3 → 2v2 → 1v1. Each period is another MATCH_SECS. If still tied at 1v1,
    // the team with more possession time wins (stored as a "goal" for tiebreak).
    let (final_us, final_them) = if is_home_nn {
        (world.match_state.score_home, world.match_state.score_away)
    } else {
        (world.match_state.score_away, world.match_state.score_home)
    };

    let (final_us, final_them) = if final_us == final_them && final_us > 0 {
        run_tiebreaker(world, weights, opp_brain, is_home_nn, params, rng, &mut steps, final_us, final_them, home_mask.as_ref(), away_mask.as_ref())
    } else {
        (final_us, final_them)
    };

    (steps, final_us, final_them)
}

/// Tiebreaker flow:
/// 1. Extra time: 4v4 for another 90 min. If still tied →
/// 2. Remove 1 random player per team → 3v3 for 90 min. If still tied →
/// 3. Remove 1 random player per team → 2v2 for 90 min. If still tied →
/// 4. Remove 1 random player per team → 1v1 for 90 min. If still tied →
/// 5. Better possession stats wins. If possession also tied → random.
fn run_tiebreaker(
    mut world: MatchWorld,
    weights: &NetWeights,
    opp_brain: &mut dyn TeamBrain,
    is_home_nn: bool,
    params: &SimParams,
    rng: &mut StdRng,
    steps: &mut Vec<Step>,
    mut prev_us: u32,
    mut prev_them: u32,
    home_mask: Option<&ApiFieldMask>,
    away_mask: Option<&ApiFieldMask>,
) -> (u32, u32) {
    let max_ticks = ((MATCH_SECS / FIXED_DT).ceil() as u64).max(1);
    let mut det_cache_nn = DeterministicCache::default();
    let mut det_cache_opp = DeterministicCache::default();
    let mut prev_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_valid = false;

    // Helper: play one extra period.
    let mut play_period = |world: &mut MatchWorld,
                       weights: &NetWeights,
                       opp_brain: &mut dyn TeamBrain,
                       is_home_nn: bool,
                       params: &SimParams,
                       rng: &mut StdRng,
                       steps: &mut Vec<Step>,
                       prev_us: &mut u32,
                       prev_them: &mut u32,
                       det_cache_nn: &mut DeterministicCache,
                       det_cache_opp: &mut DeterministicCache,
                       home_mask: Option<&ApiFieldMask>,
                       away_mask: Option<&ApiFieldMask>| -> (u32, u32) {
        // Reset for new period.
        world.match_state.clock_s = 0.0;
        world.match_state.phase = MatchPhase::Kickoff;
        world.match_state.phase_timer = 0.0;
        world.match_state.kickoff_ticks = 0;
        world.match_state.kickoff_seen_carrier = false;
        world.match_state.kickoff_circle_lock = true;
        world.match_state.kickoff_suppress_away_team_side = true;
        world.reset_to_kickoff();
        reset_possession_for_kickoff(&mut world.possession);
        world.possession.carrier = None;
        world.ball.held = false;

        for _ in 0..max_ticks {
            let (home_api, away_api) = world.build_apis_masked(home_mask, away_mask);
            let (nn_api, opp_api) = if is_home_nn {
                (&home_api, &away_api)
            } else {
                (&away_api, &home_api)
            };

            let mut cur_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
            for (i, id) in aicomp_soccer_sim::player::PlayerId::ALL.iter().enumerate() {
                cur_opp_pos[i] = nn_api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
            }
            let mut opp_vel = [bevy::prelude::Vec2::ZERO; 4];
            if prev_opp_pos_valid {
                for i in 0..4 {
                    opp_vel[i] = (cur_opp_pos[i] - prev_opp_pos[i]) / FIXED_DT;
                }
            }
            prev_opp_pos = cur_opp_pos;
            prev_opp_pos_valid = true;

            let input = extract_features_raw(nn_api, &opp_vel);
            let act = weights.forward_with_activations(&input);

            let mut noise = [0.0f32; OUTPUT_DIM];
            for i in 0..OUTPUT_DIM {
                noise[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
            }

            let (mut nn_out, nn_actions) = output_to_commands(&act.output, &noise, nn_api, rng);
            let mut opp_out = opp_brain.think(opp_api);

            evaluate_and_apply_with_action(nn_api, params, &mut nn_out, det_cache_nn, &nn_actions);
            evaluate_and_apply(opp_api, params, &mut opp_out, det_cache_opp);

            let (home_out, away_out) = if is_home_nn {
                (&nn_out, &opp_out)
            } else {
                (&opp_out, &nn_out)
            };
            world.step_with_commands(home_out, away_out, FIXED_DT);
            let sh_after = world.match_state.score_home;
            let sa_after = world.match_state.score_away;

            let (cur_us, cur_them) = if is_home_nn {
                (sh_after, sa_after)
            } else {
                (sa_after, sh_after)
            };

            let shot_taken = nn_out.commands.iter().any(|c| c.shoot);
            let reward = compute_reward(nn_api, *prev_us, *prev_them, cur_us, cur_them, false, false, false, false, false, false, shot_taken, false);
            *prev_us = cur_us;
            *prev_them = cur_them;

            let old_log_prob = weights.log_prob(&act.output, &noise, ACTION_SIGMA);
            steps.push(Step {
                activations: act,
                action_noise: noise,
                reward,
                old_log_prob,
            });

            if sh_after.abs_diff(sa_after) >= 7 && (sh_after == 0 || sa_after == 0) {
                break;
            }
        }

        if is_home_nn {
            (world.match_state.score_home, world.match_state.score_away)
        } else {
            (world.match_state.score_away, world.match_state.score_home)
        }
    };

    // Phase 1: Extra time — 4v4, another 90 min.
    let (us, them) = play_period(
        &mut world, weights, opp_brain, is_home_nn, params, rng, steps,
        &mut prev_us, &mut prev_them, &mut det_cache_nn, &mut det_cache_opp,
        home_mask, away_mask,
    );
    if us != them {
        return (us, them);
    }

    // Phases 2-4: Remove 1 random player per team each period.
    // 3v3 → 2v2 → 1v1.
    let mut available_ids: Vec<u8> = vec![1, 2, 3, 4];
    for _ in 0..3 {
        // Pick a random player to remove from each team.
        let remove_idx = rng.gen_range(0..available_ids.len());
        let remove_id = available_ids.remove(remove_idx);

        world.disable_player(TeamId::Home, remove_id);
        world.disable_player(TeamId::Away, remove_id);

        let (us, them) = play_period(
            &mut world, weights, opp_brain, is_home_nn, params, rng, steps,
            &mut prev_us, &mut prev_them, &mut det_cache_nn, &mut det_cache_opp,
            home_mask, away_mask,
        );
        if us != them {
            return (us, them);
        }
    }

    // Phase 5: Still tied at 1v1 — decide by possession stats.
    let home_poss = world.match_state.possession_s_home;
    let away_poss = world.match_state.possession_s_away;
    let (us, them) = if is_home_nn {
        (world.match_state.score_home, world.match_state.score_away)
    } else {
        (world.match_state.score_away, world.match_state.score_home)
    };

    if home_poss != away_poss {
        let home_wins = home_poss > away_poss;
        let nn_wins = if is_home_nn { home_wins } else { !home_wins };
        if nn_wins {
            (us + 1, them)
        } else {
            (us, them + 1)
        }
    } else {
        if rng.gen_bool(0.5) {
            (us + 1, them)
        } else {
            (us, them + 1)
        }
    }
}

/// Self-play match: current weights vs another NN (best or current).
/// Both sides get full NN treatment (aimbot + deterministic layer).
/// Returns trajectory for the current-weights side only.
fn run_self_play(
    weights: &NetWeights,
    opp_weights: &NetWeights,
    nn_side: TeamId,
    kickoff: TeamId,
    params: &SimParams,
    rng: &mut StdRng,
) -> (Vec<Step>, u32, u32) {
    let mut steps = Vec::new();
    let mut world = MatchWorld::new_kickoff_opening(params.clone(), kickoff);
    let is_home_cur = nn_side == TeamId::Home;
    let cur_brain = TrainedBrain::with_weights(weights.clone()).with_team(nn_side);
    let opp_brain = TrainedBrain::with_weights(opp_weights.clone()).with_team(nn_side.other());
    if is_home_cur {
        world.set_kickoff_formation(TeamId::Home, cur_brain.kickoff_formation());
        world.set_kickoff_formation(TeamId::Away, opp_brain.kickoff_formation());
    } else {
        world.set_kickoff_formation(TeamId::Home, opp_brain.kickoff_formation());
        world.set_kickoff_formation(TeamId::Away, cur_brain.kickoff_formation());
    }
    if world.params.kickoff_delay_s < 1.0 {
        world.params.kickoff_delay_s = 4.9;
    }

    let max_ticks = ((MATCH_SECS / FIXED_DT).ceil() as u64).max(1);
    let mut prev_us = 0u32;
    let mut prev_them = 0u32;

    let home_mask = cur_brain.api_mask();
    let away_mask = opp_brain.api_mask();

    let mut det_cache_cur = DeterministicCache::default();
    let mut det_cache_opp = DeterministicCache::default();
    let mut prev_opp_pos_cur = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_cur_valid = false;
    let mut prev_opp_pos_opp = [bevy::prelude::Vec2::ZERO; 4];
    let mut prev_opp_pos_opp_valid = false;

    for _ in 0..max_ticks {
        let (home_api, away_api) = world.build_apis_masked(home_mask.as_ref(), away_mask.as_ref());
        let (cur_api, opp_api) = if is_home_cur {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        // Compute opp velocities for cur side.
        let mut cur_opp_pos = [bevy::prelude::Vec2::ZERO; 4];
        for (i, id) in aicomp_soccer_sim::player::PlayerId::ALL.iter().enumerate() {
            cur_opp_pos[i] = cur_api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
        }
        let mut opp_vel_cur = [bevy::prelude::Vec2::ZERO; 4];
        if prev_opp_pos_cur_valid {
            for i in 0..4 {
                opp_vel_cur[i] = (cur_opp_pos[i] - prev_opp_pos_cur[i]) / FIXED_DT;
            }
        }
        prev_opp_pos_cur = cur_opp_pos;
        prev_opp_pos_cur_valid = true;

        // Compute opp velocities for opp side (from opp's perspective).
        let mut cur_opp_pos_opp = [bevy::prelude::Vec2::ZERO; 4];
        for (i, id) in aicomp_soccer_sim::player::PlayerId::ALL.iter().enumerate() {
            cur_opp_pos_opp[i] = opp_api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
        }
        let mut opp_vel_opp = [bevy::prelude::Vec2::ZERO; 4];
        if prev_opp_pos_opp_valid {
            for i in 0..4 {
                opp_vel_opp[i] = (cur_opp_pos_opp[i] - prev_opp_pos_opp[i]) / FIXED_DT;
            }
        }
        prev_opp_pos_opp = cur_opp_pos_opp;
        prev_opp_pos_opp_valid = true;

        // Current weights side (collect trajectory for training).
        let input_cur = extract_features_raw(cur_api, &opp_vel_cur);
        let act_cur = weights.forward_with_activations(&input_cur);
        let mut noise_cur = [0.0f32; OUTPUT_DIM];
        for i in 0..OUTPUT_DIM {
            noise_cur[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
        }
        let (mut cur_out, cur_actions) = output_to_commands(&act_cur.output, &noise_cur, cur_api, rng);

        // Opponent NN side (no gradient collection).
        let input_opp = extract_features_raw(opp_api, &opp_vel_opp);
        let act_opp = opp_weights.forward_with_activations(&input_opp);
        let (mut opp_out, opp_actions) = output_to_commands(&act_opp.output, &[0.0f32; OUTPUT_DIM], opp_api, rng);

        // Both sides get full aimbot + deterministic treatment.
        evaluate_and_apply_with_action(cur_api, params, &mut cur_out, &mut det_cache_cur, &cur_actions);
        evaluate_and_apply_with_action(opp_api, params, &mut opp_out, &mut det_cache_opp, &opp_actions);

        // Auto-tackle and tackle coordinator for both NN sides.
        auto_tackle(cur_api, &mut cur_out, params);
        auto_tackle(opp_api, &mut opp_out, params);
        coordinate_tackles(cur_api, &mut cur_out, params);
        coordinate_tackles(opp_api, &mut opp_out, params);

        let (home_out, away_out) = if is_home_cur {
            (&cur_out, &opp_out)
        } else {
            (&opp_out, &cur_out)
        };
        world.step_with_commands(home_out, away_out, FIXED_DT);
        let sh_after = world.match_state.score_home;
        let sa_after = world.match_state.score_away;

        let (cur_us, cur_them) = if is_home_cur {
            (sh_after, sa_after)
        } else {
            (sa_after, sh_after)
        };

        let shot_taken = cur_out.commands.iter().any(|c| c.shoot);
        let reward = compute_reward(cur_api, prev_us, prev_them, cur_us, cur_them, false, false, false, false, false, false, shot_taken, false);
        prev_us = cur_us;
        prev_them = cur_them;

        let old_log_prob = weights.log_prob(&act_cur.output, &noise_cur, ACTION_SIGMA);
        steps.push(Step {
            activations: act_cur,
            action_noise: noise_cur,
            reward,
            old_log_prob,
        });

        if sh_after.abs_diff(sa_after) >= 7 && (sh_after == 0 || sa_after == 0) {
            break;
        }
    }

    let (final_us, final_them) = if is_home_cur {
        (world.match_state.score_home, world.match_state.score_away)
    } else {
        (world.match_state.score_away, world.match_state.score_home)
    };

    (steps, final_us, final_them)
}

/// Compute advantages using Generalized Advantage Estimation (GAE).
/// Uses a moving average baseline to reduce variance.
/// GAE: A_t = sum_{l>=0} (gamma*lambda)^l * delta_{t+l}
/// where delta_t = r_t + gamma * V(t+1) - V(t)
/// Without a value network, V(t) is approximated by the moving average baseline.
static mut BASELINE_AVG: f32 = 0.0;

fn compute_gae(steps: &[Step], gamma: f32, lambda: f32) -> Vec<f32> {
    let n = steps.len();
    if n == 0 { return Vec::new(); }

    // Get the current baseline (moving average of returns).
    let baseline = unsafe { BASELINE_AVG };

    // Compute discounted returns for baseline update.
    let mut returns = vec![0.0f32; n];
    let mut running = 0.0;
    for i in (0..n).rev() {
        running = steps[i].reward + gamma * running;
        returns[i] = running;
    }

    // Update baseline with exponential moving average.
    let match_avg_return = returns.iter().sum::<f32>() / n as f32;
    let new_baseline = if baseline == 0.0 {
        match_avg_return
    } else {
        BASELINE_DECAY * baseline + (1.0 - BASELINE_DECAY) * match_avg_return
    };
    unsafe { BASELINE_AVG = new_baseline; }

    // Compute GAE advantages using the baseline as V(t).
    // delta_t = r_t + gamma * V(t+1) - V(t)
    // V(t) = baseline for all t (no value network).
    // A_t = sum_{l>=0} (gamma*lambda)^l * delta_{t+l}
    let mut advantages = vec![0.0f32; n];
    let mut gae = 0.0;
    for i in (0..n).rev() {
        let v_next = if i + 1 < n { baseline } else { 0.0 };
        let delta = steps[i].reward + gamma * v_next - baseline;
        gae = delta + gamma * lambda * gae;
        advantages[i] = gae;
    }

    // Normalize advantages.
    let mean_adv = advantages.iter().sum::<f32>() / n as f32;
    let std_adv = {
        let var = advantages.iter().map(|a| (a - mean_adv).powi(2)).sum::<f32>() / n as f32;
        var.sqrt().max(1e-6)
    };
    for a in &mut advantages {
        *a = (*a - mean_adv) / std_adv;
    }

    advantages
}

/// PPO clipped surrogate update.
/// For each of PPO_EPOCHS passes over the batch:
///   1. Recompute log_prob with current weights
///   2. Compute ratio = exp(new_log_prob - old_log_prob)
///   3. Clipped surrogate: min(ratio * adv, clip(ratio, 1-eps, 1+eps) * adv)
///   4. Backprop and apply gradient
fn ppo_update(
    weights: &mut NetWeights,
    steps: &[Step],
    advantages: &[f32],
    lr: f32,
) {
    let n = steps.len();
    if n == 0 { return; }

    for _epoch in 0..PPO_EPOCHS {
        let mut total_grad = NetGradients::zeros(weights);

        for (i, step) in steps.iter().enumerate() {
            // Recompute forward pass with current weights.
            let new_act = weights.forward_with_activations(&step.activations.input);
            let new_log_prob = weights.log_prob(&new_act.output, &step.action_noise, ACTION_SIGMA);

            // Importance ratio.
            let ratio = (new_log_prob - step.old_log_prob).exp();

            // Clipped surrogate objective.
            let adv = advantages[i];
            let surr1 = ratio * adv;
            let surr2 = ratio.clamp(1.0 - PPO_CLIP_EPS, 1.0 + PPO_CLIP_EPS) * adv;

            // PPO gradient:
            // L = -min(ratio * adv, clip(ratio) * adv)
            // dL/d(theta) = -adv * ratio * d(log_prob)/d(mean)  when surr1 < surr2
            // dL/d(theta) = 0                                   when surr1 >= surr2 (clipped)
            let lp_grad = log_prob_grad(&new_act.output, &step.action_noise);
            let mut grad_out = [0.0f32; OUTPUT_DIM];

            if surr1 < surr2 {
                // Unclipped: full gradient with ratio.
                for j in 0..OUTPUT_DIM {
                    grad_out[j] = -lp_grad[j] * adv * ratio;
                }
            }
            // Clipped: policy gradient is zero.
            // Note: entropy gradient is zero for fixed-sigma Gaussian (H doesn't depend on mean).

            let step_grad = weights.backward(&grad_out, 0.0, &new_act);
            total_grad.add(&step_grad, 1.0 / n as f32);
        }

        // Gradient clipping.
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

        weights.apply_gradient(&total_grad, lr);
    }
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

/// Shared training state for the dashboard server.
#[derive(Clone, serde::Serialize)]
struct TrainingState {
    epoch: usize,
    total_epochs: usize,
    elapsed_ms: u128,
    num_matches: usize,
    completed_matches: usize,
    total_gf: u32,
    total_ga: u32,
    total_reward: f32,
    avg_reward: f32,
    best_diff: f32,
    /// Per-opponent results: (label, gf, ga)
    per_opp: Vec<(String, u32, u32)>,
    self_best: (u32, u32),
    self_pure: (u32, u32),
    /// History of (epoch, total_gf, total_ga, diff, reward)
    history: Vec<(usize, u32, u32, f32, f32)>,
    status: String,
    /// Live match results as they complete: (label, gf, ga)
    match_log: Vec<(String, u32, u32)>,
}

impl Default for TrainingState {
    fn default() -> Self {
        Self {
            epoch: 0, total_epochs: 0, elapsed_ms: 0, num_matches: 0,
            completed_matches: 0,
            total_gf: 0, total_ga: 0, total_reward: 0.0, avg_reward: 0.0,
            best_diff: f32::MIN,
            per_opp: Vec::new(), self_best: (0, 0), self_pure: (0, 0),
            history: Vec::new(), status: "starting...".into(),
            match_log: Vec::new(),
        }
    }
}

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>PPO Training Dashboard</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: #0d1117; color: #c9d1d9; font-family: 'Segoe UI', system-ui, sans-serif; padding: 20px; }
  h1 { color: #58a6ff; margin-bottom: 16px; font-size: 24px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; max-width: 1200px; }
  .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 16px; }
  .card h2 { color: #8b949e; font-size: 14px; text-transform: uppercase; margin-bottom: 12px; }
  .stat { display: inline-block; margin-right: 24px; }
  .stat .val { font-size: 28px; font-weight: bold; }
  .stat .lbl { font-size: 12px; color: #8b949e; }
  .gf { color: #3fb950; } .ga { color: #f85149; } .diff-pos { color: #3fb950; } .diff-neg { color: #f85149; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; color: #8b949e; padding: 6px 8px; border-bottom: 1px solid #30363d; }
  td { padding: 6px 8px; border-bottom: 1px solid #21262d; }
  .opp-name { max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #status { font-size: 14px; color: #8b949e; margin-bottom: 16px; }
  .progress-bar { background: #21262d; border-radius: 4px; height: 8px; margin: 8px 0; overflow: hidden; }
  .progress-fill { background: #58a6ff; height: 100%; transition: width 0.5s; }
  .match-progress { background: #21262d; border-radius: 4px; height: 6px; margin: 4px 0 12px; overflow: hidden; }
  .match-progress-fill { background: #3fb950; height: 100%; transition: width 0.3s; }
  #match-log { max-height: 200px; overflow-y: auto; font-size: 12px; font-family: monospace; display: flex; flex-direction: column-reverse; }
  #match-log div { padding: 2px 0; border-bottom: 1px solid #21262d; }
  canvas { width: 100%; height: 200px; }
  .refresh-note { font-size: 11px; color: #484f58; margin-top: 8px; }
</style>
</head>
<body>
<h1>PPO Soccer Training Dashboard</h1>
<div id="status">Connecting...</div>
<div class="progress-bar"><div class="progress-fill" id="prog" style="width:0%"></div></div>
<div id="match-prog-text" style="font-size:12px;color:#8b949e;margin-bottom:4px">Matches: 0/0</div>
<div class="match-progress"><div class="match-progress-fill" id="match-prog" style="width:0%"></div></div>
<div class="grid">
  <div class="card">
    <h2>Epoch Summary</h2>
    <div class="stat"><div class="val" id="epoch">-</div><div class="lbl">Epoch</div></div>
    <div class="stat"><div class="val gf" id="gf">-</div><div class="lbl">Goals Scored</div></div>
    <div class="stat"><div class="val ga" id="ga">-</div><div class="lbl">Goals Conceded</div></div>
    <div class="stat"><div class="val" id="diff">-</div><div class="lbl">Goal Difference</div></div>
    <div class="stat"><div class="val" id="reward">-</div><div class="lbl">Reward</div></div>
    <div class="stat"><div class="val" id="best">-</div><div class="lbl">Best Difference</div></div>
    <div class="stat"><div class="val" id="time">-</div><div class="lbl">Epoch Time</div></div>
  </div>
  <div class="card">
    <h2>Score History</h2>
    <canvas id="chart"></canvas>
  </div>
  <div class="card" style="grid-column: 1 / -1">
    <h2>Per-Opponent Results (4 matches each)</h2>
    <table>
      <thead><tr><th>Opponent</th><th>Scored</th><th>Conceded</th><th>Difference</th></tr></thead>
      <tbody id="opp-table"></tbody>
    </table>
  </div>
  <div class="card" style="grid-column: 1 / -1">
    <h2>Live Match Log</h2>
    <div id="match-log"></div>
  </div>
</div>
<script>
async function update() {
  try {
    const r = await fetch('/data');
    const d = await r.json();
    document.getElementById('status').textContent = 'Status: ' + d.status;
    document.getElementById('epoch').textContent = d.epoch + '/' + d.total_epochs;
    document.getElementById('gf').textContent = d.total_gf;
    document.getElementById('ga').textContent = d.total_ga;
    const diff = d.total_gf - d.total_ga;
    const diffEl = document.getElementById('diff');
    diffEl.textContent = (diff >= 0 ? '+' : '') + diff;
    diffEl.className = 'val ' + (diff >= 0 ? 'diff-pos' : 'diff-neg');
    document.getElementById('reward').textContent = d.total_reward.toFixed(1);
    document.getElementById('best').textContent = d.best_diff == -99999 ? '-' : (d.best_diff >= 0 ? '+' : '') + d.best_diff.toFixed(0);
    document.getElementById('time').textContent = (d.elapsed_ms / 1000).toFixed(1) + 's';
    document.getElementById('prog').style.width = (d.total_epochs > 0 ? (d.epoch / d.total_epochs * 100) : 0) + '%';
    const mp = d.num_matches > 0 ? d.completed_matches / d.num_matches : 0;
    document.getElementById('match-prog').style.width = (mp * 100) + '%';
    document.getElementById('match-prog-text').textContent = 'Matches: ' + d.completed_matches + '/' + d.num_matches;
    let html = '';
    for (const [name, gf, ga] of d.per_opp) {
      const short = name.replace(/^graph:.*[\\\/]/, '').replace(/\.txt$/, '');
      const dd = gf - ga;
      html += '<tr><td class="opp-name">' + short + '</td><td>' + gf + '</td><td>' + ga + '</td><td style="color:' + (dd >= 0 ? '#3fb950' : '#f85149') + '">' + (dd >= 0 ? '+' : '') + dd + '</td></tr>';
    }
    const sb = d.self_best, sp = d.self_pure;
    html += '<tr style="border-top:2px solid #30363d"><td>SELF (vs best)</td><td>'+sb[0]+'</td><td>'+sb[1]+'</td><td style="color:' + (sb[0]-sb[1] >= 0 ? '#3fb950' : '#f85149') + '">' + (sb[0]-sb[1] >= 0 ? '+' : '') + (sb[0]-sb[1]) + '</td></tr>';
    html += '<tr><td>SELF (pure)</td><td>'+sp[0]+'</td><td>'+sp[1]+'</td><td style="color:' + (sp[0]-sp[1] >= 0 ? '#3fb950' : '#f85149') + '">' + (sp[0]-sp[1] >= 0 ? '+' : '') + (sp[0]-sp[1]) + '</td></tr>';
    document.getElementById('opp-table').innerHTML = html;
    let logHtml = '';
    for (const [name, gf, ga] of d.match_log) {
      const short = name.replace(/^graph:.*[\\\\/]/, '').replace(/\.txt$/, '');
      const dd = gf - ga;
      logHtml += '<div><span style="color:#8b949e">' + short + '</span> &mdash; <span style="color:#3fb950">' + gf + '</span> : <span style="color:#f85149">' + ga + '</span> (' + (dd >= 0 ? '+' : '') + dd + ')</div>';
    }
    document.getElementById('match-log').innerHTML = logHtml;
    drawChart(d.history);
  } catch(e) { document.getElementById('status').textContent = 'Error: ' + e.message; }
}
function drawChart(hist) {
  const c = document.getElementById('chart');
  const ctx = c.getContext('2d');
  c.width = c.offsetWidth; c.height = 200;
  ctx.clearRect(0, 0, c.width, c.height);
  if (hist.length < 2) return;
  const maxAbs = Math.max(...hist.map(h => Math.abs(h[3])), 10);
  const w = c.width, h = c.height, pad = 30;
  ctx.strokeStyle = '#30363d'; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(pad, h/2); ctx.lineTo(w-pad, h/2); ctx.stroke();
  ctx.strokeStyle = '#58a6ff'; ctx.lineWidth = 2;
  ctx.beginPath();
  hist.forEach((e, i) => {
    const x = pad + (w - 2*pad) * i / (hist.length - 1);
    const y = h/2 - (e[3] / maxAbs) * (h/2 - 10);
    if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
  });
  ctx.stroke();
  ctx.fillStyle = '#8b949e'; ctx.font = '10px sans-serif';
  ctx.fillText('+' + maxAbs, 2, 12); ctx.fillText('-' + maxAbs, 2, h-2);
  ctx.fillText('0', 2, h/2);
}
update();
setInterval(update, 2000);
</script>
</body>
</html>"#;

fn start_dashboard_server(state: Arc<Mutex<TrainingState>>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind("127.0.0.1:7878") {
            Ok(l) => l,
            Err(e) => { eprintln!("dashboard server failed: {e}"); return; }
        };
        eprintln!("dashboard: http://127.0.0.1:7878");
        for stream in listener.incoming() {
            let mut stream = match stream { Ok(s) => s, Err(_) => continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let (status, content_type, body) = if req.contains("GET /data") {
                let s = state.lock().unwrap();
                let json = serde_json::to_string(&*s).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
                ("200 OK", "application/json", json)
            } else {
                ("200 OK", "text/html; charset=utf-8", DASHBOARD_HTML.to_string())
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
}

fn main() {
    // Debug: verify main() is reached.
    println!("ppo_train starting...");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let matches_per_epoch: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(128);
    let mut lr: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.001);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);

    eprintln!("=== PPO Training ===");
    eprintln!("epochs: {epochs}, matches/epoch: {matches_per_epoch}, lr: {lr}, seed: {seed}");
    eprintln!("match_secs: {MATCH_SECS}, gamma: {GAMMA}, sigma: {ACTION_SIGMA}, clip: {PPO_CLIP_EPS}, ppo_epochs: {PPO_EPOCHS}, gae_lambda: {GAE_LAMBDA}");

    let params = SimParams::default();
    let cache = Arc::new(ProgramCache::default());
    let opponents = discover_opponents();
    eprintln!("opponents found: {}", opponents.len());
    for opp in &opponents {
        eprintln!("  - {}", opp.label());
    }

    let mut rng = StdRng::seed_from_u64(seed);

    let mut weights = NetWeights::load_from_path("assets/ppo_weights.json")
        .or_else(|| NetWeights::load_from_path("assets/ppo_best.json"))
        .or_else(|| NetWeights::load_from_path("assets/es_best.json"))
        .unwrap_or_else(|| NetWeights::random(INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM));

    let weights_path = "assets/ppo_weights.json";
    let best_path = "assets/ppo_best.json";

    let mut best_score_diff: f32 = f32::MIN;

    let mut best_weights = NetWeights::load_from_path(best_path)
        .unwrap_or_else(|| weights.clone());

    // Start dashboard server.
    let dash_state = Arc::new(Mutex::new(TrainingState {
        total_epochs: epochs,
        status: "initializing...".into(),
        ..Default::default()
    }));
    start_dashboard_server(dash_state.clone());

    for epoch in 0..epochs {
        let t0 = Instant::now();

        // Graceful shutdown: if stop_training file exists, save and exit.
        if std::path::Path::new("stop_training").exists() {
            let _ = std::fs::remove_file("stop_training");
            weights.save_to_path(best_path).ok();
            weights.save_to_path(weights_path).ok();
            eprintln!("=== Graceful shutdown requested. Saved weights. ===");
            break;
        }

        // Round-robin shuffled matchmaking: build a pool of ALL opponents,
        // shuffle it, then deal matches by cycling through the pool. Every
        // opponent is faced before any repeats — like a tournament bracket
        // with randomized sides each round.
        #[derive(Clone, Copy)]
        enum MatchKind { Graph(usize), SelfBest, SelfPure, Chase, Idle }

        // Build the full opponent pool: 1 entry per graph opponent,
        // plus chase, idle, self-best, self-pure.
        let mut pool: Vec<MatchKind> = Vec::new();
        for i in 0..opponents.len() {
            pool.push(MatchKind::Graph(i));
        }
        pool.push(MatchKind::Chase);
        pool.push(MatchKind::Idle);
        pool.push(MatchKind::SelfBest);
        pool.push(MatchKind::SelfPure);

        // Shuffle the pool for this epoch.
        pool.shuffle(&mut rng);

        let mut match_configs: Vec<(MatchKind, TeamId, TeamId, u64)> = Vec::new();
        for m in 0..matches_per_epoch {
            // Cycle through the shuffled pool — wraps around so every opponent
            // is played before any repeats.
            let kind = pool[m % pool.len()];
            let nn_side = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
            let kickoff = if rng.gen_bool(0.5) { TeamId::Home } else { TeamId::Away };
            match_configs.push((kind, nn_side, kickoff, rng.gen::<u64>()));
        }

        let num_matches = match_configs.len();

        // Update dashboard: matches running.
        let completed_counter = Arc::new(AtomicUsize::new(0));
        let match_log: Arc<Mutex<Vec<(String, u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let mut ds = dash_state.lock().unwrap();
            ds.status = format!("Running {} matches (epoch {})...", num_matches, epoch + 1);
            ds.completed_matches = 0;
            ds.num_matches = num_matches;
            ds.match_log.clear();
        }

        // Run all matches in parallel using rayon.
        let weights_clone = weights.clone();
        let best_weights_clone = best_weights.clone();
        let params_clone = params.clone();
        let opponents_ref = &opponents;
        let cache_ref = &cache;
        let dash_ref = &dash_state;
        let counter_ref = &completed_counter;
        let log_ref = &match_log;

        let match_results: Vec<(MatchKind, TeamId, TeamId, Vec<Step>, u32, u32)> = match_configs
            .par_iter()
            .map(|&(kind, nn_side, kickoff, match_seed)| {
                let mut match_rng = StdRng::seed_from_u64(match_seed);
                let (steps, gf, ga) = match kind {
                    MatchKind::Graph(opp_idx) => {
                        let mut opp = match build_brain(&opponents_ref[opp_idx], GraphEngine::Runtime, Some(cache_ref)) {
                            Ok(b) => b,
                            Err(_) => return (kind, nn_side, kickoff, Vec::new(), 0, 0),
                        };
                        run_trajectory(&weights_clone, &mut *opp, nn_side, kickoff, &params_clone, &mut match_rng, None)
                    }
                    MatchKind::Chase => {
                        let mut opp: Box<dyn TeamBrain> = Box::new(ChaseBallBrain::default());
                        run_trajectory(&weights_clone, &mut *opp, nn_side, kickoff, &params_clone, &mut match_rng, None)
                    }
                    MatchKind::Idle => {
                        let mut opp: Box<dyn TeamBrain> = Box::new(IdleBrain);
                        run_trajectory(&weights_clone, &mut *opp, nn_side, kickoff, &params_clone, &mut match_rng, None)
                    }
                    MatchKind::SelfBest => {
                        run_self_play(&weights_clone, &best_weights_clone, nn_side, kickoff, &params_clone, &mut match_rng)
                    }
                    MatchKind::SelfPure => {
                        run_self_play(&weights_clone, &weights_clone, nn_side, kickoff, &params_clone, &mut match_rng)
                    }
                };

                // Report progress to dashboard.
                let label = match kind {
                    MatchKind::Graph(idx) => opponents_ref[idx].label(),
                    MatchKind::Chase => "chase".to_string(),
                    MatchKind::Idle => "idle".to_string(),
                    MatchKind::SelfBest => "SELF(best)".to_string(),
                    MatchKind::SelfPure => "SELF(pure)".to_string(),
                };
                {
                    let mut log = log_ref.lock().unwrap();
                    log.push((label, gf, ga));
                }
                let completed = counter_ref.fetch_add(1, Ordering::Relaxed) + 1;
                {
                    let mut ds = dash_ref.lock().unwrap();
                    ds.completed_matches = completed;
                    ds.match_log = log_ref.lock().unwrap().clone();
                }

                (kind, nn_side, kickoff, steps, gf, ga)
            })
            .collect();

        // Sequential gradient updates + per-opponent stats.
        let mut total_gf = 0u32;
        let mut total_ga = 0u32;
        let mut total_reward = 0.0f32;
        let mut total_steps = 0usize;

        // Per-opponent aggregate scores: (gf, ga) keyed by opponent label.
        let mut per_opp: std::collections::BTreeMap<String, (u32, u32)> = std::collections::BTreeMap::new();
        let mut self_best_score = (0u32, 0u32);
        let mut self_pure_score = (0u32, 0u32);

        // Collect ALL steps from ALL matches into one batch for a single PPO update.
        // Compute advantages per-match to avoid cross-match credit contamination.
        let mut all_steps: Vec<Step> = Vec::new();
        let mut all_advantages: Vec<f32> = Vec::new();
        for (kind, _nn_side, _kickoff, steps, gf, ga) in &match_results {
            total_gf += *gf;
            total_ga += *ga;
            let traj_reward: f32 = steps.iter().map(|s| s.reward).sum();
            total_reward += traj_reward;
            total_steps += steps.len();

            match kind {
                MatchKind::Graph(opp_idx) => {
                    let label = opponents_ref[*opp_idx].label();
                    let entry = per_opp.entry(label.to_string()).or_insert((0, 0));
                    entry.0 += *gf;
                    entry.1 += *ga;
                }
                MatchKind::Chase => {
                    let entry = per_opp.entry("chase".to_string()).or_insert((0, 0));
                    entry.0 += *gf;
                    entry.1 += *ga;
                }
                MatchKind::Idle => {
                    let entry = per_opp.entry("idle".to_string()).or_insert((0, 0));
                    entry.0 += *gf;
                    entry.1 += *ga;
                }
                MatchKind::SelfBest => {
                    self_best_score.0 += *gf;
                    self_best_score.1 += *ga;
                }
                MatchKind::SelfPure => {
                    self_pure_score.0 += *gf;
                    self_pure_score.1 += *ga;
                }
            }

            // Subsample if needed, then compute per-match advantages.
            let train_steps: Vec<Step> = if steps.len() > MAX_TRAIN_STEPS {
                let stride = steps.len() / MAX_TRAIN_STEPS;
                steps.iter().step_by(stride).take(MAX_TRAIN_STEPS).cloned().collect()
            } else {
                steps.clone()
            };
            let match_advantages = compute_gae(&train_steps, GAMMA, GAE_LAMBDA);
            all_steps.extend(train_steps);
            all_advantages.extend(match_advantages);
        }

        // Single PPO update on the full batch with per-match normalized advantages.
        ppo_update(&mut weights, &all_steps, &all_advantages, lr);

        // Learning rate decay.
        lr = lr * LR_DECAY;

        let diff = total_gf as f32 - total_ga as f32;
        let elapsed = t0.elapsed().as_millis();

        // Update dashboard state.
        {
            let mut ds = dash_state.lock().unwrap();
            ds.epoch = epoch + 1;
            ds.elapsed_ms = elapsed;
            ds.num_matches = num_matches;
            ds.total_gf = total_gf;
            ds.total_ga = total_ga;
            ds.total_reward = total_reward;
            ds.avg_reward = if total_steps > 0 { total_reward / total_steps as f32 } else { 0.0 };
            ds.best_diff = best_score_diff;
            ds.per_opp = per_opp.iter().map(|(l, (gf, ga))| (l.clone(), *gf, *ga)).collect();
            ds.self_best = self_best_score;
            ds.self_pure = self_pure_score;
            ds.history.push((epoch, total_gf, total_ga, diff, total_reward));
            ds.status = format!("epoch {} done, training...", epoch + 1);
        }

        // Print per-opponent breakdown.
        eprintln!("\n=== Epoch {epoch} | {num_matches} matches | {elapsed}ms ===");
        for (label, (gf, ga)) in &per_opp {
            let d = *gf as i32 - *ga as i32;
            eprintln!("  vs {label:<30} GF{gf:>2} GA{ga:>2} diff={d:+3}");
        }
        let sb_d = self_best_score.0 as i32 - self_best_score.1 as i32;
        eprintln!("  vs SELF(best)                    GF{:>2} GA{:>2} diff={:+3}", self_best_score.0, self_best_score.1, sb_d);
        let sp_d = self_pure_score.0 as i32 - self_pure_score.1 as i32;
        eprintln!("  vs SELF(pure)                    GF{:>2} GA{:>2} diff={:+3}", self_pure_score.0, self_pure_score.1, sp_d);
        eprintln!("  TOTAL: GF{total_gf} GA{total_ga} diff={diff:+.0} reward={total_reward:.1} avg_r={:.4}",
            if total_steps > 0 { total_reward / total_steps as f32 } else { 0.0 });

        if diff > best_score_diff {
            best_score_diff = diff;
            weights.save_to_path(best_path).ok();
            best_weights = weights.clone();
            eprintln!("  >>> NEW BEST! saved to {best_path}");
        }

        // Anti-collapse: if policy degrades too far from best, restore best weights.
        if diff < best_score_diff - RESTORE_THRESHOLD && best_score_diff > f32::MIN + 1.0 {
            eprintln!("  !!! Policy collapsed (diff={diff:.0}, best={best_score_diff:.0}). Restoring best weights.");
            weights = best_weights.clone();
            lr = (lr / LR_DECAY).max(0.0001); // Reset LR to avoid further collapse
        }

        weights.save_to_path(weights_path).ok();
    }

    weights.save_to_path(weights_path).ok();
    eprintln!("saved final weights to {weights_path}");
}
