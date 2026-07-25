//! VM runtime values — no String registers (strings are compiler metadata).

use bevy::prelude::{Vec2, Vec3};

use crate::graph::GraphValue;
use crate::graph::pitch_vec::{pitch_plane, vec3_from_pitch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegisterKind {
    Float,
    Bool,
    Vector,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VmValue {
    Float(f32),
    Bool(bool),
    /// Full Unity Vector3 (X, Y height, Z).
    Vector(Vec3),
    Null,
}

impl Default for VmValue {
    fn default() -> Self {
        VmValue::Null
    }
}

impl VmValue {
    pub fn kind(self) -> RegisterKind {
        match self {
            VmValue::Float(_) => RegisterKind::Float,
            VmValue::Bool(_) => RegisterKind::Bool,
            VmValue::Vector(_) => RegisterKind::Vector,
            VmValue::Null => RegisterKind::Null,
        }
    }

    pub fn from_graph(v: &GraphValue) -> Self {
        match v {
            GraphValue::Float(f) => VmValue::Float(*f),
            GraphValue::Bool(b) => VmValue::Bool(*b),
            GraphValue::Vec(v) => VmValue::Vector(*v),
            GraphValue::Transform(v) => VmValue::Vector(vec3_from_pitch(*v)),
            GraphValue::String(_) | GraphValue::Null => VmValue::Null,
        }
    }

    pub fn to_graph(self) -> GraphValue {
        match self {
            VmValue::Float(f) => GraphValue::Float(f),
            VmValue::Bool(b) => GraphValue::Bool(b),
            VmValue::Vector(v) => GraphValue::Vec(v),
            VmValue::Null => GraphValue::Null,
        }
    }

    pub fn pitch_plane(self) -> Vec2 {
        match self {
            VmValue::Vector(v) => pitch_plane(v),
            _ => Vec2::ZERO,
        }
    }
}
