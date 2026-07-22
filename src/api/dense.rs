//! Dense SoccerGet catalogs — label ↔ `u16` index (no per-tick string HashMaps
//! on the RuntimeBrain LoadApi hot path).

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::prelude::Vec2;

use super::labels::{
    GET_BOOL, GET_FLOAT, GET_FLOAT_FIELD_MARKS, GET_TRANSFORM, GET_VECTOR3,
};

fn float_catalog() -> &'static [&'static str] {
    // Merge once: main floats + field marks.
    static MERGED: OnceLock<Vec<&'static str>> = OnceLock::new();
    MERGED.get_or_init(|| {
        let mut v = Vec::with_capacity(GET_FLOAT.len() + GET_FLOAT_FIELD_MARKS.len());
        v.extend_from_slice(GET_FLOAT);
        for label in GET_FLOAT_FIELD_MARKS {
            if !v.contains(label) {
                v.push(*label);
            }
        }
        v
    })
}

fn index_map(labels: &[&'static str]) -> HashMap<&'static str, u16> {
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| (*l, i as u16))
        .collect()
}

fn bool_map() -> &'static HashMap<&'static str, u16> {
    static M: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    M.get_or_init(|| index_map(GET_BOOL))
}

fn float_map() -> &'static HashMap<&'static str, u16> {
    static M: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    M.get_or_init(|| index_map(float_catalog()))
}

fn transform_map() -> &'static HashMap<&'static str, u16> {
    static M: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    M.get_or_init(|| index_map(GET_TRANSFORM))
}

fn vector_map() -> &'static HashMap<&'static str, u16> {
    static M: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    M.get_or_init(|| index_map(GET_VECTOR3))
}

pub fn bool_count() -> usize {
    GET_BOOL.len()
}
pub fn float_count() -> usize {
    float_catalog().len()
}
pub fn transform_count() -> usize {
    GET_TRANSFORM.len()
}
pub fn vector_count() -> usize {
    GET_VECTOR3.len()
}

pub fn bool_index(label: &str) -> Option<u16> {
    bool_map().get(label).copied()
}
pub fn float_index(label: &str) -> Option<u16> {
    float_map().get(label).copied()
}
pub fn transform_index(label: &str) -> Option<u16> {
    transform_map().get(label).copied()
}
pub fn vector_index(label: &str) -> Option<u16> {
    vector_map().get(label).copied()
}

/// Sentinel: slot not in SoccerGet catalog (unknown / mistyped label).
pub const UNKNOWN_ID: u16 = u16::MAX;

/// Dense team API snapshot — indexed by catalog `u16`, not strings.
#[derive(Debug, Clone)]
pub struct DenseTeamApi {
    pub team: crate::brain::TeamId,
    pub bools: Box<[bool]>,
    pub floats: Box<[f32]>,
    pub transforms: Box<[Vec2]>,
    /// `None` = AIComp null Vector3.
    pub vectors: Box<[Option<Vec2>]>,
}

impl DenseTeamApi {
    pub fn empty(team: crate::brain::TeamId) -> Self {
        Self {
            team,
            bools: vec![false; bool_count()].into_boxed_slice(),
            floats: vec![0.0; float_count()].into_boxed_slice(),
            transforms: vec![Vec2::ZERO; transform_count()].into_boxed_slice(),
            vectors: vec![None; vector_count()].into_boxed_slice(),
        }
    }

    pub fn set_bool(&mut self, label: &'static str, v: bool) {
        if let Some(i) = bool_index(label) {
            self.bools[i as usize] = v;
        }
    }

    pub fn set_float(&mut self, label: &'static str, v: f32) {
        if let Some(i) = float_index(label) {
            self.floats[i as usize] = v;
        }
    }

    pub fn set_transform(&mut self, label: &'static str, v: Vec2) {
        if let Some(i) = transform_index(label) {
            self.transforms[i as usize] = v;
        }
    }

    pub fn set_vector(&mut self, label: &'static str, v: Option<Vec2>) {
        if let Some(i) = vector_index(label) {
            self.vectors[i as usize] = v;
        }
    }

    pub fn get_bool_id(&self, id: u16) -> Option<bool> {
        self.bools.get(id as usize).copied()
    }
    pub fn get_float_id(&self, id: u16) -> Option<f32> {
        self.floats.get(id as usize).copied()
    }
    pub fn get_transform_id(&self, id: u16) -> Option<Vec2> {
        self.transforms.get(id as usize).copied()
    }
    pub fn get_vector_id(&self, id: u16) -> Option<Option<Vec2>> {
        self.vectors.get(id as usize).copied()
    }

    pub fn get_bool(&self, label: &str) -> Option<bool> {
        bool_index(label).and_then(|i| self.get_bool_id(i))
    }
    pub fn get_float(&self, label: &str) -> Option<f32> {
        float_index(label).and_then(|i| self.get_float_id(i))
    }
    pub fn get_transform(&self, label: &str) -> Option<Vec2> {
        transform_index(label).and_then(|i| self.get_transform_id(i))
    }
    pub fn get_vector3(&self, label: &str) -> Option<Option<Vec2>> {
        vector_index(label).and_then(|i| self.get_vector_id(i))
    }

    /// Fill from legacy HashMaps (snapshot build / tests).
    pub fn from_maps(
        team: crate::brain::TeamId,
        bools: &HashMap<&'static str, bool>,
        floats: &HashMap<&'static str, f32>,
        transforms: &HashMap<&'static str, Vec2>,
        vectors: &HashMap<&'static str, Option<Vec2>>,
    ) -> Self {
        let mut api = Self::empty(team);
        for (k, v) in bools {
            api.set_bool(k, *v);
        }
        for (k, v) in floats {
            api.set_float(k, *v);
        }
        for (k, v) in transforms {
            api.set_transform(k, *v);
        }
        for (k, v) in vectors {
            api.set_vector(k, *v);
        }
        api
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_labels_index() {
        assert!(bool_index("Is Home Team").is_some());
        assert!(float_index("Ball Speed").is_some());
        assert!(float_index("Field Width").is_some());
        assert!(transform_index("Ball").is_some());
        assert!(vector_index("Ball Velocity").is_some());
        assert_eq!(bool_index("not a real label"), None);
    }
}
