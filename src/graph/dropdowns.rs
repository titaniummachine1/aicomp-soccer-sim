//! Auto-generated SoccerGet* dropdown labels (index -> label).
//! Source: AIGamePyLibrary.data.DROPDOWN_OPTIONS
//! Regenerate: python scripts/gen_graph_dropdowns.py
//!
//! RelativePosition table below is hand-maintained (not in SoccerGet* regen).

use bevy::prelude::Vec2;

pub const SOCCER_GET_BOOL: &[&str] = &[
    "Team Has Ball",
    "Opponent Has Ball",
    "Is Ball Loose",
    "Team Player 1 Has Ball",
    "Team Player 2 Has Ball",
    "Team Player 3 Has Ball",
    "Team Player 4 Has Ball",
    "Opponent Player 1 Has Ball",
    "Opponent Player 2 Has Ball",
    "Opponent Player 3 Has Ball",
    "Opponent Player 4 Has Ball",
    "Is Ball Nearby Team Player 1",
    "Is Ball Nearby Team Player 2",
    "Is Ball Nearby Team Player 3",
    "Is Ball Nearby Team Player 4",
    "Is Ball Nearby Opponent Player 1",
    "Is Ball Nearby Opponent Player 2",
    "Is Ball Nearby Opponent Player 3",
    "Is Ball Nearby Opponent Player 4",
    "Is Team Player 1 Closest Teammate to Ball",
    "Is Team Player 2 Closest Teammate to Ball",
    "Is Team Player 3 Closest Teammate to Ball",
    "Is Team Player 4 Closest Teammate to Ball",
    "Is Opponent Player 1 Closest Opponent to Ball",
    "Is Opponent Player 2 Closest Opponent to Ball",
    "Is Opponent Player 3 Closest Opponent to Ball",
    "Is Opponent Player 4 Closest Opponent to Ball",
    "Is Team Player 1 Open",
    "Is Team Player 2 Open",
    "Is Team Player 3 Open",
    "Is Team Player 4 Open",
    "Is Opponent Player 1 Open",
    "Is Opponent Player 2 Open",
    "Is Opponent Player 3 Open",
    "Is Opponent Player 4 Open",
    "Is Kickoff",
    "Is Team Kicking off",
    "Is Opponent Kicking off",
    "Team Is Winning",
    "Opponent Is Winning",
    "Team Scored Last Point",
    "Opponent Scored Last Point",
    "Ball On Team Side",
    "Ball On Opponent Side",
    "Is Ball Headed Towards Team Goal",
    "Is Ball Headed Towards Opponent Goal",
    "Is Home Team",
    "Is Away Team",
    "Is Active Graph",
];

pub const SOCCER_GET_FLOAT: &[&str] = &[
    "Team Score",
    "Opponent Score",
    "Team Shots",
    "Opponent Shots",
    "Team Possession %",
    "Opponent Possession %",
    "Team Attacking %",
    "Opponent Attacking %",
    "Ball Speed",
    "Player Interact Radius",
    "Player With Ball Shot Charge %",
    "Ball Carrier Stamina",
    "Ball Carrier Shot Charge",
    "Teammate 1 Shot Charge",
    "Teammate 2 Shot Charge",
    "Teammate 3 Shot Charge",
    "Teammate 4 Shot Charge",
    "Field Width",
    "Field Depth",
    "Kickoff Circle Radius",
    "Goal Width",
    "Goal Height",
    "Team Player 1 Stamina",
    "Team Player 2 Stamina",
    "Team Player 3 Stamina",
    "Team Player 4 Stamina",
    "Distance from Team Player 1 to nearest Opponent",
    "Distance from Team Player 2 to nearest Opponent",
    "Distance from Team Player 3 to nearest Opponent",
    "Distance from Team Player 4 to nearest Opponent",
    "Opponent Player 1 Stamina",
    "Opponent Player 2 Stamina",
    "Opponent Player 3 Stamina",
    "Opponent Player 4 Stamina",
    "Opponent Nearest Teammate Player 1 Stamina",
    "Opponent Nearest Teammate Player 2 Stamina",
    "Opponent Nearest Teammate Player 3 Stamina",
    "Opponent Nearest Teammate Player 4 Stamina",
    "Stamina of last defending opponent",
    "Current Simulation Time",
    "Max Simulation Time",
    "Simulation Time Remaining",
    "Delta Time",
    "Fixed Delta Time",
    "Pi",
];

