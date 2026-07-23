//! Runtime values flowing through graph ports.

use bevy::prelude::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum GraphValue {
    Bool(bool),
    Float(f32),
    String(String),
    /// Pitch-plane position / direction (Unity XZ → our xy).
    Vec(Vec2),
    /// World transform treated as position for RelativePosition / GetTransform.
    Transform(Vec2),
    Null,
}

impl GraphValue {
    pub fn as_bool(&self) -> bool {
        match self {
            GraphValue::Bool(b) => *b,
            GraphValue::Float(f) => *f != 0.0,
            GraphValue::Null => false,
            _ => false,
        }
    }

    pub fn as_float(&self) -> f32 {
        match self {
            GraphValue::Float(f) => *f,
            GraphValue::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            GraphValue::Vec(v) | GraphValue::Transform(v) => v.length(),
            _ => 0.0,
        }
    }

    pub fn as_vec(&self) -> Vec2 {
        match self {
            GraphValue::Vec(v) | GraphValue::Transform(v) => *v,
            _ => Vec2::ZERO,
        }
    }

    pub fn as_transform_pos(&self) -> Vec2 {
        match self {
            GraphValue::Transform(v) | GraphValue::Vec(v) => *v,
            _ => Vec2::ZERO,
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            GraphValue::String(s) => s.clone(),
            _ => String::new(),
        }
    }
}
