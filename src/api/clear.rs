//! 8-way clear-direction sensors (AIComp order).
//!
//! AIA Spherecast Floats (LOCKED): radius **0.25**, distance **20**.
//! HitInfo ≈ Unity RaycastHit (hit / distance / collider.tag); miss → ∞.

use bevy::prelude::*;

/// AIA graph Spherecast radius (not `Ball Radius` ≈0.406).
pub const SPHERECAST_RADIUS: f32 = 0.25;
/// AIA graph Spherecast max distance.
pub const SPHERECAST_DISTANCE: f32 = 20.0;

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

/// "Direction of opponent/team goal from Teammate N" — **not** raw aim at the
/// center. Real TimePlots show these are usually null; when present they are
/// 8-way unit dirs whose ray reaches the goal mouth unobstructed.
///
/// AIA's Set First Direction prefers this over clear-dir; always returning a
/// vector made carriers march straight at goal (Ball.Z≈0).
pub fn clear_dir_into_goal_mouth(
    origin: Vec2,
    toward_home_goal: bool,
    blockers: &[Vec2],
    blocker_r: f32,
    goal_line_x: f32,
    goal_half_width: f32,
) -> Option<Vec2> {
    let goal_x = if toward_home_goal {
        -goal_line_x.abs()
    } else {
        goal_line_x.abs()
    };
    // Prefer the attacking team's clear order (Home order when shooting +X).
    let shooting_home_side = !toward_home_goal;
    for dir in clear_order(shooting_home_side) {
        let u = dir.unit();
        if u.x.abs() < 1e-4 {
            continue;
        }
        let t = (goal_x - origin.x) / u.x;
        if t <= 0.05 {
            continue;
        }
        let z_hit = origin.y + u.y * t;
        if z_hit.abs() > goal_half_width {
            continue;
        }
        // Path to the mouth must be clear (no sideline/goal-line avoid — the
        // target *is* the goal line).
        if ray_clear(
            origin,
            u,
            t,
            blockers,
            blocker_r,
            false,
            false,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
        ) {
            return Some(u);
        }
    }
    None
}

/// "Direction of clear teammate from Teammate N" — 8-way unit dir toward a
/// teammate only when that lane is unobstructed. Real Present ~28–92% by role
/// (T2≈62%); raw dir-to-nearest always-Some made Ready fire at thresh and
/// dump weak kicks.
///
/// Requires LOS to the mate body, an 8-way snap within MIN_DOT, and the
/// snapped corridor itself clear out to the mate (rejects "glancing" mates
/// that LOS through a gap but whose octant ray clips a blocker).
pub fn clear_dir_toward_teammate(
    origin: Vec2,
    teammates: &[Vec2],
    blockers: &[Vec2],
    blocker_r: f32,
) -> Option<Vec2> {
    let all = [
        SensorDir::E,
        SensorDir::C,
        SensorDir::H,
        SensorDir::B,
        SensorDir::G,
        SensorDir::A,
        SensorDir::F,
        SensorDir::D,
    ];
    // Half-wedge cos(22.5°)≈0.924. Slightly stricter than half-wedge to cut
    // false T1/T2 Present (real T1≈0.28 / T2≈0.62) without cratering T3.
    const MIN_DOT: f32 = 0.93;
    let mut best: Option<(f32, Vec2)> = None;
    for (mate_i, &mate) in teammates.iter().enumerate() {
        let delta = mate - origin;
        let dist = delta.length();
        if !(1.5..=36.0).contains(&dist) {
            continue;
        }
        let toward = delta / dist;
        let mut block: Vec<Vec2> = blockers.to_vec();
        for (j, &m) in teammates.iter().enumerate() {
            if j != mate_i {
                block.push(m);
            }
        }
        if !ray_clear(
            origin,
            toward,
            dist,
            &block,
            blocker_r,
            false,
            false,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
        ) {
            continue;
        }
        let mut best_u: Option<Vec2> = None;
        let mut best_dot = MIN_DOT;
        for d in all {
            let u = d.unit();
            let dot = u.dot(toward);
            if dot > best_dot {
                best_dot = dot;
                best_u = Some(u);
            }
        }
        let Some(u) = best_u else {
            continue;
        };
        // Short-range octant corridor only (long-range goalie lanes skip this).
        if dist <= 18.0
            && !ray_clear(
                origin,
                u,
                dist,
                &block,
                blocker_r,
                false,
                false,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
            )
        {
            continue;
        }
        best = match best {
            None => Some((dist, u)),
            Some((d, _)) if dist < d => Some((dist, u)),
            other => other,
        };
    }
    best.map(|(_, u)| u)
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
    // Continuous closest-approach vs each blocker (12 samples alone miss
    // near-miss geometry and flicker Clear C↔H frame-to-frame).
    for b in blockers {
        let delta = *b - origin;
        let t = delta.dot(dir).clamp(0.05, range);
        let closest = origin + dir * t;
        if (*b - closest).length() < blocker_r {
            return false;
        }
    }
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
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opp_goal_dir_null_at_center_when_midfield_blocked() {
        // O2 sits on +X — E lane into Away mouth is blocked ⇒ null at kickoff spot.
        let blockers = [Vec2::new(11.0, 0.0)];
        let d = clear_dir_into_goal_mouth(
            Vec2::ZERO,
            false,
            &blockers,
            1.75,
            39.5,
            6.0,
        );
        assert!(d.is_none(), "expected null, got {d:?}");
    }

    #[test]
    fn opp_goal_dir_e_when_near_mouth() {
        let d = clear_dir_into_goal_mouth(
            Vec2::new(35.0, -5.0),
            false,
            &[],
            1.75,
            39.5,
            6.0,
        );
        let d = d.expect("clear E into mouth");
        assert!((d.x - 1.0).abs() < 1e-3 && d.y.abs() < 1e-3);
    }

    #[test]
    fn clear_mate_null_when_blocked() {
        let mates = [Vec2::new(10.0, 0.0)];
        let blockers = [Vec2::new(5.0, 0.0)];
        let d = clear_dir_toward_teammate(Vec2::ZERO, &mates, &blockers, 1.0);
        assert!(d.is_none(), "expected null, got {d:?}");
    }

    #[test]
    fn clear_mate_e_when_open() {
        let mates = [Vec2::new(10.0, 0.0)];
        let d = clear_dir_toward_teammate(Vec2::ZERO, &mates, &[], 1.0);
        let d = d.expect("clear E to mate");
        assert!((d.x - 1.0).abs() < 1e-3 && d.y.abs() < 1e-3);
    }

    #[test]
    fn clear_mate_null_when_off_wedge() {
        // Mate almost exactly between E and B (north-east-ish but closer to
        // midpoint of non-adjacent dirs) — use a mate behind that no 8-way
        // with MIN_DOT=0.85 covers? Actually with 0.85, most angles snap.
        // Blocked mate is the reliable null case (covered above); this checks
        // a mate too close to count.
        let mates = [Vec2::new(1.0, 0.0)];
        let d = clear_dir_toward_teammate(Vec2::ZERO, &mates, &[], 1.0);
        assert!(d.is_none(), "expected null for cling-distance mate, got {d:?}");
    }
}
