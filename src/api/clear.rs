//! 8-way clear-direction sensors (AIComp order).

use bevy::prelude::*;

/// Sensor wedge labels A–H around a player (top-down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorDir {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

impl SensorDir {
    /// Unit direction on pitch. Attack for Home is +X.
    pub fn unit(self) -> Vec2 {
        // Rough 8-way compass; E≈+X attack for home graphic.
        match self {
            SensorDir::A => Vec2::new(-0.707, 0.707),
            SensorDir::B => Vec2::new(0.0, 1.0),
            SensorDir::C => Vec2::new(0.707, 0.707),
            SensorDir::D => Vec2::new(-1.0, 0.0),
            SensorDir::E => Vec2::new(1.0, 0.0),
            SensorDir::F => Vec2::new(-0.707, -0.707),
            SensorDir::G => Vec2::new(0.0, -1.0),
            SensorDir::H => Vec2::new(0.707, -0.707),
        }
    }
}

/// Home clear-dir priority: E, C, H, B, G, A, F, D
pub const CLEAR_ORDER_HOME: [SensorDir; 8] = [
    SensorDir::E,
    SensorDir::C,
    SensorDir::H,
    SensorDir::B,
    SensorDir::G,
    SensorDir::A,
    SensorDir::F,
    SensorDir::D,
];

/// Away clear-dir priority: D, F, A, G, B, H, C, E
pub const CLEAR_ORDER_AWAY: [SensorDir; 8] = [
    SensorDir::D,
    SensorDir::F,
    SensorDir::A,
    SensorDir::G,
    SensorDir::B,
    SensorDir::H,
    SensorDir::C,
    SensorDir::E,
];

pub fn clear_order(team_is_home: bool) -> &'static [SensorDir; 8] {
    if team_is_home {
        &CLEAR_ORDER_HOME
    } else {
        &CLEAR_ORDER_AWAY
    }
}

/// First unobstructed direction, or `None` (AIComp null).
pub fn first_clear_dir(
    origin: Vec2,
    team_is_home: bool,
    blockers: &[Vec2],
    blocker_r: f32,
    range: f32,
    avoid_sidelines: bool,
    avoid_goal_lines: bool,
    x_min: f32,
    x_max: f32,
    z_min: f32,
    z_max: f32,
) -> Option<Vec2> {
    for dir in clear_order(team_is_home) {
        let u = dir.unit();
        if ray_clear(
            origin,
            u,
            range,
            blockers,
            blocker_r,
            avoid_sidelines,
            avoid_goal_lines,
            x_min,
            x_max,
            z_min,
            z_max,
        ) {
            return Some(u);
        }
    }
    None
}

fn ray_clear(
    origin: Vec2,
    dir: Vec2,
    range: f32,
    blockers: &[Vec2],
    blocker_r: f32,
    avoid_sidelines: bool,
    avoid_goal_lines: bool,
    x_min: f32,
    x_max: f32,
    z_min: f32,
    z_max: f32,
) -> bool {
    let steps = 12;
    for i in 1..=steps {
        let t = range * (i as f32) / (steps as f32);
        let p = origin + dir * t;
        if avoid_sidelines && (p.y < z_min || p.y > z_max) {
            return false;
        }
        if avoid_goal_lines && (p.x < x_min || p.x > x_max) {
            return false;
        }
        for b in blockers {
            if (*b - p).length() < blocker_r {
                return false;
            }
        }
    }
    true
}
