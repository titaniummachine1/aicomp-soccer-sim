//! Unity Vector3 ↔ sim pitch-plane mapping.
//!
//! AIComp Vector3 uses (X, Y, Z) where Y is height and Z is the pitch
//! sideline axis. The sim's `Vec2` pitch plane is (X, Z). Never drop Y when
//! constructing or splitting Vector3 in the graph — only strip it when
//! emitting a 2D controller move target or physics step.

use bevy::prelude::{Vec2, Vec3};

#[inline]
pub fn vec3_from_pitch(v: Vec2) -> Vec3 {
    Vec3::new(v.x, 0.0, v.y)
}

#[inline]
pub fn pitch_plane(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

#[inline]
pub fn vec3_from_components(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}
