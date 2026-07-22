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
    /// Upright post centers at (±posts_x, ±goal_half_width).
    pub posts_x: f32,
    /// Solid post world radius (visual + geometry).
    pub post_radius: f32,
    /// Ball-center contact radius = post_radius + ball_radius.
    pub post_contact_radius: f32,
    pub kickoff_circle_r: f32,
    pub player_max_speed: f32,
    pub player_accel: f32,
    /// Soft cap on launch |v| (TimePlot ~29.94; ball maxSpeed ~30).
    pub kick_max_speed: f32,
    /// Planar launch: `min(base + per_charge * c, kick_horiz_cap)` (TimePlot sweep).
    pub kick_speed_base: f32,
    pub kick_speed_per_charge: f32,
    pub kick_horiz_cap: f32,
    /// Lift (hidden Y): `max(0, lift_base + lift_per_charge * c)`.
    pub kick_lift_base: f32,
    pub kick_lift_per_charge: f32,
    /// Gravity for hidden ball height (m/s²).
    pub gravity: f32,
    /// Resting ball center height while grounded / held (TimePlot ~0.33).
    pub ball_rest_height: f32,
    /// Vertical bounce restitution (TimePlot landings ≈0.23; material 0.4 high).
    pub ball_bounce_e: f32,
    pub ball_bounce_settle: f32,
    /// Facing turn cap (deg/s). TimePlot DB18: yaw steps **exactly 2500**
    /// (47.5° / FixedDt 0.019s). Forward.X looks nonlinear because it is cos(θ);
    /// do not use |dForward.X/dt| as deg/s (~40 peak ≠ 4014).
    pub angular_speed_deg: f32,
    /// Idle seconds after sprint before regen starts (Frida staminaRegenDelay).
    pub stamina_regen_delay_s: f32,
    /// Extra regen lockout after a tackle (Frida tackleStaminaRegenDelay).
    pub stamina_tackle_regen_delay_s: f32,
    /// Seconds of continuous sprint to empty 0..1 stamina (community until TimePlot).
    pub stamina_drain_full_s: f32,
    /// Seconds of idle regen to refill 0..1 (community until TimePlot).
    pub stamina_regen_full_s: f32,
    pub pickup_delay_s: f32,
    /// Seconds of Interact-with-ball before shot_charge starts rising.
    /// Baseline: ~0.30s after pickup (Home T1 and Away O2).
    pub shot_charge_warmup_s: f32,
    /// Seconds of charging Interact to reach shot_charge=1.0.
    /// Baseline: +0.05/tick at ~52.6 Hz ⇒ ~0.38s (not 0.8).
    pub shot_charge_time_s: f32,
    /// Pickup/tackle reach. **CONFIRMED** 1.75 (ApiProbe timeplot).
    pub interact_radius: f32,
    /// Soft radius for clear-dir blockers / future post-sweep (**CANDIDATE**).
    /// Not a NavMeshAgent — field has no obstacles for normal play.
    pub body_radius: f32,
    /// Hold/aim offset from player center (**CANDIDATE**).
    pub hold_offset: f32,
    /// Small ring at BallHoldLocation.
    pub hold_marker_radius: f32,
    /// Frida walkSpeed (cruise); sim uses [`player_max_speed`] for sprint cap.
    pub player_walk_speed: f32,
    /// Ball.pickupDelayAfterExchange — wired on successful tackle exchange.
    pub pickup_delay_after_exchange_s: f32,
    /// Whistle idle timeout (staleBallTimeoutSeconds). Wired in `tick_stale_ball`.
    pub stale_ball_timeout_s: f32,
    /// Whistle move epsilon (staleBallDistanceThreshold). Wired in `tick_stale_ball`.
    pub stale_ball_distance_threshold_m: f32,
    /// Kickoff spawn circle margin — reserved.
    pub kickoff_spawn_circle_margin_m: f32,
    /// Kickoff delay before play — reserved.
    pub kickoff_delay_s: f32,
    /// Mover minimumMoveDelta — reserved.
    pub minimum_move_delta_m: f32,
    pub source_path: PathBuf,
}

impl Default for SimParams {
    fn default() -> Self {
        Self::fallback(default_params_path())
    }
}

