use crate::api::TeamApi;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
use crate::player::PlayerId;
use bevy::prelude::Vec2;
use serde::Deserialize;

const INPUT_DIM: usize = 8;
const OUTPUT_DIM: usize = 8;

#[derive(Deserialize)]
struct LayerWeights {
    w: Vec<Vec<f32>>,
    b: Vec<f32>,
}

#[derive(Deserialize)]
struct NetWeights {
    layer0: LayerWeights,
    layer1: LayerWeights,
    layer2: LayerWeights,
    input_dim: usize,
    hidden_dim: usize,
    output_dim: usize,
    normalization_scale: f32,
    square_half_size: f32,
}

impl NetWeights {
    fn load_from_path(path: &str) -> Self {
        let json = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read weights file {path}: {e}"));

        Self::load_from_str(&json)
    }

    #[allow(dead_code)]
    fn load_from_str(json: &str) -> Self {
        serde_json::from_str(json).expect("failed to parse net weights JSON")
    }
}

fn tanh_node(x: f32) -> f32 {
    (2.0 / (1.0 + (-2.0 * x).exp())) - 1.0
}

fn dense_layer(layer: &LayerWeights, input: &[f32]) -> Vec<f32> {
    layer
        .w
        .iter()
        .zip(&layer.b)
        .map(|(row, bias)| {
            let sum: f32 = row.iter()
                .zip(input)
                .map(|(w, i)| w * i)
                .sum();

            tanh_node(sum + bias)
        })
        .collect()
}

fn forward(weights: &NetWeights, input: [f32; INPUT_DIM]) -> [f32; OUTPUT_DIM] {
    let h1 = dense_layer(&weights.layer0, &input);
    let h2 = dense_layer(&weights.layer1, &h1);
    let out = dense_layer(&weights.layer2, &h2);

    out.try_into()
        .unwrap_or_else(|v: Vec<f32>| {
            panic!("expected {OUTPUT_DIM} outputs, got {}", v.len())
        })
}

const SQUARE_HALF_SIZE: f32 = 6.0;
const NORMALIZATION_SCALE: f32 = 20.0;

const CORNER_OFFSETS: [Vec2; 4] = [
    Vec2::new(SQUARE_HALF_SIZE, SQUARE_HALF_SIZE),
    Vec2::new(-SQUARE_HALF_SIZE, SQUARE_HALF_SIZE),
    Vec2::new(-SQUARE_HALF_SIZE, -SQUARE_HALF_SIZE),
    Vec2::new(SQUARE_HALF_SIZE, -SQUARE_HALF_SIZE),
];

fn mirror_x(team: bool, value: f32) -> f32 {
    match team {
        false => -value,
        true => value,
    }
}

pub struct TrainedBrain {
    weights: NetWeights,
}

impl Default for TrainedBrain {
    fn default() -> Self {
        Self {
            weights: NetWeights::load_from_path("assets/square_weights.json"),
        }
    }
}

impl TeamBrain for TrainedBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut out = BrainOutput::default();

        let team = api.get_bool("Is Home Team").unwrap();
        let ball = api.get_transform("Ball")
            .unwrap_or(Vec2::ZERO);

        let mut input = [0f32; INPUT_DIM];
        let mut players = [Vec2::ZERO; 4];

        for id in PlayerId::ALL {
            let idx = (id.0 as usize) - 1;

            let me_label = format!("Team Player {}", id.0);

            let mut me = api.get_transform(&me_label)
                .unwrap_or(ball);

            let mut local_ball = ball;

            if !team {
                me.x = -me.x;
                local_ball.x = -local_ball.x;
            }

            players[idx] = me;

            let mut corner = CORNER_OFFSETS[idx];

            if !team {
                corner.x = -corner.x;
            }

            let target = local_ball + corner;

            let rel = ((target - me) / NORMALIZATION_SCALE)
                .clamp(Vec2::splat(-1.0), Vec2::splat(1.0));

            input[idx * 2] = rel.x;
            input[idx * 2 + 1] = rel.y;
        }

        let output = forward(&self.weights, input);

        for id in PlayerId::ALL {
            let idx = (id.0 as usize) - 1;

            let mut dir = Vec2::new(
                output[idx * 2],
                output[idx * 2 + 1],
            );

            dir.x = mirror_x(team, dir.x);

            let move_to = players[idx] + dir * NORMALIZATION_SCALE;

            out.commands[idx] = BrainCommand {
                move_to,
                sprint: true,
                interact: false,
            };
        }

        out
    }
}