pub const SOCCER_GET_TRANSFORM: &[&str] = &[
    "Ball",
    "Team Player 1",
    "Team Player 2",
    "Team Player 3",
    "Team Player 4",
    "Opponent Player 1",
    "Opponent Player 2",
    "Opponent Player 3",
    "Opponent Player 4",
    "Teammate Nearest Team Player 1",
    "Teammate Nearest Team Player 2",
    "Teammate Nearest Team Player 3",
    "Teammate Nearest Team Player 4",
    "Opponent Nearest Team Player 1",
    "Opponent Nearest Team Player 2",
    "Opponent Nearest Team Player 3",
    "Opponent Nearest Team Player 4",
    "Team Goal Center",
    "Team Goal Left Post",
    "Team Goal Right Post",
    "Opponent Goal Center",
    "Opponent Goal Left Post",
    "Opponent Goal Right Post",
    "Opponent Nearest Team Goal",
    "Opponent Nearest Opponent Goal",
    "Teammate Nearest Team Goal",
    "Teammate Nearest Opponent Goal",
];

pub const SOCCER_GET_VECTOR3: &[&str] = &[
    "Ball Velocity",
    "Clear direction from team carrier",
    "Backwards clear direction from team carrier",
    "Clear direction from team carrier (avoid goal lines)",
    "Clear direction from team carrier (avoid sidelines)",
    "Clear direction from team carrier (avoid all walls)",
    "Upper Corner Home Side",
    "Lower Corner Home Side",
    "Upper Midfield",
    "Lower Midfield",
    "Upper Corner Away Side",
    "Lower Corner Away Side",
    "Upper Corner Opposing Side",
    "Lower Corner Opposing Side",
    "Upper Corner Team Side",
    "Lower Corner Team Side",
    "Center Field",
    "Get nearest open teammate",
    "Get furthest open teammate",
    "Get most open teammate",
    "Get nearest open opponent",
    "Get furthest open opponent",
    "Get most open opponent",
    "Direction of clear teammate from Teammate 1",
    "Direction of clear teammate from Teammate 2",
    "Direction of clear teammate from Teammate 3",
    "Direction of clear teammate from Teammate 4",
    "Direction of clear teammate from Opponent 1",
    "Direction of clear teammate from Opponent 2",
    "Direction of clear teammate from Opponent 3",
    "Direction of clear teammate from Opponent 4",
    "Direction of ball from Teammate 1",
    "Direction of ball from Teammate 2",
    "Direction of ball from Teammate 3",
    "Direction of ball from Teammate 4",
    "Direction of ball from Opponent 1",
    "Direction of ball from Opponent 2",
    "Direction of ball from Opponent 3",
    "Direction of ball from Opponent 4",
    "Direction of team goal from Teammate 1",
    "Direction of team goal from Teammate 2",
    "Direction of team goal from Teammate 3",
    "Direction of team goal from Teammate 4",
    "Direction of opponent goal from Teammate 1",
    "Direction of opponent goal from Teammate 2",
    "Direction of opponent goal from Teammate 3",
    "Direction of opponent goal from Teammate 4",
    "Clear direction from Teammate 1",
    "Clear direction from Teammate 2",
    "Clear direction from Teammate 3",
    "Clear direction from Teammate 4",
    "Direction of teammate from Team Player 1",
    "Direction of teammate from Team Player 2",
    "Direction of teammate from Team Player 3",
    "Direction of teammate from Team Player 4",
];

pub fn resolve<'a>(node_id: &str, modifier: &'a str) -> &'a str {
    let opts: &[&str] = match node_id {
        "SoccerGetBool" => SOCCER_GET_BOOL,
        "SoccerGetFloat" => SOCCER_GET_FLOAT,
        "SoccerGetTransform" => SOCCER_GET_TRANSFORM,
        "SoccerGetVector3" => SOCCER_GET_VECTOR3,
        "RelativePosition" => RELATIVE_POSITION,
        // Unity saves Operation as dropdown index (AIGamePyLibrary.nodes.Operation).
        "Operation" => OPERATION,
        _ => return modifier,
    };
    if let Ok(i) = modifier.parse::<usize>() {
        if let Some(label) = opts.get(i) {
            return label;
        }
    }
    modifier
}

/// Unity Operation dropdown order (AIGamePyLibrary `Operation(...).index(value)`).
/// Index 10 = sqrt — not our old internal encoding where sqrt was 5.
pub const OPERATION: &[&str] = &[
    "abs",    // 0
    "round",  // 1
    "floor",  // 2
    "ceil",   // 3
    "sin",    // 4
    "cos",    // 5
    "tan",    // 6
    "asin",   // 7
    "acos",   // 8
    "atan",   // 9
    "sqrt",   // 10
    "sign",   // 11
    "ln",     // 12
    "log10",  // 13
    "e^",     // 14
    "10^",    // 15
];

/// Map Operation modifier (label or Unity dropdown index) → kind index.
///
/// **Hard-fails** on unknown values — silent identity fallback hid sqrt bugs
/// (Unity `sqrt` is `"10"`; unmatched → old kind 13 → returned the input).
pub fn operation_kind(modifier: &str) -> u32 {
    let m = modifier.trim();
    if let Ok(i) = m.parse::<usize>() {
        if i < OPERATION.len() {
            return i as u32;
        }
        panic!(
            "Operation modifier index {i} out of range 0..{} (Unity dropdown)",
            OPERATION.len()
        );
    }
    let key = m.to_ascii_lowercase();
    OPERATION
        .iter()
        .position(|label| *label == key || (*label == "sign" && key == "signum"))
        .map(|i| i as u32)
        .unwrap_or_else(|| {
            panic!(
                "unknown Operation modifier {modifier:?}; expected one of {OPERATION:?} \
                 or index 0..{}",
                OPERATION.len()
            )
        })
}