impl SimParams {
    pub fn fallback(source_path: PathBuf) -> Self {
        let body_radius = 0.5;
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
            posts_x: 40.2,
            post_radius: 0.3,
            post_contact_radius: 0.3 + 0.40637236,
            kickoff_circle_r: 7.25,
            player_max_speed: 8.0,
            player_accel: 100.0,
            kick_max_speed: 29.94,
            // (10 + 290 c) / 9  — TimePlot charge sweep 2026-07-22
            kick_speed_base: 10.0 / 9.0,
            kick_speed_per_charge: 290.0 / 9.0,
            kick_horiz_cap: 29.42,
            kick_lift_base: -0.323,
            kick_lift_per_charge: 6.6667,
            gravity: 9.81,
            ball_rest_height: 0.33,
            // TimePlot landings median e≈0.23 (material 0.4 is too high).
            ball_bounce_e: 0.23,
            ball_bounce_settle: 0.5,
            angular_speed_deg: 2500.0,
            stamina_regen_delay_s: 0.0,
            stamina_tackle_regen_delay_s: 0.0,
            // TimePlot 17-05-04 DebugBuild=14 continuous sprint/has segments.
            stamina_drain_full_s: 34.5,
            stamina_regen_full_s: 20.0,
            pickup_delay_s: 0.125,
            shot_charge_warmup_s: 0.30,
            shot_charge_time_s: 0.38,
            interact_radius: 1.75,
            body_radius,
            hold_offset: 1.557,
            hold_marker_radius: ball_radius * 0.55,
            player_walk_speed: 7.0,
            pickup_delay_after_exchange_s: 0.25,
            stale_ball_timeout_s: 5.0,
            stale_ball_distance_threshold_m: 2.5,
            kickoff_spawn_circle_margin_m: 0.5,
            kickoff_delay_s: 4.9,
            minimum_move_delta_m: 0.25,
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
            if let Some(y) = b.rest_y {
                p.ball_rest_height = y;
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
                if let Some(px) = g.posts_x {
                    p.posts_x = px;
                }
                if let Some(pr) = g.post_radius_world {
                    p.post_radius = pr;
                }
                if let Some(cr) = g.post_contact_radius {
                    p.post_contact_radius = cr;
                } else {
                    p.post_contact_radius = p.post_radius + p.ball_radius;
                }
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
            if let Some(v) = k.horiz_cap_mps {
                p.kick_horiz_cap = v;
            }
            if let Some(v) = k.speed_base_mps {
                p.kick_speed_base = v;
            }
            if let Some(v) = k.speed_per_charge_mps {
                p.kick_speed_per_charge = v;
            }
            if let Some(v) = k.lift_base_mps {
                p.kick_lift_base = v;
            }
            if let Some(v) = k.lift_per_charge_mps {
                p.kick_lift_per_charge = v;
            }
            if let Some(v) = k.gravity_mps2 {
                p.gravity = v;
            }
            if let Some(v) = k.bounce_e {
                p.ball_bounce_e = v;
            }
            if let Some(v) = k.bounce_settle_mps {
                p.ball_bounce_settle = v;
            }
        }
        let mut hold_from_file = false;
        if let Some(pl) = raw.player_candidates {
            if let Some(w) = pl.walk_speed_mps {
                p.player_walk_speed = w;
            }
            if let Some(d) = pl.intercept_defaults_mps {
                p.player_max_speed = d.max_speed.unwrap_or(p.player_max_speed);
                p.player_accel = d.acceleration.unwrap_or(p.player_accel);
            }
            if let Some(t) = pl.global_pickup_delay_after_shot_s {
                p.pickup_delay_s = t;
            }
            if let Some(t) = pl.global_pickup_delay_after_exchange_s {
                p.pickup_delay_after_exchange_s = t;
            }
            if let Some(t) = pl.shot_charge_warmup_s {
                p.shot_charge_warmup_s = t;
            }
            if let Some(t) = pl.shot_charge_time_s {
                p.shot_charge_time_s = t;
            }
            if let Some(r) = pl.body_radius_m.or(pl.nav_agent_radius_m) {
                p.body_radius = r;
            }
            if let Some(h) = pl.hold_offset_m {
                p.hold_offset = h;
                hold_from_file = true;
            }
            if let Some(ir) = pl.interact_radius_m.or(pl.tackle_range_m) {
                p.interact_radius = ir;
            }
            if let Some(m) = pl.minimum_move_delta_m {
                p.minimum_move_delta_m = m;
            }
            if let Some(k) = pl.kickoff_frida {
                if let Some(r) = k.circle_radius_m {
                    p.kickoff_circle_r = r;
                }
                if let Some(m) = k.spawn_circle_margin_m {
                    p.kickoff_spawn_circle_margin_m = m;
                }
                if let Some(d) = k.delay_seconds {
                    p.kickoff_delay_s = d;
                }
            }
            if let Some(s) = pl.stale_ball_frida {
                if let Some(t) = s.timeout_seconds {
                    p.stale_ball_timeout_s = t;
                }
                if let Some(d) = s.distance_threshold_m {
                    p.stale_ball_distance_threshold_m = d;
                }
            }
            if let Some(sh) = pl.shot_frida {
                if let Some(t) = sh.pickup_charge_delay_s {
                    p.shot_charge_warmup_s = t;
                }
            }
            if let Some(s) = pl.stamina_frida {
                if let Some(t) = s.regen_delay_s {
                    p.stamina_regen_delay_s = t;
                }
                if let Some(t) = s.tackle_regen_delay_s {
                    p.stamina_tackle_regen_delay_s = t;
                }
            }
            if let Some(t) = pl.stamina_drain_full_s {
                p.stamina_drain_full_s = t;
            }
            if let Some(t) = pl.stamina_regen_full_s {
                p.stamina_regen_full_s = t;
            }
        }
        if !hold_from_file {
            p.hold_offset = p.body_radius + p.ball_radius;
        }
        p.hold_marker_radius = p.ball_radius * 0.55;
        p
    }
}

