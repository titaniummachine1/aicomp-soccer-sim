use crate::api::{ApiFieldMask, TeamApi};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
use crate::deterministic::{
    evaluate_and_apply_with_action, DeterministicCache, NNActions, ShootTarget,
};
use crate::params::SimParams;
use crate::player::PlayerId;
use bevy::prelude::Vec2;
use serde::{Deserialize, Serialize};

/// Input feature dimension.
/// EXISTING (114):
///   Ball(2) + ball_vel(2) + 4 team_pos(8) + 4 opp_pos(8) +
///   16 dist_opp + 12 dist_mate + 4 pass_dir(8) + 4 can_pass(4) +
///   3 bools(team_has/opp_has/loose) +
///   opp_goal_center(2) + opp_goal_left(2) + opp_goal_right(2) +
///   team_goal_center(2) + team_goal_left(2) + team_goal_right(2) +
///   shot_charge(1) + carrier_stam(1) +
///   4 team_has_ball + 4 opp_has_ball + 4 team_stamina + 4 opp_stamina + 4 team_charge +
///   4 loose_ball_intercept_pos(8) +
///   4 tackle_intercept_pos(8) + best_tackler(1)
/// NEW (146):
///   4 opp_vel(8) + carrier_id(1) +
///   4 dist_to_ball(4) + 4 dist_to_own_goal(4) + 4 dist_to_opp_goal(4) +
///   kickoff(1) + 4 can_opp_intercept_pass(4) +
///   4×4 dir_to_teammate(32) + 4×4 dir_to_opp(32) +
///   4×4 perfect_intercept_opp(32) + 4 receive_pass_dir(8) +
///   4 perfect_pass_dir_per_player(8) + 4 is_open(4) + 4 opp_is_open(4)
/// Total = 114 + 146 = 260
pub const INPUT_DIM: usize = 260;
/// Output layout (hivemind, 157 dims):
/// Per player (4 × 37 = 148):
///   8 compass directions (N/NE/E/SE/S/SW/W/NW) — softmax
///   4 toward teammates — softmax
///   4 away from teammates — softmax
///   4 receive-pass directions — softmax (move to catch pass from each teammate)
///   4 perfect pass directions — softmax (walk in direction of lead pass)
///   4 toward opponents — softmax
///   4 away from opponents — softmax
///   3 goal directions (left/center/right) — softmax
///   1 sprint — sigmoid
///   1 tackle — sigmoid
/// Team-level (9): shoot_signal(1) + shoot_target(3) + pass_signal(1) + pass_target(4)
pub const OUTPUT_DIM: usize = 157;
pub const VALUES_PER_PLAYER: usize = 37;
pub const HIDDEN_DIM: usize = 128;

/// 8 compass directions: N, NE, E, SE, S, SW, W, NW
pub const COMPASS_DIRS: [Vec2; 8] = [
    Vec2::new(0.0, 1.0),   // N
    Vec2::new(0.7071, 0.7071), // NE
    Vec2::new(1.0, 0.0),   // E
    Vec2::new(0.7071, -0.7071), // SE
    Vec2::new(0.0, -1.0),  // S
    Vec2::new(-0.7071, -0.7071), // SW
    Vec2::new(-1.0, 0.0),  // W
    Vec2::new(-0.7071, 0.7071), // NW
];

/// Number of softmax groups per player (each group is a set of direction logits)
/// Groups: compass(8) + toward_mate(4) + away_mate(4) + receive_pass(4) +
///         perfect_pass(4) + toward_opp(4) + away_opp(4) + goal(3) = 35
/// Plus sprint(1) + tackle(1) = 37 total per player
pub const SOFTMAX_GROUPS: &[(usize, usize)] = &[
    (0, 8),   // compass
    (8, 4),   // toward teammates
    (12, 4),  // away from teammates
    (16, 4),  // receive-pass directions
    (20, 4),  // perfect pass directions
    (24, 4),  // toward opponents
    (28, 4),  // away from opponents
    (32, 3),  // goal directions
];
pub const SPRINT_OFFSET: usize = 35;
pub const TACKLE_OFFSET: usize = 36;
/// Team-level output starts at 4 * 37 = 148
pub const TEAM_OUTPUT_START: usize = 148;

