//! VM runtime values — no String registers (strings are compiler metadata).

use bevy::prelude::Vec2;

use crate::graph::GraphValue;

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
    Vector(Vec2),
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
            GraphValue::Vec(v) | GraphValue::Transform(v) => VmValue::Vector(*v),
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
}
