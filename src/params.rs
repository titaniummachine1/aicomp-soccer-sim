//! Load / hot-reload numbers from `bevy_sim_params_v05.json`.

use bevy::prelude::*;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Runtime sim constants (subset of the JSON pack + 2D body/hold candidates).
#[derive(Resource, Debug, Clone)]
pub struct SimParams {
    pub ball_radius: f32,
    pub slide_accel: f32,
    pub wall_e: f32,
    pub wall_mu: f32,
    pub stop_speed_eps: f32,
    pub x_min: f32,
    pub x_max: f32,
    pub z_min: f32,
    pub z_max: f32,
    pub goal_half_width: f32,
    pub goal_line_x: f32,
    pub kickoff_circle_r: f32,
    pub player_max_speed: f32,
    pub player_accel: f32,
    pub kick_max_speed: f32,
    pub pickup_delay_s: f32,
    pub interact_radius: f32,
    /// Disc radius for the player body (nav-agent width / 2). **CANDIDATE**.
    pub player_radius: f32,
    /// Distance from player center to BallHoldLocation along facing. **CANDIDATE**.
    pub hold_offset: f32,
    /// Small ring drawn at the hold/aim point.
    pub hold_marker_radius: f32,
    pub source_path: PathBuf,
}

impl Default for SimParams {
    fn default() -> Self {
        Self::fallback(default_params_path())
    }
}

impl SimParams {
    pub fn fallback(source_path: PathBuf) -> Self {
        // player_radius: Unity NavMeshAgent-style body width matters more for
        // footprint than mesh hitbox in this game. 0.5 m radius (= 1.0 m wide)
        // is a CANDIDATE until we scrape a confirmed agent radius.
        // hold_offset: BallHoldLocation sits in front — approx body_r + ball_r.
        let player_radius = 0.5;
        let ball_radius = 0.40637236;
        Self {
            ball_radius,
            slide_accel: 5.95,
            wall_e: 0.2,
            wall_mu: 0.35,
            stop_speed_eps: 0.0001,
            x_min: -39.5,
            x_max: 39.5,
            z_min: -24.7,
            z_max: 24.7,
            goal_half_width: 6.0,
            goal_line_x: 39.5,
            kickoff_circle_r: 7.25,
            player_max_speed: 4.5,
            player_accel: 2.5,
            kick_max_speed: 30.0,
            pickup_delay_s: 0.3,
            interact_radius: 1.5,
            player_radius,
            hold_offset: player_radius + ball_radius,
            hold_marker_radius: ball_radius * 0.55,
            source_path,
        }
    }

    pub fn load_from_disk(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
        let raw: RawParams = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
        Ok(Self::from_raw(raw, path.to_path_buf()))
    }

    pub fn reload(&mut self) -> Result<(), String> {
        *self = Self::load_from_disk(&self.source_path)?;
        Ok(())
    }

