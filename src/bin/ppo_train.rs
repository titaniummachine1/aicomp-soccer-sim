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
};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::possession::reset_possession_for_kickoff;
use aicomp_soccer_sim::deterministic::{
    evaluate_and_apply, evaluate_and_apply_with_action, DeterministicCache, AssistAction,
};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

const NORM: f32 = 20.0;
const ACTION_SIGMA: f32 = 0.3;
const GAMMA: f32 = 0.99;
const TIME_SCALE: f32 = 20.0;
const INGAME_MATCH_MIN: f32 = 90.0;
/// 90 in-game minutes at 20x = 270 sim seconds
const MATCH_SECS: f32 = INGAME_MATCH_MIN * 60.0 / TIME_SCALE;

struct Step {
    activations: Activations,
    action_noise: [f32; OUTPUT_DIM],
    reward: f32,
}

fn extract_features_raw(api: &aicomp_soccer_sim::api::TeamApi) -> [f32; INPUT_DIM] {
    let ball = api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
    let ball_vel = api.get_vector3("Ball Velocity").flatten().unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let team_goal = api.get_transform("Team Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let team_has = api.get_bool("Team Has Ball").unwrap_or(false) as u32 as f32;
    let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false) as u32 as f32;
    let is_loose = api.get_bool("Is Ball Loose").unwrap_or(false) as u32 as f32;
    let charge = api.get_float("Ball Carrier Shot Charge").unwrap_or(0.0) / 1.0;
    let stam = api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;

    use aicomp_soccer_sim::player::PlayerId;
    let mut team_pos = [bevy::prelude::Vec2::ZERO; 4];
    let mut opp_pos = [bevy::prelude::Vec2::ZERO; 4];
    for (i, id) in PlayerId::ALL.iter().enumerate() {
        team_pos[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
        opp_pos[i] = api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
    }

    let mut dist_opp = [0.0f32; 16];
    for pi in 0..4 {
        for oi in 0..4 {
            dist_opp[pi * 4 + oi] = api
                .get_float(&format!("Distance from Team Player {} to Opponent {}", pi + 1, oi + 1))
                .unwrap_or(0.0) / NORM;
        }
    }

    let mut dist_mate = [0.0f32; 12];
    let mate_pairs = [(0,1),(0,2),(0,3),(1,0),(1,2),(1,3),(2,0),(2,1),(2,3),(3,0),(3,1),(3,2)];
    for (k, (pi, mi)) in mate_pairs.iter().enumerate() {
        dist_mate[k] = api
            .get_float(&format!("Distance from Team Player {} to Teammate {}", pi + 1, mi + 1))
            .unwrap_or(0.0) / NORM;
    }

    let mut pass_dir = [0.0f32; 8];
    let mut can_pass = [0.0f32; 4];
    for i in 0..4 {
        let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", i + 1)).flatten();
        if let Some(d) = pd {
            pass_dir[i * 2] = d.x;
            pass_dir[i * 2 + 1] = d.y;
        }
        can_pass[i] = api.get_bool(&format!("Can Pass to Teammate {}", i + 1)).unwrap_or(false) as u32 as f32;
    }

    // Per-player ball possession (who is the carrier?).
    let mut team_has_ball = [0.0f32; 4];
    let mut opp_has_ball = [0.0f32; 4];
    for i in 0..4 {
        team_has_ball[i] = api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false) as u32 as f32;
        opp_has_ball[i] = api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) as u32 as f32;
    }

    // Per-player stamina and shot charge.
    let mut team_stam = [0.0f32; 4];
    let mut team_charge = [0.0f32; 4];
    for i in 0..4 {
        team_stam[i] = api.get_float(&format!("Team Player {} Stamina", i + 1)).unwrap_or(0.0) / 100.0;
        team_charge[i] = api.get_float(&format!("Teammate {} Shot Charge", i + 1)).unwrap_or(0.0) / 1.0;
    }

    let mut input = [0.0f32; INPUT_DIM];
    let mut o = 0usize;
    input[o..o+2].copy_from_slice(&[ball.x / NORM, ball.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[ball_vel.x / NORM, ball_vel.y / NORM]); o += 2;
    for i in 0..4 { input[o..o+2].copy_from_slice(&[team_pos[i].x / NORM, team_pos[i].y / NORM]); o += 2; }
    for i in 0..4 { input[o..o+2].copy_from_slice(&[opp_pos[i].x / NORM, opp_pos[i].y / NORM]); o += 2; }
    input[o..o+16].copy_from_slice(&dist_opp); o += 16;
    input[o..o+12].copy_from_slice(&dist_mate); o += 12;
    input[o..o+8].copy_from_slice(&pass_dir); o += 8;
    input[o..o+4].copy_from_slice(&can_pass); o += 4;
    input[o..o+3].copy_from_slice(&[team_has, opp_has, is_loose]); o += 3;
    input[o..o+2].copy_from_slice(&[opp_goal.x / NORM, opp_goal.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[team_goal.x / NORM, team_goal.y / NORM]); o += 2;
    input[o] = charge; o += 1;
    input[o] = stam; o += 1;
    input[o..o+4].copy_from_slice(&team_has_ball); o += 4;
    input[o..o+4].copy_from_slice(&opp_has_ball); o += 4;
    input[o..o+4].copy_from_slice(&team_stam); o += 4;
    input[o..o+4].copy_from_slice(&team_charge); o += 4;

    input
}

fn compute_reward(
    api: &aicomp_soccer_sim::api::TeamApi,
    prev_score_us: u32,
    prev_score_them: u32,
    cur_score_us: u32,
    cur_score_them: u32,
) -> f32 {
    let mut r = 0.0;

    if cur_score_us > prev_score_us {
        r += 5.0;
    }
    if cur_score_them > prev_score_them {
        r -= 0.5;
    }

    let ball = api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);
    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let team_goal = api.get_transform("Team Goal Center").unwrap_or(bevy::prelude::Vec2::ZERO);
    let goal_dist = (opp_goal - ball).length();
    let own_goal_dist = (team_goal - ball).length();
    r += 0.2 * (own_goal_dist - goal_dist) / (own_goal_dist + goal_dist + 0.001);

    r
}

