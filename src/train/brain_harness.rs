use crate::api::{ApiFieldMask, TeamApi};
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
use crate::player::PlayerId;
use bevy::prelude::Vec2;
use serde::{Deserialize, Serialize};

/// Input feature dimension.
/// Ball(2) + ball_vel(2) + 4 team_pos(8) + 4 opp_pos(8) +
/// 16 dist_opp + 12 dist_mate + 4 pass_dir(8) + 4 can_pass(4) +
/// 3 bools(team_has/opp_has/loose) + opp_goal(2) + team_goal(2) +
/// shot_charge(1) + carrier_stam(1) +
/// 4 team_has_ball + 4 opp_has_ball + 4 team_stamina + 4 team_charge = 85
pub const INPUT_DIM: usize = 85;
/// Per player: move_x, move_y, sprint, + 8 action logits = 11 × 4 = 44
/// Actions: [none, shoot_left, shoot_center, shoot_right, pass_1, pass_2, pass_3, pass_4]
pub const OUTPUT_DIM: usize = 44;
pub const ACTIONS_PER_PLAYER: usize = 8;
pub const HIDDEN_DIM: usize = 64;

const NORM: f32 = 20.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerWeights {
    pub w: Vec<Vec<f32>>,
    pub b: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetWeights {
    pub layer0: LayerWeights,
    pub layer1: LayerWeights,
    pub layer2: LayerWeights,
    pub input_dim: usize,
    pub hidden_dim: usize,
    pub output_dim: usize,
}

impl NetWeights {
    pub fn random(input_dim: usize, hidden_dim: usize, output_dim: usize) -> Self {
        let scale0 = (2.0 / input_dim as f32).sqrt();
        let scale1 = (2.0 / hidden_dim as f32).sqrt();
        let scale2 = (2.0 / hidden_dim as f32).sqrt();
        Self {
            layer0: random_layer(input_dim, hidden_dim, scale0),
            layer1: random_layer(hidden_dim, hidden_dim, scale1),
            layer2: random_layer(hidden_dim, output_dim, scale2),
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
            input_dim: self.input_dim,
            hidden_dim: self.hidden_dim,
            output_dim: self.output_dim,
        }
    }

    pub fn param_count(&self) -> usize {
        let lc = |l: &LayerWeights| l.w.iter().map(|r| r.len()).sum::<usize>() + l.b.len();
        lc(&self.layer0) + lc(&self.layer1) + lc(&self.layer2)
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
pub struct Activations {
    pub input: Vec<f32>,
    pub h1: Vec<f32>,
    pub h2: Vec<f32>,
    pub output: Vec<f32>,
}

/// Gradient w.r.t. all weights, same shape as NetWeights.
#[derive(Clone)]
pub struct NetGradients {
    pub layer0: LayerWeights,
    pub layer1: LayerWeights,
    pub layer2: LayerWeights,
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
    }
}

impl NetWeights {
    /// Forward pass that also returns intermediate activations for backprop.
    pub fn forward_with_activations(&self, input: &[f32]) -> Activations {
        let h1 = dense(&self.layer0, input);
        let h2 = dense(&self.layer1, &h1);
        let output = dense(&self.layer2, &h2);
        Activations {
            input: input.to_vec(),
            h1,
            h2,
            output,
        }
    }

    /// Backward pass: compute dL/dweights given dL/doutput and activations.
    pub fn backward(&self, grad_output: &[f32], act: &Activations) -> NetGradients {
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
    }
}

fn extract_features(api: &TeamApi) -> ([f32; INPUT_DIM], bool) {
    let is_home = api.get_bool("Is Home Team").unwrap_or(true);
    let ball = api.get_transform("Ball").unwrap_or(Vec2::ZERO);
    let ball_vel = api.get_vector3("Ball Velocity").flatten().unwrap_or(Vec2::ZERO);
    let opp_goal = api.get_transform("Opponent Goal Center").unwrap_or(Vec2::ZERO);
    let team_goal = api.get_transform("Team Goal Center").unwrap_or(Vec2::ZERO);
    let team_has = api.get_bool("Team Has Ball").unwrap_or(false) as u32 as f32;
    let opp_has = api.get_bool("Opponent Has Ball").unwrap_or(false) as u32 as f32;
    let is_loose = api.get_bool("Is Ball Loose").unwrap_or(false) as u32 as f32;
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
    input[o..o+4].copy_from_slice(&team_charge);

    (input, is_home)
}

pub struct TrainedBrain {
    weights: NetWeights,
    team: Option<crate::brain::TeamId>,
}

impl TrainedBrain {
    pub fn with_weights(weights: NetWeights) -> Self {
        Self { weights, team: None }
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
        Self { weights, team: None }
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
        m.needs_transform_set("Ball");
        m.needs_transform_set("Opponent Goal Center");
        m.needs_transform_set("Team Goal Center");
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
            m.needs_float_set(&format!("Team Player {n} Stamina"));
            m.needs_float_set(&format!("Teammate {n} Shot Charge"));
        }
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
        let (input, _is_home) = extract_features(api);
        let output = forward(&self.weights, &input);

        let mut players = [Vec2::ZERO; 4];
        for (i, id) in PlayerId::ALL.iter().enumerate() {
            players[i] = api.get_transform(&format!("Team Player {}", id.0)).unwrap_or(Vec2::ZERO);
        }

        for i in 0..4 {
            let base = i * 11;
            let dx = output[base];
            let dy = output[base + 1];
            let sprint_sig = output[base + 2];

            let dir = Vec2::new(dx, dy);
            let move_to = players[i] + dir * NORM;

            // Pick action with highest logit
            let mut best_action = 0usize;
            let mut best_val = output[base + 3];
            for a in 1..ACTIONS_PER_PLAYER {
                let v = output[base + 3 + a];
                if v > best_val {
                    best_val = v;
                    best_action = a;
                }
            }

            // Action 0 = none (no interact). Actions 1-7 = interact (shoot/pass)
            let interact = best_action > 0;

            out.commands[i] = BrainCommand {
                move_to,
                sprint: sprint_sig > 0.0,
                interact,
            };
        }

        out
    }
}