pub fn default_params_path() -> PathBuf {
    // Prefer JSON next to the exe (portable alpha builds), then CARGO_MANIFEST_DIR (dev).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("bevy_sim_params_v05.json");
            if beside.is_file() {
                return beside;
            }
        }
    }
    let cwd = PathBuf::from("bevy_sim_params_v05.json");
    if cwd.is_file() {
        return cwd;
    }
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
    rest_y: Option<f32>,
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
    posts_x: Option<f32>,
    post_radius_world: Option<f32>,
    post_contact_radius: Option<f32>,
}

#[derive(Deserialize)]
struct RawMarks {
    #[serde(rename = "Kickoff Circle Radius")]
    kickoff_circle_radius: Option<f32>,
}

#[derive(Deserialize)]
struct RawKick {
    max_power_speed_mps: Option<f32>,
    /// Soft planar launch cap (TimePlot ~29.42).
    horiz_cap_mps: Option<f32>,
    speed_base_mps: Option<f32>,
    speed_per_charge_mps: Option<f32>,
    lift_base_mps: Option<f32>,
    lift_per_charge_mps: Option<f32>,
    gravity_mps2: Option<f32>,
    bounce_e: Option<f32>,
    bounce_settle_mps: Option<f32>,
}

#[derive(Deserialize)]
struct RawPlayers {
    walk_speed_mps: Option<f32>,
    intercept_defaults_mps: Option<RawIntercept>,
    minimum_move_delta_m: Option<f32>,
    global_pickup_delay_after_shot_s: Option<f32>,
    global_pickup_delay_after_exchange_s: Option<f32>,
    shot_charge_warmup_s: Option<f32>,
    shot_charge_time_s: Option<f32>,
    shot_frida: Option<RawShotFrida>,
    kickoff_frida: Option<RawKickoffFrida>,
    stale_ball_frida: Option<RawStaleBallFrida>,
    stamina_frida: Option<RawStaminaFrida>,
    stamina_drain_full_s: Option<f32>,
    stamina_regen_full_s: Option<f32>,
    body_radius_m: Option<f32>,
    nav_agent_radius_m: Option<f32>,
    hold_offset_m: Option<f32>,
    interact_radius_m: Option<f32>,
    tackle_range_m: Option<f32>,
}

#[derive(Deserialize)]
struct RawShotFrida {
    pickup_charge_delay_s: Option<f32>,
}

#[derive(Deserialize)]
struct RawKickoffFrida {
    circle_radius_m: Option<f32>,
    spawn_circle_margin_m: Option<f32>,
    delay_seconds: Option<f32>,
}

#[derive(Deserialize)]
struct RawStaleBallFrida {
    timeout_seconds: Option<f32>,
    distance_threshold_m: Option<f32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RawStaminaFrida {
    max: Option<f32>,
    consume_rate: Option<f32>,
    regen_rate: Option<f32>,
    regen_delay_s: Option<f32>,
    tackle_regen_delay_s: Option<f32>,
}

#[derive(Deserialize)]
struct RawIntercept {
    max_speed: Option<f32>,
    acceleration: Option<f32>,
}