    fn from_raw(raw: RawParams, source_path: PathBuf) -> Self {
        let mut p = Self::fallback(source_path);
        if let Some(b) = raw.ball {
            if let Some(r) = b.radius_world {
                p.ball_radius = r;
            }
        }
        if let Some(s) = raw.free_ball_slide {
            if let Some(a) = s.accel_mps2 {
                p.slide_accel = a;
            }
            if let Some(e) = s.stop_speed_eps {
                p.stop_speed_eps = e;
            }
        }
        if let Some(c) = raw.contacts {
            if let Some(e) = c.effective_wall_restitution_e {
                p.wall_e = e;
            }
            if let Some(m) = c.effective_wall_mu {
                p.wall_mu = m;
            }
        }
        if let Some(pitch) = raw.pitch {
            if let Some(aabb) = pitch.playable_ball_center_aabb {
                p.x_min = aabb.x_min.unwrap_or(p.x_min);
                p.x_max = aabb.x_max.unwrap_or(p.x_max);
                p.z_min = aabb.z_min.unwrap_or(p.z_min);
                p.z_max = aabb.z_max.unwrap_or(p.z_max);
            }
            if let Some(g) = pitch.goal {
                p.goal_half_width = g.half_width.unwrap_or(p.goal_half_width);
                p.goal_line_x = g.line_x.unwrap_or(p.goal_line_x);
            }
        }
        if let Some(m) = raw.pitch_marks_soccer_get_float {
            if let Some(r) = m.kickoff_circle_radius {
                p.kickoff_circle_r = r;
            }
        }
        if let Some(k) = raw.kick_airborne_candidates {
            if let Some(v) = k.max_power_speed_mps {
                p.kick_max_speed = v;
            }
        }
        if let Some(pl) = raw.player_candidates {
            if let Some(d) = pl.intercept_defaults_mps {
                p.player_max_speed = d.max_speed.unwrap_or(p.player_max_speed);
                p.player_accel = d.acceleration.unwrap_or(p.player_accel);
            }
            if let Some(t) = pl.global_pickup_delay_after_shot_s {
                p.pickup_delay_s = t;
            }
            if let Some(r) = pl.nav_agent_radius_m {
                p.player_radius = r;
            }
            if let Some(h) = pl.hold_offset_m {
                p.hold_offset = h;
            }
            if let Some(ir) = pl.interact_radius_m {
                p.interact_radius = ir;
            }
        }
        // Keep hold offset consistent if only radius changed
        if raw
            .player_candidates
            .as_ref()
            .and_then(|c| c.hold_offset_m)
            .is_none()
        {
            p.hold_offset = p.player_radius + p.ball_radius;
        }
        p.hold_marker_radius = p.ball_radius * 0.55;
        p
    }
}

pub fn default_params_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bevy_sim_params_v05.json")
}

#[derive(Deserialize)]
struct RawParams {
    ball: Option<RawBall>,
    free_ball_slide: Option<RawSlide>,
    contacts: Option<RawContacts>,
    pitch: Option<RawPitch>,
    #[serde(rename = "pitch_marks_soccer_get_float")]
    pitch_marks_soccer_get_float: Option<RawMarks>,
    kick_airborne_candidates: Option<RawKick>,
    player_candidates: Option<RawPlayers>,
}

#[derive(Deserialize)]
struct RawBall {
    radius_world: Option<f32>,
}

#[derive(Deserialize)]
struct RawSlide {
    accel_mps2: Option<f32>,
    stop_speed_eps: Option<f32>,
}

#[derive(Deserialize)]
struct RawContacts {
    effective_wall_restitution_e: Option<f32>,
    effective_wall_mu: Option<f32>,
}

#[derive(Deserialize)]
struct RawPitch {
    playable_ball_center_aabb: Option<RawAabb>,
    goal: Option<RawGoal>,
}

#[derive(Deserialize)]
struct RawAabb {
    x_min: Option<f32>,
    x_max: Option<f32>,
    z_min: Option<f32>,
    z_max: Option<f32>,
}

#[derive(Deserialize)]
struct RawGoal {
    half_width: Option<f32>,
    line_x: Option<f32>,
}

#[derive(Deserialize)]
struct RawMarks {
    #[serde(rename = "Kickoff Circle Radius")]
    kickoff_circle_radius: Option<f32>,
}

#[derive(Deserialize)]
struct RawKick {
    max_power_speed_mps: Option<f32>,
}

#[derive(Deserialize)]
struct RawPlayers {
    intercept_defaults_mps: Option<RawIntercept>,
    global_pickup_delay_after_shot_s: Option<f32>,
    nav_agent_radius_m: Option<f32>,
    hold_offset_m: Option<f32>,
    interact_radius_m: Option<f32>,
}

#[derive(Deserialize)]
struct RawIntercept {
    max_speed: Option<f32>,
    acceleration: Option<f32>,
}
