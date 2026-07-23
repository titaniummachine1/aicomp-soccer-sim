//! Unity-style Keypress node state for the Bevy viewer.
//!
//! Headless / batch leave this empty → all Keypress reads are `false`.
//! Viewer calls [`set_pressed`] each frame from Bevy `ButtonInput<KeyCode>`.

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

/// Fixed catalog — RuntimeBrain encodes Keypress as an index into this table.
pub const UNITY_KEYS: &[&str] = &[
    "", // 0 = unknown / unmapped
    "W",
    "A",
    "S",
    "D",
    "E",
    "Q",
    "R",
    "F",
    "C",
    "X",
    "Z",
    "Space",
    "LeftShift",
    "RightShift",
    "LeftControl",
    "RightControl",
    "LeftAlt",
    "RightAlt",
    "UpArrow",
    "DownArrow",
    "LeftArrow",
    "RightArrow",
    "Escape",
    "Tab",
    "Return",
    "Backspace",
    "Alpha1",
    "Alpha2",
    "Alpha3",
    "Alpha4",
];

static PRESSED: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// Replace the pressed-key set (Unity Keypress modifier strings, e.g. `"W"`).
pub fn set_pressed(keys: impl IntoIterator<Item = impl AsRef<str>>) {
    let mut g = PRESSED.write().unwrap_or_else(|e| e.into_inner());
    g.clear();
    for k in keys {
        g.insert(normalize_key(k.as_ref()));
    }
}

pub fn clear() {
    set_pressed(std::iter::empty::<&str>());
}

pub fn is_pressed(modifier: &str) -> bool {
    let g = PRESSED.read().unwrap_or_else(|e| e.into_inner());
    let mut n = normalize_key(modifier);
    // Viewer: Space is pause/resume. Unity Controller graphs wire interact to
    // Space — remap those reads to E so shoot/tackle doesn't pause the match.
    if n == "Space" {
        n = "E".into();
    }
    g.contains(&n)
}

pub fn is_pressed_id(id: u32) -> bool {
    UNITY_KEYS
        .get(id as usize)
        .filter(|s| !s.is_empty())
        .map(|s| is_pressed(s))
        .unwrap_or(false)
}

/// Map Unity Keypress modifier → catalog id (0 = unknown → always false).
pub fn key_id(modifier: &str) -> u32 {
    let n = normalize_key(modifier);
    UNITY_KEYS
        .iter()
        .position(|k| *k == n)
        .unwrap_or(0) as u32
}

fn normalize_key(s: &str) -> String {
    let t = s.trim();
    match t {
        "Left Shift" | "Shift" | "LeftShift" => "LeftShift".into(),
        "Right Shift" | "RightShift" => "RightShift".into(),
        "Left Control" | "LeftControl" | "Left Ctrl" => "LeftControl".into(),
        "Right Control" | "RightControl" => "RightControl".into(),
        "Space" | "Spacebar" => "Space".into(),
        "UpArrow" | "Up Arrow" => "UpArrow".into(),
        "DownArrow" | "Down Arrow" => "DownArrow".into(),
        "LeftArrow" | "Left Arrow" => "LeftArrow".into(),
        "RightArrow" | "Right Arrow" => "RightArrow".into(),
        other => other.to_string(),
    }
}

/// Map Bevy key codes → Unity Keypress modifier names used in Soccer graphs.
pub fn bevy_key_name(code: bevy::prelude::KeyCode) -> Option<&'static str> {
    use bevy::prelude::KeyCode::*;
    Some(match code {
        KeyW => "W",
        KeyA => "A",
        KeyS => "S",
        KeyD => "D",
        KeyE => "E",
        KeyQ => "Q",
        KeyR => "R",
        KeyF => "F",
        KeyC => "C",
        KeyX => "X",
        KeyZ => "Z",
        Digit1 => "Alpha1",
        Digit2 => "Alpha2",
        Digit3 => "Alpha3",
        Digit4 => "Alpha4",
        Space => "Space",
        ShiftLeft => "LeftShift",
        ShiftRight => "RightShift",
        ControlLeft => "LeftControl",
        ControlRight => "RightControl",
        AltLeft => "LeftAlt",
        AltRight => "RightAlt",
        ArrowUp => "UpArrow",
        ArrowDown => "DownArrow",
        ArrowLeft => "LeftArrow",
        ArrowRight => "RightArrow",
        Escape => "Escape",
        Tab => "Tab",
        Enter => "Return",
        Backspace => "Backspace",
        _ => return None,
    })
}