const NORM: f32 = 20.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayerWeights {
    pub w: Vec<Vec<f32>>,
    pub b: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetWeights {
    pub layer0: LayerWeights,
    pub layer1: LayerWeights,
    pub layer2: LayerWeights,
    /// Value head: hidden_dim -> 1 (for AlphaZero-style value estimation)
    #[serde(default)]
    pub value_head: LayerWeights,
    pub input_dim: usize,
    pub hidden_dim: usize,
    pub output_dim: usize,
}

impl NetWeights {
    pub fn random(input_dim: usize, hidden_dim: usize, output_dim: usize) -> Self {
        let scale0 = (2.0 / input_dim as f32).sqrt();
        let scale1 = (2.0 / hidden_dim as f32).sqrt();
        let scale2 = (2.0 / hidden_dim as f32).sqrt();
        let scale_v = (2.0 / hidden_dim as f32).sqrt();
        Self {
            layer0: random_layer(input_dim, hidden_dim, scale0),
            layer1: random_layer(hidden_dim, hidden_dim, scale1),
            layer2: random_layer(hidden_dim, output_dim, scale2),
            value_head: random_layer(hidden_dim, 1, scale_v),
            input_dim,
            hidden_dim,
            output_dim,
        }
    }

    pub fn load_from_path(path: &str) -> Option<Self> {
        let json = std::fs::read_to_string(path).ok()?;
        let w: NetWeights = serde_json::from_str(&json).ok()?;
        if w.input_dim != INPUT_DIM || w.output_dim != OUTPUT_DIM || w.hidden_dim != HIDDEN_DIM {
            eprintln!("load_from_path: dimension mismatch (got {}x{}x{}, expected {}x{}x{}), using random init",
                w.input_dim, w.hidden_dim, w.output_dim,
                INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM);
            return None;
        }
        // If value_head is missing (old format), initialize it randomly.
        if w.value_head.w.is_empty() {
            eprintln!("load_from_path: value_head missing, initializing randomly");
            let mut w = w;
            let scale_v = (2.0 / HIDDEN_DIM as f32).sqrt();
            w.value_head = random_layer(HIDDEN_DIM, 1, scale_v);
            return Some(w);
        }
        Some(w)
    }

    pub fn save_to_path(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap();
        std::fs::write(path, json)
    }

    pub fn perturb(&self, noise: f32, rng: &mut impl rand::Rng) -> Self {
        Self {
            layer0: perturb_layer(&self.layer0, noise, rng),
            layer1: perturb_layer(&self.layer1, noise, rng),
            layer2: perturb_layer(&self.layer2, noise, rng),
            value_head: perturb_layer(&self.value_head, noise, rng),
            input_dim: self.input_dim,
            hidden_dim: self.hidden_dim,
            output_dim: self.output_dim,
        }
    }

    pub fn param_count(&self) -> usize {
        let lc = |l: &LayerWeights| l.w.iter().map(|r| r.len()).sum::<usize>() + l.b.len();
        lc(&self.layer0) + lc(&self.layer1) + lc(&self.layer2) + lc(&self.value_head)
    }
}

fn random_layer(in_dim: usize, out_dim: usize, scale: f32) -> LayerWeights {
    let mut rng = rand::thread_rng();
    let w = (0..out_dim)
        .map(|_| {
            (0..in_dim)
                .map(|_| (rand::Rng::gen_range(&mut rng, -1.0..1.0) * scale))
                .collect()
        })
        .collect();
    let b = (0..out_dim).map(|_| 0.0).collect();
    LayerWeights { w, b }
}

fn perturb_layer(layer: &LayerWeights, noise: f32, rng: &mut impl rand::Rng) -> LayerWeights {
    let w = layer
        .w
        .iter()
        .map(|row| {
            row.iter()
                .map(|&v| v + rng.gen_range(-noise..noise))
                .collect()
        })
        .collect();
    let b = layer.b.iter().map(|&v| v + rng.gen_range(-noise..noise)).collect();
    LayerWeights { w, b }
}

fn tanh(x: f32) -> f32 {
    x.tanh()
}

fn dense(layer: &LayerWeights, input: &[f32]) -> Vec<f32> {
    layer
        .w
        .iter()
        .zip(&layer.b)
        .map(|(row, bias)| {
            let sum: f32 = row.iter().zip(input).map(|(w, i)| w * i).sum();
            tanh(sum + bias)
        })
        .collect()
}

fn forward(weights: &NetWeights, input: &[f32]) -> Vec<f32> {
    let h1 = dense(&weights.layer0, input);
    let h2 = dense(&weights.layer1, &h1);
    dense(&weights.layer2, &h2)
}

/// Activations saved during forward pass for backprop.
#[derive(Clone)]
pub struct Activations {
    pub input: Vec<f32>,
    pub h1: Vec<f32>,
    pub h2: Vec<f32>,
    pub output: Vec<f32>,
    /// Value head output (scalar).
    pub value: f32,
}

/// Gradient w.r.t. all weights, same shape as NetWeights.
#[derive(Clone)]
pub struct NetGradients {
    pub layer0: LayerWeights,
    pub layer1: LayerWeights,
    pub layer2: LayerWeights,
    pub value_head: LayerWeights,
}

impl NetGradients {
    pub fn zeros(w: &NetWeights) -> Self {
        let zeros_layer = |l: &LayerWeights| LayerWeights {
            w: l.w.iter().map(|r| vec![0.0; r.len()]).collect(),
            b: vec![0.0; l.b.len()],
        };
        Self {
            layer0: zeros_layer(&w.layer0),
            layer1: zeros_layer(&w.layer1),
            layer2: zeros_layer(&w.layer2),
            value_head: zeros_layer(&w.value_head),
        }
    }

    pub fn add(&mut self, other: &NetGradients, scale: f32) {
        for (r, o) in self.layer0.w.iter_mut().zip(&other.layer0.w) {
            for (v, ov) in r.iter_mut().zip(o) { *v += ov * scale; }
        }
        for (v, ov) in self.layer0.b.iter_mut().zip(&other.layer0.b) { *v += ov * scale; }
        for (r, o) in self.layer1.w.iter_mut().zip(&other.layer1.w) {
            for (v, ov) in r.iter_mut().zip(o) { *v += ov * scale; }
        }
        for (v, ov) in self.layer1.b.iter_mut().zip(&other.layer1.b) { *v += ov * scale; }
        for (r, o) in self.layer2.w.iter_mut().zip(&other.layer2.w) {
            for (v, ov) in r.iter_mut().zip(o) { *v += ov * scale; }
        }
        for (v, ov) in self.layer2.b.iter_mut().zip(&other.layer2.b) { *v += ov * scale; }
        for (r, o) in self.value_head.w.iter_mut().zip(&other.value_head.w) {
            for (v, ov) in r.iter_mut().zip(o) { *v += ov * scale; }
        }
        for (v, ov) in self.value_head.b.iter_mut().zip(&other.value_head.b) { *v += ov * scale; }
    }
}

impl NetWeights {
    /// Forward pass that also returns intermediate activations for backprop.
    pub fn forward_with_activations(&self, input: &[f32]) -> Activations {
        let h1 = dense(&self.layer0, input);
        let h2 = dense(&self.layer1, &h1);
        let output = dense(&self.layer2, &h2);
        let value = tanh(self.value_head.w[0].iter().zip(&h2).map(|(w, x)| w * x).sum::<f32>() + self.value_head.b[0]);
        Activations {
            input: input.to_vec(),
            h1,
            h2,
            output,
            value,
        }
    }

    /// Backward pass: compute dL/dweights given dL/doutput, dL/dvalue, and activations.
    /// grad_value is the gradient w.r.t. the value head output (scalar).
    pub fn backward(&self, grad_output: &[f32], grad_value: f32, act: &Activations) -> NetGradients {
        let hidden_dim = self.hidden_dim;
        let output_dim = self.output_dim;
        let input_dim = self.input_dim;

        // Layer 2: output = tanh(w2 * h2 + b2)
        // dL/dout_pre = grad_output * (1 - out^2)
        let mut grad_out_pre = vec![0.0f32; output_dim];
        for i in 0..output_dim {
            grad_out_pre[i] = grad_output[i] * (1.0 - act.output[i] * act.output[i]);
        }

        let mut grad_l2_w: Vec<Vec<f32>> = (0..output_dim).map(|_| vec![0.0; hidden_dim]).collect();
        let mut grad_l2_b = vec![0.0f32; output_dim];
        let mut grad_h2 = vec![0.0f32; hidden_dim];
        for i in 0..output_dim {
            grad_l2_b[i] = grad_out_pre[i];
            for j in 0..hidden_dim {
                grad_l2_w[i][j] = grad_out_pre[i] * act.h2[j];
                grad_h2[j] += self.layer2.w[i][j] * grad_out_pre[i];
            }
        }

        // Value head: value = tanh(w_v * h2 + b_v)
        // dL/dval_pre = grad_value * (1 - value^2)
        let grad_val_pre = grad_value * (1.0 - act.value * act.value);
        let mut grad_vh_w: Vec<Vec<f32>> = vec![vec![0.0; hidden_dim]];
        let mut grad_vh_b = vec![0.0f32];
        grad_vh_b[0] = grad_val_pre;
        for j in 0..hidden_dim {
            grad_vh_w[0][j] = grad_val_pre * act.h2[j];
            grad_h2[j] += self.value_head.w[0][j] * grad_val_pre;
        }

        // Layer 1: h2 = tanh(w1 * h1 + b1)
        let mut grad_h2_pre = vec![0.0f32; hidden_dim];
        for i in 0..hidden_dim {
            grad_h2_pre[i] = grad_h2[i] * (1.0 - act.h2[i] * act.h2[i]);
        }

        let mut grad_l1_w: Vec<Vec<f32>> = (0..hidden_dim).map(|_| vec![0.0; hidden_dim]).collect();
        let mut grad_l1_b = vec![0.0f32; hidden_dim];
        let mut grad_h1 = vec![0.0f32; hidden_dim];
        for i in 0..hidden_dim {
            grad_l1_b[i] = grad_h2_pre[i];
            for j in 0..hidden_dim {
                grad_l1_w[i][j] = grad_h2_pre[i] * act.h1[j];
                grad_h1[j] += self.layer1.w[i][j] * grad_h2_pre[i];
            }
        }

        // Layer 0: h1 = tanh(w0 * input + b0)
        let mut grad_h1_pre = vec![0.0f32; hidden_dim];
        for i in 0..hidden_dim {
            grad_h1_pre[i] = grad_h1[i] * (1.0 - act.h1[i] * act.h1[i]);
        }

        let mut grad_l0_w: Vec<Vec<f32>> = (0..hidden_dim).map(|_| vec![0.0; input_dim]).collect();
        let mut grad_l0_b = vec![0.0f32; hidden_dim];
        for i in 0..hidden_dim {
            grad_l0_b[i] = grad_h1_pre[i];
            for j in 0..input_dim {
                grad_l0_w[i][j] = grad_h1_pre[i] * act.input[j];
            }
        }

        NetGradients {
            layer0: LayerWeights { w: grad_l0_w, b: grad_l0_b },
            layer1: LayerWeights { w: grad_l1_w, b: grad_l1_b },
            layer2: LayerWeights { w: grad_l2_w, b: grad_l2_b },
            value_head: LayerWeights { w: grad_vh_w, b: grad_vh_b },
        }
    }

    /// Apply gradient update: w += lr * grad
    pub fn apply_gradient(&mut self, grad: &NetGradients, lr: f32) {
        for (r, gr) in self.layer0.w.iter_mut().zip(&grad.layer0.w) {
            for (v, gv) in r.iter_mut().zip(gr) { *v += lr * gv; }
        }
        for (v, gv) in self.layer0.b.iter_mut().zip(&grad.layer0.b) { *v += lr * gv; }
        for (r, gr) in self.layer1.w.iter_mut().zip(&grad.layer1.w) {
            for (v, gv) in r.iter_mut().zip(gr) { *v += lr * gv; }
        }
        for (v, gv) in self.layer1.b.iter_mut().zip(&grad.layer1.b) { *v += lr * gv; }
        for (r, gr) in self.layer2.w.iter_mut().zip(&grad.layer2.w) {
            for (v, gv) in r.iter_mut().zip(gr) { *v += lr * gv; }
        }
        for (v, gv) in self.layer2.b.iter_mut().zip(&grad.layer2.b) { *v += lr * gv; }
        for (r, gr) in self.value_head.w.iter_mut().zip(&grad.value_head.w) {
            for (v, gv) in r.iter_mut().zip(gr) { *v += lr * gv; }
        }
        for (v, gv) in self.value_head.b.iter_mut().zip(&grad.value_head.b) { *v += lr * gv; }
    }

    /// Forward pass returning just the output (no activations saved).
    pub fn forward(&self, input: &[f32]) -> Vec<f32> {
        forward(self, input)
    }

    /// Log probability of an action given the network's output (mean) and
    /// the noise that was added.  action = mean + noise, noise ~ N(0, sigma^2).
    /// log_prob = -0.5 * sum(noise^2 / sigma^2) - 0.5 * D * log(2*pi*sigma^2)
    pub fn log_prob(&self, mean: &[f32], noise: &[f32], sigma: f32) -> f32 {
        let d = mean.len();
        let mut sq_sum = 0.0f32;
        for i in 0..d {
            sq_sum += noise[i] * noise[i];
        }
        -0.5 * sq_sum / (sigma * sigma) - 0.5 * d as f32 * (2.0 * std::f32::consts::PI * sigma * sigma).ln()
    }
}

pub fn extract_features(api: &TeamApi, opp_vel: &[Vec2; 4]) -> ([f32; INPUT_DIM], bool) {
    let is_home = api.get_bool("Is Home Team").unwrap_or(true);
    let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
    let ball_vel = api.get_vector3("Ball Velocity").flatten().unwrap_or(Vec2::ZERO);
    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(Vec2::ZERO);
    let opp_goal_left = api.get_transform("Opponent Goal Left Post").unwrap_or(Vec2::ZERO);
    let opp_goal_right = api.get_transform("Opponent Goal Right Post").unwrap_or(Vec2::ZERO);
    let team_goal = api.get_transform("Team Goal Center").unwrap_or(Vec2::ZERO);
    let team_goal_left = api.get_transform("Team Goal Left Post").unwrap_or(Vec2::ZERO);
    let team_goal_right = api.get_transform("Team Goal Right Post").unwrap_or(Vec2::ZERO);
    let team_has = api.get_bool("Team Has Ball").unwrap_or(false) as u32 as f32;
    let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false) as u32 as f32;
    let is_loose = api.get_bool("Is Ball Loose").unwrap_or(false) as u32 as f32;
    let is_kickoff = api.get_bool("Is Kickoff").unwrap_or(false) as u32 as f32;
    let charge = api.get_float("Ball Carrier Shot Charge").unwrap_or(0.0) / 1.0;
    let stam = api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;

    let mut team_pos = [Vec2::ZERO; 4];
    let mut opp_pos = [Vec2::ZERO; 4];
    for (i, id) in PlayerId::ALL.iter().enumerate() {
        team_pos[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(Vec2::ZERO);
        opp_pos[i] = api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(Vec2::ZERO);
    }

    let mut dist_opp = [0.0f32; 16];
    for pi in 0..4 {
        for oi in 0..4 {
            dist_opp[pi * 4 + oi] = api
                .get_float(&format!("Distance from Team Player {} to Opponent {}", pi + 1, oi + 1))
                .unwrap_or(0.0)
                / NORM;
        }
    }

    let mut dist_mate = [0.0f32; 12];
    let mate_pairs = [(0,1),(0,2),(0,3),(1,0),(1,2),(1,3),(2,0),(2,1),(2,3),(3,0),(3,1),(3,2)];
    for (k, (pi, mi)) in mate_pairs.iter().enumerate() {
        dist_mate[k] = api
            .get_float(&format!("Distance from Team Player {} to Teammate {}", pi + 1, mi + 1))
            .unwrap_or(0.0)
            / NORM;
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

    let mut team_has_ball = [0.0f32; 4];
    let mut opp_has_ball = [0.0f32; 4];
    for i in 0..4 {
        team_has_ball[i] = api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false) as u32 as f32;
        opp_has_ball[i] = api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) as u32 as f32;
    }

    let mut team_stam = [0.0f32; 4];
    let mut team_charge = [0.0f32; 4];
    let mut opp_stam = [0.0f32; 4];
    for i in 0..4 {
        team_stam[i] = api.get_float(&format!("Team Player {} Stamina", i + 1)).unwrap_or(0.0) / 100.0;
        team_charge[i] = api.get_float(&format!("Teammate {} Shot Charge", i + 1)).unwrap_or(0.0) / 1.0;
        opp_stam[i] = api.get_float(&format!("Opponent Player {} Stamina", i + 1)).unwrap_or(0.0) / 100.0;
    }

    // Loose ball intercept positions for each teammate.
    let mut intercept_pos = [0.0f32; 8];
    for i in 0..4 {
        let ip = api.get_vector3(&format!("Loose Ball Intercept Position Teammate {}", i + 1)).flatten();
        if let Some(p) = ip {
            intercept_pos[i * 2] = p.x / NORM;
            intercept_pos[i * 2 + 1] = p.y / NORM;
        }
    }

    // Tackle interception: for each teammate, where to go to intercept the
    // opponent ball carrier. Project carrier movement toward our goal.
    let mut tackle_intercept = [0.0f32; 8];
    let mut best_tackler = 0.0f32;
    let opp_carrier_idx = (0..4).find(|&i| opp_has_ball[i] > 0.5);
    if let Some(oci) = opp_carrier_idx {
        let carrier_pos = opp_pos[oci];
        // Carrier likely moves toward our goal.
        let to_our_goal = team_goal - carrier_pos;
        let carrier_dir = if to_our_goal.length_squared() > 1e-6 {
            to_our_goal.normalize()
        } else {
            Vec2::ZERO
        };
        let carrier_speed = 7.0; // approx walk speed
        let player_speed = 8.0;  // approx sprint speed

        let mut best_dist = f32::MAX;
        for ti in 0..4 {
            let dist = (team_pos[ti] - carrier_pos).length();
            let time_to_reach = dist / player_speed;
            // Carrier moves toward our goal during that time.
            let intercept_point = carrier_pos + carrier_dir * carrier_speed * time_to_reach;
            tackle_intercept[ti * 2] = intercept_point.x / NORM;
            tackle_intercept[ti * 2 + 1] = intercept_point.y / NORM;
            if dist < best_dist {
                best_dist = dist;
                best_tackler = ti as f32;
            }
        }
    }

    // ── NEW FEATURES ──

    // Opponent velocities (computed from position deltas).
    let mut opp_vel_arr = [0.0f32; 8];
    for i in 0..4 {
        opp_vel_arr[i * 2] = opp_vel[i].x / NORM;
        opp_vel_arr[i * 2 + 1] = opp_vel[i].y / NORM;
    }

    // Carrier ID: 0.0-0.25 = team P1, 0.25-0.5 = team P2, etc.
    // 0.5-0.75 = opp P1, 0.75-1.0 = opp P2, etc. 1.0 = loose.
    let carrier_id = if let Some(ti) = (0..4).find(|&i| team_has_ball[i] > 0.5) {
        ti as f32 / 8.0
    } else if let Some(oi) = (0..4).find(|&i| opp_has_ball[i] > 0.5) {
        (oi as f32 + 4.0) / 8.0
    } else {
        1.0
    };

    // Distance to ball per player.
    let mut dist_ball = [0.0f32; 4];
    for i in 0..4 {
        dist_ball[i] = (team_pos[i] - ball).length() / NORM;
    }

    // Distance to own goal per player.
    let mut dist_own_goal = [0.0f32; 4];
    for i in 0..4 {
        dist_own_goal[i] = (team_pos[i] - team_goal).length() / NORM;
    }

    // Distance to opponent goal per player.
    let mut dist_opp_goal = [0.0f32; 4];
    for i in 0..4 {
        dist_opp_goal[i] = (team_pos[i] - opp_goal).length() / NORM;
    }

    // Can opponent intercept pass to each teammate.
    let mut can_opp_intercept = [0.0f32; 4];
    for i in 0..4 {
        can_opp_intercept[i] = (!api.get_bool(&format!("Can Pass to Teammate {}", i + 1)).unwrap_or(false)) as u32 as f32;
    }

    // Is open flags.
    let mut is_open = [0.0f32; 4];
    let mut opp_is_open = [0.0f32; 4];
    for i in 0..4 {
        is_open[i] = api.get_bool(&format!("Is Team Player {} Open", i + 1)).unwrap_or(false) as u32 as f32;
        opp_is_open[i] = api.get_bool(&format!("Is Opponent Player {} Open", i + 1)).unwrap_or(false) as u32 as f32;
    }

    // Direction to each teammate per player (4×4×2 = 32).
    let mut dir_to_mate = [0.0f32; 32];
    for pi in 0..4 {
        for mi in 0..4 {
            if pi == mi {
                continue;
            }
            let d = team_pos[mi] - team_pos[pi];
            let len = d.length();
            if len > 1e-6 {
                let n = d / len;
                dir_to_mate[(pi * 4 + mi) * 2] = n.x;
                dir_to_mate[(pi * 4 + mi) * 2 + 1] = n.y;
            }
        }
    }

    // Direction to each opponent per player (4×4×2 = 32).
    let mut dir_to_opp = [0.0f32; 32];
    for pi in 0..4 {
        for oi in 0..4 {
            let d = opp_pos[oi] - team_pos[pi];
            let len = d.length();
            if len > 1e-6 {
                let n = d / len;
                dir_to_opp[(pi * 4 + oi) * 2] = n.x;
                dir_to_opp[(pi * 4 + oi) * 2 + 1] = n.y;
            }
        }
    }

    // Perfect intercept direction to each opponent per player (4×4×2 = 32).
    // Direction from each player to where they should go to intercept each opponent.
    // Uses opponent velocity to predict future position.
    let mut perfect_intercept_opp = [0.0f32; 32];
    let player_speed = 8.0;
    for pi in 0..4 {
        for oi in 0..4 {
            let to_opp = opp_pos[oi] - team_pos[pi];
            let dist = to_opp.length();
            if dist < 1e-6 {
                continue;
            }
            let time_to_reach = dist / player_speed;
            let future_opp = opp_pos[oi] + opp_vel[oi] * time_to_reach;
            let to_future = future_opp - team_pos[pi];
            let len = to_future.length();
            if len > 1e-6 {
                let n = to_future / len;
                perfect_intercept_opp[(pi * 4 + oi) * 2] = n.x;
                perfect_intercept_opp[(pi * 4 + oi) * 2 + 1] = n.y;
            }
        }
    }

    // Receive-pass direction per player (4×2 = 8).
    // Direction each player should move to receive a pass from the current carrier.
    // If a teammate has the ball, direction is toward the perfect pass arrival point.
    // Approximated as direction toward the carrier's perfect pass direction target.
    let mut receive_pass_dir = [0.0f32; 8];
    let team_carrier = (0..4).find(|&i| team_has_ball[i] > 0.5);
    if let Some(ci) = team_carrier {
        for i in 0..4 {
            if i == ci {
                continue;
            }
            let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", i + 1)).flatten();
            if let Some(d) = pd {
                // The pass direction is where the ball goes. The receiver should
                // move in that same direction to meet the ball.
                receive_pass_dir[i * 2] = d.x;
                receive_pass_dir[i * 2 + 1] = d.y;
            }
        }
    }

    // Perfect pass direction per player (4×2 = 8).
    // Already in pass_dir, but include as per-player feature for the carrier.
    // pass_dir is from carrier's perspective. For non-carriers, it's zeros.
    let mut perfect_pass_per_player = [0.0f32; 8];
    if team_carrier.is_some() {
        perfect_pass_per_player.copy_from_slice(&pass_dir);
    }

    // ── Pack all inputs ──
    let mut input = [0.0f32; INPUT_DIM];
    let mut o = 0usize;
    // Existing features (114)
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
    input[o..o+2].copy_from_slice(&[opp_goal_left.x / NORM, opp_goal_left.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[opp_goal_right.x / NORM, opp_goal_right.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[team_goal.x / NORM, team_goal.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[team_goal_left.x / NORM, team_goal_left.y / NORM]); o += 2;
    input[o..o+2].copy_from_slice(&[team_goal_right.x / NORM, team_goal_right.y / NORM]); o += 2;
    input[o] = charge; o += 1;
    input[o] = stam; o += 1;
    input[o..o+4].copy_from_slice(&team_has_ball); o += 4;
    input[o..o+4].copy_from_slice(&opp_has_ball); o += 4;
    input[o..o+4].copy_from_slice(&team_stam); o += 4;
    input[o..o+4].copy_from_slice(&opp_stam); o += 4;
    input[o..o+4].copy_from_slice(&team_charge); o += 4;
    input[o..o+8].copy_from_slice(&intercept_pos); o += 8;
    input[o..o+8].copy_from_slice(&tackle_intercept); o += 8;
    input[o] = best_tackler / 3.0; o += 1;
    // New features (130)
    input[o..o+8].copy_from_slice(&opp_vel_arr); o += 8;
    input[o] = carrier_id; o += 1;
    input[o..o+4].copy_from_slice(&dist_ball); o += 4;
    input[o..o+4].copy_from_slice(&dist_own_goal); o += 4;
    input[o..o+4].copy_from_slice(&dist_opp_goal); o += 4;
    input[o] = is_kickoff; o += 1;
    input[o..o+4].copy_from_slice(&can_opp_intercept); o += 4;
    input[o..o+32].copy_from_slice(&dir_to_mate); o += 32;
    input[o..o+32].copy_from_slice(&dir_to_opp); o += 32;
    input[o..o+32].copy_from_slice(&perfect_intercept_opp); o += 32;
    input[o..o+8].copy_from_slice(&receive_pass_dir); o += 8;
    input[o..o+8].copy_from_slice(&perfect_pass_per_player); o += 8;
    input[o..o+4].copy_from_slice(&is_open); o += 4;
    input[o..o+4].copy_from_slice(&opp_is_open); o += 4;

    (input, is_home)
}

pub struct TrainedBrain {
    weights: NetWeights,
    team: Option<crate::brain::TeamId>,
    det_cache: DeterministicCache,
    macro_tick: u32,
    was_loose: bool,
    grab_hold_release_tick: bool,
    prev_opp_pos: [Vec2; 4],
    prev_opp_pos_valid: bool,
}

impl TrainedBrain {
    pub fn with_weights(weights: NetWeights) -> Self {
        Self {
            weights,
            team: None,
            det_cache: DeterministicCache::default(),
            macro_tick: 0,
            was_loose: false,
            grab_hold_release_tick: false,
            prev_opp_pos: [Vec2::ZERO; 4],
            prev_opp_pos_valid: false,
        }
    }

    pub fn with_team(mut self, team: crate::brain::TeamId) -> Self {
        self.team = Some(team);
        self
    }
}

impl Default for TrainedBrain {
    fn default() -> Self {
        let weights = NetWeights::load_from_path("assets/ppo_weights.json")
            .or_else(|| NetWeights::load_from_path("assets/ppo_best.json"))
            .or_else(|| NetWeights::load_from_path("assets/es_best.json"))
            .or_else(|| NetWeights::load_from_path("assets/es_weights.json"))
            .unwrap_or_else(|| NetWeights::random(INPUT_DIM, HIDDEN_DIM, OUTPUT_DIM));
        Self {
            weights,
            team: None,
            det_cache: DeterministicCache::default(),
            macro_tick: 0,
            was_loose: false,
            grab_hold_release_tick: false,
            prev_opp_pos: [Vec2::ZERO; 4],
            prev_opp_pos_valid: false,
        }
    }
}

impl TeamBrain for TrainedBrain {
    fn kickoff_formation(&self) -> [Option<Vec2>; 4] {
        // Aggressive kickoff: P1 on the ball, P2 pushed up in Z,
        // P3 pushed down in Z, P4 near own goal.
        // Positions are world-absolute; clamp_to_own_half handles X mirroring.
        let goal_x = match self.team {
            Some(crate::brain::TeamId::Home) => -30.0,
            Some(crate::brain::TeamId::Away) => 30.0,
            None => -30.0, // default to Home
        };
        [
            Some(Vec2::new(0.0, 0.0)),    // P1: on the ball
            Some(Vec2::new(0.0, 6.7)),    // P2: pushed up
            Some(Vec2::new(0.0, -6.7)),   // P3: pushed down
            Some(Vec2::new(goal_x, 0.0)), // P4: near own goal
        ]
    }

    fn api_mask(&self) -> Option<ApiFieldMask> {
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
            m.needs_vector_set(&format!("Loose Ball Intercept Position Teammate {n}"));
        }
        // Tackle intercept positions are computed from opp_pos + team_goal
        // which are already requested above. No new API fields needed.
        for pi in 1..=4u8 {
            for oi in 1..=4u8 {
                m.needs_float_set(&format!("Distance from Team Player {pi} to Opponent {oi}"));
            }
        }
        for (pi, mi) in [(1u8,2u8),(1,3),(1,4),(2,1),(2,3),(2,4),(3,1),(3,2),(3,4),(4,1),(4,2),(4,3)] {
            m.needs_float_set(&format!("Distance from Team Player {pi} to Teammate {mi}"));
        }
        Some(m)
    }

    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();

        // Compute opponent velocities from position deltas.
        let mut opp_pos = [Vec2::ZERO; 4];
        for (i, id) in PlayerId::ALL.iter().enumerate() {
            opp_pos[i] = api.get_transform(&format!("Opponent Player {}", id.0)).unwrap_or(Vec2::ZERO);
        }
        let mut opp_vel = [Vec2::ZERO; 4];
        if self.prev_opp_pos_valid {
            for i in 0..4 {
                opp_vel[i] = (opp_pos[i] - self.prev_opp_pos[i]) / crate::world::FIXED_DT;
            }
        }
        self.prev_opp_pos = opp_pos;
        self.prev_opp_pos_valid = true;

        let (input, is_home) = extract_features(api, &opp_vel);
        let output = forward(&self.weights, &input);

        let mut players = [Vec2::ZERO; 4];
        for (i, id) in PlayerId::ALL.iter().enumerate() {
            players[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(Vec2::ZERO);
        }

        let mut has_ball = [false; 4];
        for i in 0..4 {
            has_ball[i] = api.get_bool(&format!("Team Player {} Has Ball", i + 1)).unwrap_or(false);
        }

        // Dynamic kickoff formation.
        let team_has_ball = api.get_bool("Team Has Ball").unwrap_or(false);
        let opp_has_ball = api.get_bool("Opponent Has Ball").unwrap_or(false);
        let is_loose_kickoff = !team_has_ball && !opp_has_ball;
        let goal_x = if is_home { -30.0 } else { 30.0 };
        if is_loose_kickoff {
            out.kickoff_positions = [
                Some(Vec2::new(0.0, 0.0)),
                Some(Vec2::new(0.0, 6.7)),
                Some(Vec2::new(0.0, -6.7)),
                Some(Vec2::new(goal_x, 0.0)),
            ];
        }

        // Per-player: decode softmax direction groups + sprint + tackle.
        // Each player has 37 output values: 35 direction logits (8 groups) + sprint + tackle.
        // We pick the best group (highest max logit across all groups) and use its direction.
        let mut opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(Vec2::ZERO);
        let opp_goal_left = api.get_transform("Opponent Goal Left Post").unwrap_or(Vec2::ZERO);
        let opp_goal_right = api.get_transform("Opponent Goal Right Post").unwrap_or(Vec2::ZERO);
        let _ = &mut opp_goal;

        for i in 0..4 {
            let base = i * VALUES_PER_PLAYER;

            // Evaluate each softmax group: pick the group+index with highest logit.
            let mut best_dir = Vec2::ZERO;
            let mut best_logit = f32::MIN;

            for (grp_offset, grp_size) in SOFTMAX_GROUPS {
                for j in 0..*grp_size {
                    let logit = output[base + grp_offset + j];
                    if logit > best_logit {
                        best_logit = logit;
                        // Decode direction based on which group and index.
                        best_dir = match *grp_offset {
                            0 => COMPASS_DIRS[j], // compass
                            8 => { // toward teammates
                                let mate = api.get_transform(&format!("Team Player {}", j + 1)).unwrap_or(Vec2::ZERO);
                                let d = mate - players[i];
                                if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO }
                            }
                            12 => { // away from teammates
                                let mate = api.get_transform(&format!("Team Player {}", j + 1)).unwrap_or(Vec2::ZERO);
                                let d = players[i] - mate;
                                if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO }
                            }
                            16 => { // receive-pass direction (move to catch pass from teammate j)
                                let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", j + 1)).flatten();
                                if let Some(d) = pd { d } else { Vec2::ZERO }
                            }
                            20 => { // perfect pass direction (walk in lead pass direction to teammate j)
                                let pd = api.get_vector3(&format!("Perfect Pass Direction to Teammate {}", j + 1)).flatten();
                                if let Some(d) = pd { d } else { Vec2::ZERO }
                            }
                            24 => { // toward opponents
                                let opp = api.get_transform(&format!("Opponent Player {}", j + 1)).unwrap_or(Vec2::ZERO);
                                let d = opp - players[i];
                                if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO }
                            }
                            28 => { // away from opponents
                                let opp = api.get_transform(&format!("Opponent Player {}", j + 1)).unwrap_or(Vec2::ZERO);
                                let d = players[i] - opp;
                                if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO }
                            }
                            32 => { // goal directions
                                match j {
                                    0 => { let d = opp_goal_left - players[i]; if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO } }
                                    1 => { let d = opp_goal - players[i]; if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO } }
                                    _ => { let d = opp_goal_right - players[i]; if d.length() > 1e-6 { d.normalize() } else { Vec2::ZERO } }
                                }
                            }
                            _ => Vec2::ZERO,
                        };
                    }
                }
            }

            let move_to = players[i] + best_dir * NORM;
            let sprint_sig = output[base + SPRINT_OFFSET];
            let tackle_sig = output[base + TACKLE_OFFSET];

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
        let shoot_signal = output[TEAM_OUTPUT_START];
        let pass_signal = output[TEAM_OUTPUT_START + 4];

        let mut shoot = false;
        let mut shoot_target = ShootTarget::Center;
        if shoot_signal > 0.0 {
            shoot = true;
            let logits = [output[TEAM_OUTPUT_START + 1], output[TEAM_OUTPUT_START + 2], output[TEAM_OUTPUT_START + 3]];
            let best = logits.iter().enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(1);
            shoot_target = match best {
                0 => ShootTarget::Left,
                1 => ShootTarget::Center,
                _ => ShootTarget::Right,
            };
        }

        let mut pass = false;
        let mut pass_target = 0usize;
        if pass_signal > 0.0 && !shoot {
            pass = true;
            let logits = [
                output[TEAM_OUTPUT_START + 5],
                output[TEAM_OUTPUT_START + 6],
                output[TEAM_OUTPUT_START + 7],
                output[TEAM_OUTPUT_START + 8],
            ];
            pass_target = logits.iter().enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
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

        let params = SimParams::default();
        evaluate_and_apply_with_action(api, &params, &mut out, &mut self.det_cache, &actions);

        // Auto-tackle macro: when opponent has ball and a player is in range, force tackle.
        let opp_has_ball = api.get_bool("Opponent Has Ball").unwrap_or(false);
        if opp_has_ball {
            let interact_r = params.interact_radius;
            let carrier_stam = api.get_float("Ball Carrier Stamina").unwrap_or(0.0) / 100.0;
            let mut carrier_pos = Vec2::ZERO;
            for i in 0..4u8 {
                if api.get_bool(&format!("Opponent Player {} Has Ball", i + 1)).unwrap_or(false) {
                    carrier_pos = api.get_transform(&format!("Opponent Player {}", i + 1)).unwrap_or(Vec2::ZERO);
                    break;
                }
            }
            let mut candidates: Vec<(usize, f32)> = Vec::new();
            for i in 0..4 {
                let p = api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(Vec2::ZERO);
                let dist = (p - carrier_pos).length();
                if dist <= interact_r {
                    let stam = api.get_float(&format!("Team Player {} Stamina", i + 1)).unwrap_or(0.0) / 100.0;
                    candidates.push((i, stam));
                }
            }
            if !candidates.is_empty() {
                let chosen = candidates.iter()
                    .filter(|(_, s)| *s >= carrier_stam - 1e-4)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .or_else(|| candidates.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)))
                    .map(|(i, _)| *i)
                    .unwrap_or(0);
                for (i, _) in &candidates {
                    out.commands[*i].interact = *i == chosen;
                }
            }
        }

        // Auto-pickup macro.
        let is_loose = api.get_bool("Is Ball Loose").unwrap_or(false);
        self.macro_tick += 1;
        if is_loose {
            let interact_r = api.get_float("Player Interact Radius").unwrap_or(1.5);
            let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);

            let mut best_idx = None;
            let mut best_dist = f32::MAX;
            for i in 0..4 {
                if has_ball[i] { continue; }
                let p = api.get_transform(&format!("Team Player {}", i + 1)).unwrap_or(Vec2::ZERO);
                let d = (p - ball).length();
                if d < best_dist {
                    best_dist = d;
                    best_idx = Some(i);
                }
            }
            if let Some(idx) = best_idx {
                if best_dist <= interact_r {
                    out.commands[idx].interact = self.macro_tick % 2 == 1;
                }
            }
        }

        // Grab-hold release.
        if self.grab_hold_release_tick {
            self.grab_hold_release_tick = false;
            for i in 0..4 {
                if has_ball[i] {
                    out.commands[i].interact = false;
                    break;
                }
            }
        }
        if self.was_loose && !is_loose {
            self.grab_hold_release_tick = true;
        }
        self.was_loose = is_loose;

        out
    }
}