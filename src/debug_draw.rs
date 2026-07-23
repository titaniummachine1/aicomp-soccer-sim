//! Unity-compatible DebugDrawLine / DebugDrawDisc frame buffer.
//!
//! Graphs and Titanium emit into this collector; the Bevy viewer only renders
//! whatever was submitted this tick (same channel AIA uses).

use std::sync::{Mutex, MutexGuard};

use bevy::prelude::Vec2;

#[derive(Debug, Clone)]
pub struct DebugLine {
    pub a: Vec2,
    pub b: Vec2,
    pub width: f32,
    pub rgba: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct DebugDisc {
    pub center: Vec2,
    pub radius: f32,
    pub width: f32,
    pub rgba: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct DebugDrawFrame {
    pub lines: Vec<DebugLine>,
    pub discs: Vec<DebugDisc>,
}

static FRAME: Mutex<DebugDrawFrame> = Mutex::new(DebugDrawFrame {
    lines: Vec::new(),
    discs: Vec::new(),
});

fn lock() -> MutexGuard<'static, DebugDrawFrame> {
    FRAME.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clear the frame buffer (call once before brains think each tick).
pub fn begin_frame() {
    let mut g = lock();
    g.lines.clear();
    g.discs.clear();
}

pub fn line(a: Vec2, b: Vec2, width: f32, color: &str) {
    line_rgba(a, b, width, named_rgba(color));
}

pub fn line_rgba(a: Vec2, b: Vec2, width: f32, rgba: [f32; 4]) {
    lock().lines.push(DebugLine {
        a,
        b,
        width: width.max(0.0),
        rgba,
    });
}

pub fn disc(center: Vec2, radius: f32, width: f32, color: &str) {
    disc_rgba(center, radius, width, named_rgba(color));
}

pub fn disc_rgba(center: Vec2, radius: f32, width: f32, rgba: [f32; 4]) {
    lock().discs.push(DebugDisc {
        center,
        radius: radius.max(0.0),
        width: width.max(0.0),
        rgba,
    });
}

/// Snapshot current submissions (viewer render; does not clear).
pub fn snapshot() -> DebugDrawFrame {
    lock().clone()
}

/// Unity Color node names → RGBA 0..1.
pub fn named_rgba(name: &str) -> [f32; 4] {
    let n = name.trim();
    match n {
        "White" => [1.0, 1.0, 1.0, 1.0],
        "Black" => [0.0, 0.0, 0.0, 1.0],
        "Red" => [1.0, 0.2, 0.15, 1.0],
        "Green" | "Lime" => [0.3, 1.0, 0.45, 1.0],
        "Blue" => [0.25, 0.45, 1.0, 1.0],
        "Yellow" | "Gold" => [1.0, 0.9, 0.2, 1.0],
        "Orange" => [1.0, 0.55, 0.15, 1.0],
        "Cyan" | "Light Blue" | "Aqua" => [0.35, 0.85, 1.0, 1.0],
        "Purple" | "Magenta" | "Violet" | "Hot Pink" => [0.72, 0.25, 1.0, 1.0],
        "Gray" | "Grey" => [0.6, 0.6, 0.6, 1.0],
        _ => [0.9, 0.9, 0.95, 0.9],
    }
}