/// Decode NN output + noise into BrainOutput and per-player actions.
/// Per player (11 values): move_x, move_y, sprint, 8 action logits.
/// Actions: [none, shoot_left, shoot_center, shoot_right, pass_1, pass_2, pass_3, pass_4]
/// Returns (BrainOutput, [AssistAction; 4]).
fn output_to_commands(
    output: &[f32],
    noise: &[f32; OUTPUT_DIM],
    api: &aicomp_soccer_sim::api::TeamApi,
    rng: &mut StdRng,
) -> (BrainOutput, [AssistAction; 4]) {
    use aicomp_soccer_sim::player::PlayerId;
    let mut out = BrainOutput::default();
    let mut actions = [AssistAction::None; 4];
    let mut players = [bevy::prelude::Vec2::ZERO; 4];
    for (i, id) in PlayerId::ALL.iter().enumerate() {
        players[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(bevy::prelude::Vec2::ZERO);
    }

    for i in 0..4 {
        let base = i * 11;
        let dx = (output[base] + noise[base]).clamp(-1.0, 1.0);
        let dy = (output[base + 1] + noise[base + 1]).clamp(-1.0, 1.0);
        let sprint_sig = output[base + 2] + noise[base + 2];

        let dir = bevy::prelude::Vec2::new(dx, dy);
        let move_to = players[i] + dir * NORM;

        // Sample action from softmax of logits (with noise) for exploration.
        let mut logits = [0.0f32; 8];
        for a in 0..8 {
            logits[a] = output[base + 3 + a] + noise[base + 3 + a];
        }
        let max_logit = logits.iter().cloned().fold(f32::MIN, f32::max);
        let mut exp_vals = [0.0f32; 8];
        let mut sum_exp = 0.0;
        for a in 0..8 {
            exp_vals[a] = (logits[a] - max_logit).exp();
            sum_exp += exp_vals[a];
        }
        let sample = rng.gen_range(0.0..sum_exp);
        let mut cum = 0.0;
        let mut best_action = 0usize;
        for a in 0..8 {
            cum += exp_vals[a];
            if sample <= cum {
                best_action = a;
                break;
            }
        }
        actions[i] = AssistAction::from_index(best_action);

        // Action 0 = none (no interact). Actions 1-7 = interact (shoot/pass).
        // The aimbot will handle aiming if it's a shoot/pass action.
        let interact = best_action > 0;

        out.commands[i] = BrainCommand {
            move_to,
            sprint: sprint_sig > 0.0,
            interact,
        };
    }
    (out, actions)
}

fn log_prob_grad(mean: &[f32], noise: &[f32; OUTPUT_DIM]) -> [f32; OUTPUT_DIM] {
    let mut grad = [0.0f32; OUTPUT_DIM];
    for i in 0..OUTPUT_DIM {
        grad[i] = noise[i] / (ACTION_SIGMA * ACTION_SIGMA);
    }
    grad
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
    let mut auto_pickup_active = false;
    let mut auto_pickup_slot: Option<usize> = None;

    let home_mask = nn_brain.api_mask();
    let away_mask = opp_brain.api_mask();

    let mut det_cache_nn = DeterministicCache::default();
    let mut det_cache_opp = DeterministicCache::default();

    for _ in 0..max_ticks {
        let (home_api, away_api) = world.build_apis_masked(home_mask.as_ref(), away_mask.as_ref());
        let (nn_api, opp_api) = if is_home_nn {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        let input = extract_features_raw(nn_api);
        let act = weights.forward_with_activations(&input);

        let mut noise = [0.0f32; OUTPUT_DIM];
        for i in 0..OUTPUT_DIM {
            noise[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
        }

        let (mut nn_out, nn_actions) = output_to_commands(&act.output, &noise, nn_api, rng);
        let mut opp_out = opp_brain.think(opp_api);

        // NN side: guaranteed score detection (always on) + NN-invoked aimbot.
        evaluate_and_apply_with_action(nn_api, params, &mut nn_out, &mut det_cache_nn, nn_actions);
        // Opponent side: just guaranteed score detection.
        evaluate_and_apply(opp_api, params, &mut opp_out, &mut det_cache_opp);

        // Auto-pickup macro: if ball is loose, force interact on the closest
        // NN player within pickup range — unless the NN already chose to
        // interact with a different player (NN's choice takes priority).
        // Pulses every other tick: if macro fired last tick, force release
        // this tick so the interact latch resets and can fire again.
        let is_loose = nn_api.get_bool("Is Ball Loose").unwrap_or(false);
        if auto_pickup_active {
            // Last tick the macro fired interact. If the ball is still loose,
            // the pickup failed — release this tick so the latch resets for
            // a fresh attempt next tick. If the ball is no longer loose,
            // the pickup succeeded — don't interfere, let NN/det layer control.
            if is_loose {
                if let Some(idx) = auto_pickup_slot {
                    nn_out.commands[idx].interact = false;
                }
            }
            auto_pickup_active = false;
            auto_pickup_slot = None;
        } else if is_loose {
            let interact_r = nn_api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = nn_api.get_transform("Ball").unwrap_or(bevy::prelude::Vec2::ZERO);

            // Check if NN already set interact=true on any player.
            let nn_interacting: Vec<usize> = (0..4)
                .filter(|&i| nn_out.commands[i].interact)
                .collect();

            if nn_interacting.is_empty() {
                // NN didn't choose anyone to interact — find closest player to ball.
                let mut best_idx = None;
                let mut best_dist = f32::MAX;
                for i in 0..4 {
                    // Skip players who already have the ball.
                    let has = nn_api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
                    if has { continue; }
                    let p = nn_api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(bevy::prelude::Vec2::ZERO);
                    let d = (p - ball).length();
                    if d < best_dist {
                        best_dist = d;
                        best_idx = Some(i);
                    }
                }
                // Only auto-pickup if within interact range.
                if let Some(idx) = best_idx {
                    if best_dist <= interact_r {
                        nn_out.commands[idx].interact = true;
                        auto_pickup_active = true;
                        auto_pickup_slot = Some(idx);
                    }
                }
            }
        }

        let sh_before = world.match_state.score_home;
        let sa_before = world.match_state.score_away;
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

        let reward = compute_reward(nn_api, prev_us, prev_them, cur_us, cur_them);
        prev_us = cur_us;
        prev_them = cur_them;

        steps.push(Step {
            activations: act,
            action_noise: noise,
            reward,
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

    // Helper: play one extra period.
    let play_period = |world: &mut MatchWorld,
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

            let input = extract_features_raw(nn_api);
            let act = weights.forward_with_activations(&input);

            let mut noise = [0.0f32; OUTPUT_DIM];
            for i in 0..OUTPUT_DIM {
                noise[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
            }

            let (mut nn_out, nn_actions) = output_to_commands(&act.output, &noise, nn_api, rng);
            let mut opp_out = opp_brain.think(opp_api);

            evaluate_and_apply_with_action(nn_api, params, &mut nn_out, det_cache_nn, nn_actions);
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

            let reward = compute_reward(nn_api, *prev_us, *prev_them, cur_us, cur_them);
            *prev_us = cur_us;
            *prev_them = cur_them;

            steps.push(Step {
                activations: act,
                action_noise: noise,
                reward,
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

    for _ in 0..max_ticks {
        let (home_api, away_api) = world.build_apis_masked(home_mask.as_ref(), away_mask.as_ref());
        let (cur_api, opp_api) = if is_home_cur {
            (&home_api, &away_api)
        } else {
            (&away_api, &home_api)
        };

        // Current weights side (collect trajectory for training).
        let input_cur = extract_features_raw(cur_api);
        let act_cur = weights.forward_with_activations(&input_cur);
        let mut noise_cur = [0.0f32; OUTPUT_DIM];
        for i in 0..OUTPUT_DIM {
            noise_cur[i] = rng.gen_range(-ACTION_SIGMA..ACTION_SIGMA);
        }
        let (mut cur_out, cur_actions) = output_to_commands(&act_cur.output, &noise_cur, cur_api, rng);

        // Opponent NN side (no gradient collection).
        let input_opp = extract_features_raw(opp_api);
        let act_opp = opp_weights.forward_with_activations(&input_opp);
        let (mut opp_out, opp_actions) = output_to_commands(&act_opp.output, &[0.0f32; OUTPUT_DIM], opp_api, rng);

        // Both sides get full aimbot + deterministic treatment.
        evaluate_and_apply_with_action(cur_api, params, &mut cur_out, &mut det_cache_cur, cur_actions);
        evaluate_and_apply_with_action(opp_api, params, &mut opp_out, &mut det_cache_opp, opp_actions);

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

        let reward = compute_reward(cur_api, prev_us, prev_them, cur_us, cur_them);
        prev_us = cur_us;
        prev_them = cur_them;

        steps.push(Step {
            activations: act_cur,
            action_noise: noise_cur,
            reward,
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

fn compute_returns(steps: &[Step]) -> Vec<f32> {
    let n = steps.len();
    let mut returns = vec![0.0f32; n];
    let mut running = 0.0;
    for i in (0..n).rev() {
        running = steps[i].reward + GAMMA * running;
        returns[i] = running;
    }
    returns
}

fn reinforce_update(
    weights: &mut NetWeights,
    steps: &[Step],
    returns: &[f32],
    lr: f32,
) {
    let n = steps.len();
    if n == 0 { return; }

    let mean_return: f32 = returns.iter().sum::<f32>() / n as f32;
    let std_return: f32 = {
        let var: f32 = returns.iter().map(|r| (r - mean_return).powi(2)).sum::<f32>() / n as f32;
        var.sqrt().max(1e-6)
    };

    let mut total_grad = NetGradients::zeros(weights);
    for (i, step) in steps.iter().enumerate() {
        let advantage = (returns[i] - mean_return) / std_return;
        let lp_grad = log_prob_grad(&step.activations.output, &step.action_noise);
        let mut grad_out = [0.0f32; OUTPUT_DIM];
        for j in 0..OUTPUT_DIM {
            grad_out[j] = -lp_grad[j] * advantage;
        }
        let step_grad = weights.backward(&grad_out, &step.activations);
        total_grad.add(&step_grad, 1.0 / n as f32);
    }

    weights.apply_gradient(&total_grad, lr);
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
    let args: Vec<String> = std::env::args().collect();
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let matches_per_epoch: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let lr: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.001);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);

    eprintln!("=== REINFORCE Training ===");
    eprintln!("epochs: {epochs}, matches/epoch: {matches_per_epoch}, lr: {lr}, seed: {seed}");
    eprintln!("match_secs: {MATCH_SECS}, gamma: {GAMMA}, sigma: {ACTION_SIGMA}");

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

        // Curriculum: idle is the only truly easy opponent (stands still).
        // Chase is actually very strong (244 goals vs random NN). Graph opponents
        // vary in strength. Start with idle + self-play, then add graph opponents,
        // then add chase last.
        let curriculum_phase = if epoch < 20 {
            "idle_only"
        } else if epoch < 50 {
            "idle_plus_graph"
        } else if epoch < 80 {
            "graph_plus_chase"
        } else {
            "full"
        };

        // Match types: graph opponents, self-play vs best, pure self-play vs current.
        // Each type: 4 variants (NN home/away × kickoff home/away).
        #[derive(Clone, Copy)]
        enum MatchKind { Graph(usize), SelfBest, SelfPure, Chase, Idle }

        let mut match_configs: Vec<(MatchKind, TeamId, TeamId, u64)> = Vec::new();

        match curriculum_phase {
            "idle_only" => {
                // Idle stands still — NN can learn to score and get positive reward.
                for nn_side in [TeamId::Home, TeamId::Away] {
                    for kickoff in [TeamId::Home, TeamId::Away] {
                        for _ in 0..matches_per_epoch {
                            match_configs.push((MatchKind::Idle, nn_side, kickoff, rng.gen::<u64>()));
                        }
                    }
                }
            }
            "idle_plus_graph" => {
                // Idle + all graph opponents (but not chase yet).
                for nn_side in [TeamId::Home, TeamId::Away] {
                    for kickoff in [TeamId::Home, TeamId::Away] {
                        match_configs.push((MatchKind::Idle, nn_side, kickoff, rng.gen::<u64>()));
                    }
                }
                for opp_idx in 0..opponents.len() {
                    for nn_side in [TeamId::Home, TeamId::Away] {
                        for kickoff in [TeamId::Home, TeamId::Away] {
                            match_configs.push((MatchKind::Graph(opp_idx), nn_side, kickoff, rng.gen::<u64>()));
                        }
                    }
                }
            }
            "graph_plus_chase" => {
                // All graph opponents + chase.
                for opp_idx in 0..opponents.len() {
                    for nn_side in [TeamId::Home, TeamId::Away] {
                        for kickoff in [TeamId::Home, TeamId::Away] {
                            match_configs.push((MatchKind::Graph(opp_idx), nn_side, kickoff, rng.gen::<u64>()));
                        }
                    }
                }
                for nn_side in [TeamId::Home, TeamId::Away] {
                    for kickoff in [TeamId::Home, TeamId::Away] {
                        match_configs.push((MatchKind::Chase, nn_side, kickoff, rng.gen::<u64>()));
                    }
                }
            }
            _ => {
                // Full opponent pool + chase.
                for opp_idx in 0..opponents.len() {
                    for nn_side in [TeamId::Home, TeamId::Away] {
                        for kickoff in [TeamId::Home, TeamId::Away] {
                            match_configs.push((MatchKind::Graph(opp_idx), nn_side, kickoff, rng.gen::<u64>()));
                        }
                    }
                }
                for nn_side in [TeamId::Home, TeamId::Away] {
                    for kickoff in [TeamId::Home, TeamId::Away] {
                        match_configs.push((MatchKind::Chase, nn_side, kickoff, rng.gen::<u64>()));
                    }
                }
            }
        }
        // Self-play vs best weights.
        for nn_side in [TeamId::Home, TeamId::Away] {
            for kickoff in [TeamId::Home, TeamId::Away] {
                match_configs.push((MatchKind::SelfBest, nn_side, kickoff, rng.gen::<u64>()));
            }
        }
        // Pure self-play: current vs current.
        for nn_side in [TeamId::Home, TeamId::Away] {
            for kickoff in [TeamId::Home, TeamId::Away] {
                match_configs.push((MatchKind::SelfPure, nn_side, kickoff, rng.gen::<u64>()));
            }
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

            let returns = compute_returns(steps);
            reinforce_update(&mut weights, steps, &returns, lr);
        }

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

        weights.save_to_path(weights_path).ok();
    }

    weights.save_to_path(weights_path).ok();
    eprintln!("saved final weights to {weights_path}");
}
