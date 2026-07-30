//! Dense SoccerGet catalogs — label ↔ `u16` index (no per-tick string HashMaps
//! on the RuntimeBrain LoadApi hot path).

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::prelude::Vec2;

use super::labels::{
    GET_BOOL, GET_FLOAT, GET_FLOAT_FIELD_MARKS, GET_TRANSFORM, GET_VECTOR3,
};

fn float_catalog() -> &'static [&'static str] {
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

/// Tracks which API fields a brain's graph actually reads.
/// Built once from the compiled ApiSlotTable, reused every tick.
#[derive(Debug, Clone)]
pub struct ApiFieldMask {
    bools: Box<[bool]>,
    floats: Box<[bool]>,
    transforms: Box<[bool]>,
    vectors: Box<[bool]>,
}

/// Kind tag for `ApiFieldMask::from_dense_ids` — mirrors `graph_vm::lower::ApiKind`
/// without the cross-module dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskKind {
    Bool,
    Float,
    Transform,
    Vector3,
}

impl ApiFieldMask {
    pub fn all() -> Self {
        Self {
            bools: vec![true; bool_count()].into_boxed_slice(),
            floats: vec![true; float_count()].into_boxed_slice(),
            transforms: vec![true; transform_count()].into_boxed_slice(),
            vectors: vec![true; vector_count()].into_boxed_slice(),
        }
    }

    pub fn all_static() -> &'static Self {
        static M: OnceLock<ApiFieldMask> = OnceLock::new();
        M.get_or_init(Self::all)
    }

    pub fn none() -> Self {
        Self {
            bools: vec![false; bool_count()].into_boxed_slice(),
            floats: vec![false; float_count()].into_boxed_slice(),
            transforms: vec![false; transform_count()].into_boxed_slice(),
            vectors: vec![false; vector_count()].into_boxed_slice(),
        }
    }

    pub fn from_dense_ids(ids: &[(MaskKind, u16)]) -> Self {
        let mut m = Self::none();
        for &(kind, id) in ids {
            if id == UNKNOWN_ID {
                continue;
            }
            match kind {
                MaskKind::Bool => {
                    if let Some(slot) = m.bools.get_mut(id as usize) {
                        *slot = true;
                    }
                }
                MaskKind::Float => {
                    if let Some(slot) = m.floats.get_mut(id as usize) {
                        *slot = true;
                    }
                }
                MaskKind::Transform => {
                    if let Some(slot) = m.transforms.get_mut(id as usize) {
                        *slot = true;
                    }
                }
                MaskKind::Vector3 => {
                    if let Some(slot) = m.vectors.get_mut(id as usize) {
                        *slot = true;
                    }
                }
            }
        }
        m
    }

    pub fn needs_bool(&self, label: &str) -> bool {
        bool_index(label)
            .map(|i| self.bools[i as usize])
            .unwrap_or(false)
    }
    pub fn needs_bool_id(&self, id: u16) -> bool {
        if id == UNKNOWN_ID { return false; }
        self.bools.get(id as usize).copied().unwrap_or(false)
    }
    pub fn needs_float(&self, label: &str) -> bool {
        float_index(label)
            .map(|i| self.floats[i as usize])
            .unwrap_or(false)
    }
    pub fn needs_float_id(&self, id: u16) -> bool {
        if id == UNKNOWN_ID { return false; }
        self.floats.get(id as usize).copied().unwrap_or(false)
    }
    pub fn needs_transform(&self, label: &str) -> bool {
        transform_index(label)
            .map(|i| self.transforms[i as usize])
            .unwrap_or(false)
    }
    pub fn needs_transform_id(&self, id: u16) -> bool {
        if id == UNKNOWN_ID { return false; }
        self.transforms.get(id as usize).copied().unwrap_or(false)
    }
    pub fn needs_vector(&self, label: &str) -> bool {
        vector_index(label)
            .map(|i| self.vectors[i as usize])
            .unwrap_or(false)
    }
    pub fn needs_vector_id(&self, id: u16) -> bool {
        if id == UNKNOWN_ID { return false; }
        self.vectors.get(id as usize).copied().unwrap_or(false)
    }

    pub fn needs_bool_set(&mut self, label: &str) {
        if let Some(i) = bool_index(label) {
            if let Some(slot) = self.bools.get_mut(i as usize) {
                *slot = true;
            }
        }
    }
    pub fn needs_float_set(&mut self, label: &str) {
        if let Some(i) = float_index(label) {
            if let Some(slot) = self.floats.get_mut(i as usize) {
                *slot = true;
            }
        }
    }
    pub fn needs_transform_set(&mut self, label: &str) {
        if let Some(i) = transform_index(label) {
            if let Some(slot) = self.transforms.get_mut(i as usize) {
                *slot = true;
            }
        }
    }
    pub fn needs_vector_set(&mut self, label: &str) {
        if let Some(i) = vector_index(label) {
            if let Some(slot) = self.vectors.get_mut(i as usize) {
                *slot = true;
            }
        }
    }

    pub fn needs_any_vector(&self) -> bool {
        self.vectors.iter().any(|&v| v)
    }
    pub fn needs_any_transform(&self) -> bool {
        self.transforms.iter().any(|&v| v)
    }
    pub fn needs_any_bool(&self) -> bool {
        self.bools.iter().any(|&v| v)
    }
    pub fn needs_any_float(&self) -> bool {
        self.floats.iter().any(|&v| v)
    }
}

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

    pub fn set_bool_id(&mut self, id: u16, v: bool) {
        if let Some(slot) = self.bools.get_mut(id as usize) {
            *slot = v;
        }
    }

    pub fn set_float(&mut self, label: &'static str, v: f32) {
        if let Some(i) = float_index(label) {
            self.floats[i as usize] = v;
        }
    }

    pub fn set_float_id(&mut self, id: u16, v: f32) {
        if let Some(slot) = self.floats.get_mut(id as usize) {
            *slot = v;
        }
    }

    pub fn set_transform(&mut self, label: &'static str, v: Vec2) {
        if let Some(i) = transform_index(label) {
            self.transforms[i as usize] = v;
        }
    }

    pub fn set_transform_id(&mut self, id: u16, v: Vec2) {
        if let Some(slot) = self.transforms.get_mut(id as usize) {
            *slot = v;
        }
    }

    pub fn set_vector(&mut self, label: &'static str, v: Option<Vec2>) {
        if let Some(i) = vector_index(label) {
            self.vectors[i as usize] = v;
        }
    }

    pub fn set_vector_id(&mut self, id: u16, v: Option<Vec2>) {
        if let Some(slot) = self.vectors.get_mut(id as usize) {
            *slot = v;
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