/// Evaluate Operation by Unity dropdown kind (see [`OPERATION`]).
pub fn eval_operation(a: f32, kind: u32) -> f32 {
    match kind {
        0 => a.abs(),
        1 => a.round(),
        2 => a.floor(),
        3 => a.ceil(),
        4 => a.sin(),
        5 => a.cos(),
        6 => a.tan(),
        7 => a.asin(),
        8 => a.acos(),
        9 => a.atan(),
        10 => a.max(0.0).sqrt(),
        11 => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        12 => a.ln(),
        13 => a.log10(),
        14 => a.exp(),
        15 => 10f32.powf(a),
        _ => panic!("Operation kind {kind} out of range (internal bug)"),
    }
}

/// AIGamePyLibrary RelativePosition dropdown (+ World as used in Soccer UI).
pub const RELATIVE_POSITION: &[&str] = &[
    "Self",
    "Self + Forward",
    "Self + Backward",
    "Self + Left",
    "Self + Right",
    "Self + Up",
    "Self + Down",
    "Forward",
    "Backward",
    "Left",
    "Right",
    "Up",
    "Down",
    "World",
];

/// Pitch-plane meaning of a RelativePosition mode.
/// Forward = +X (goal axis); Left = +Z = our Y (sideline). Up/Down ignored in 2D.
/// No per-transform facing in TeamApi — offsets use world axes (good enough for Self/World).
pub fn relative_position_mode(mode: &str) -> RelativePosMode {
    match mode.trim() {
        "" | "Self" | "World" | "0" | "13" => RelativePosMode::WorldPos,
        "Self + Forward" | "1" => RelativePosMode::PosPlus(Vec2::X),
        "Self + Backward" | "2" => RelativePosMode::PosPlus(-Vec2::X),
        "Self + Left" | "3" => RelativePosMode::PosPlus(Vec2::Y),
        "Self + Right" | "4" => RelativePosMode::PosPlus(-Vec2::Y),
        "Self + Up" | "5" | "Self + Down" | "6" => RelativePosMode::WorldPos,
        "Forward" | "7" => RelativePosMode::DirOnly(Vec2::X),
        "Backward" | "8" => RelativePosMode::DirOnly(-Vec2::X),
        "Left" | "9" => RelativePosMode::DirOnly(Vec2::Y),
        "Right" | "10" => RelativePosMode::DirOnly(-Vec2::Y),
        "Up" | "11" | "Down" | "12" => RelativePosMode::DirOnly(Vec2::ZERO),
        _ => RelativePosMode::WorldPos,
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RelativePosMode {
    /// Transform world position (Self / World).
    WorldPos,
    /// Position + world-axis unit offset.
    PosPlus(Vec2),
    /// Direction only (unit or zero).
    DirOnly(Vec2),
}

pub fn apply_relative_position(pos: Vec2, mode: &str) -> Vec2 {
    match relative_position_mode(mode) {
        RelativePosMode::WorldPos => pos,
        RelativePosMode::PosPlus(d) => pos + d,
        RelativePosMode::DirOnly(d) => d,
    }
}

#[cfg(test)]
mod relative_pos_tests {
    use super::*;

    #[test]
    fn self_and_world_are_world_pos() {
        let p = Vec2::new(3.0, 4.0);
        assert_eq!(apply_relative_position(p, "Self"), p);
        assert_eq!(apply_relative_position(p, "World"), p);
        assert_eq!(apply_relative_position(p, "13"), p);
    }

    #[test]
    fn self_plus_forward_offsets_goal_axis() {
        let p = Vec2::new(10.0, 0.0);
        assert_eq!(apply_relative_position(p, "Self + Forward"), Vec2::new(11.0, 0.0));
        assert_eq!(apply_relative_position(p, "Forward"), Vec2::X);
    }

    #[test]
    fn unity_operation_index_10_is_sqrt() {
        assert_eq!(resolve("Operation", "10"), "sqrt");
        assert_eq!(operation_kind("10"), 10);
        assert_eq!(operation_kind("sqrt"), 10);
        let got = eval_operation(2.0, operation_kind("10"));
        assert!((got - 2.0_f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn unity_operation_index_5_is_cos_not_sqrt() {
        assert_eq!(resolve("Operation", "5"), "cos");
        assert_eq!(operation_kind("5"), 5);
        let got = eval_operation(0.0, 5);
        assert!((got - 1.0).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "unknown Operation modifier")]
    fn unknown_operation_modifier_panics() {
        let _ = operation_kind("nope");
    }
}
