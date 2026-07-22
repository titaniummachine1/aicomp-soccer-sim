//! ApiProbe / AIA_Debug-compatible TimePlot recorder (pitch XZ only).
//!
//! Also emits one-sample `Event.*` pulses (score / phase / possession / kick /
//! steal) so a single JSON shows how the match unfolded.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use bevy::prelude::Vec2;
use serde::Serialize;

use crate::api::TeamApi;
use crate::brain::{BrainOutput, TeamId};
use crate::match_state::MatchPhase;
use crate::player::PlayerId;
use crate::world::MatchWorld;

#[derive(Debug, Default)]
pub struct TimePlotRecorder {
    sim_time: f32,
    /// name → (color, xs, ys)
    series: BTreeMap<String, SeriesBuf>,
    prev: Option<PlotPrev>,
}

#[derive(Debug, Clone)]
struct PlotPrev {
    score_home: u32,
    score_away: u32,
    phase: MatchPhase,
    /// 0=loose, 1=home, 2=away
    poss: u8,
    home_has: bool,
    home_charge: f32,
    carrier: Option<(TeamId, u8)>,
}

#[derive(Debug, Default)]
struct SeriesBuf {
    color: String,
    x: Vec<f32>,
    y: Vec<f32>,
}

#[derive(Serialize)]
struct TimePlotFile {
    #[serde(rename = "simTime")]
    sim_time: f32,
    series: Vec<TimePlotSeries>,
}

#[derive(Serialize)]
struct TimePlotSeries {
    name: String,
    color: String,
    x: Vec<f32>,
    y: Vec<f32>,
}

impl TimePlotRecorder {
    pub fn sample_home(
        &mut self,
        world: &MatchWorld,
        home_api: &TeamApi,
        home_out: &BrainOutput,
        dt: f32,
    ) {
        self.sim_time += dt;
        let t = self.sim_time;

        // World positions — Home view: T*=Home, O*=Away (matches AIA_Debug as home).
        push_xyz(self, "Ball", "#FFFFFF", world.ball.pos, t);
        push_xyz(self, "BallVel", "#FF0000", world.ball.vel, t);
        for id in PlayerId::ALL {
            let p = world
                .players
                .iter()
                .find(|p| p.team == TeamId::Home && p.id == id);
            if let Some(p) = p {
                push_xyz(self, &format!("T{}", id.0), "#00FFFF", p.pos, t);
            }
            let o = world
                .players
                .iter()
                .find(|p| p.team == TeamId::Away && p.id == id);
            if let Some(o) = o {
                push_xyz(self, &format!("O{}", id.0), "#FFA500", o.pos, t);
            }
        }
        if let Some(g) = home_api.get_transform("Team Goal Center") {
            push_xyz(self, "TeamGoal", "#00FF00", g, t);
        }
        if let Some(g) = home_api.get_transform("Opponent Goal Center") {
            push_xyz(self, "OppGoal", "#FF0000", g, t);
        }

        // Controller outputs (same names as AIA_Debug).
        let roles = [
            ("Striker", 1u8),
            ("Playmaker", 2),
            ("Defender", 3),
            ("Goalie", 4),
        ];
        for (role, pid) in roles {
            let cmd = home_out.for_player(PlayerId(pid));
            push_xyz(
                self,
                &format!("Ctrl.{role}.MoveTo"),
                "#00FF00",
                cmd.move_to,
                t,
            );
            push_f(
                self,
                &format!("Ctrl.{role}.Interact"),
                "#00FF00",
                if cmd.interact { 1.0 } else { 0.0 },
                t,
            );
        }
        push_f(
            self,
            "Ctrl.Striker.Sprint",
            "#00FF00",
            if home_out.for_player(PlayerId(1)).sprint {
                1.0
            } else {
                0.0
            },
            t,
        );
        push_f(
            self,
            "Ctrl.Defender.Sprint",
            "#00FF00",
            if home_out.for_player(PlayerId(3)).sprint {
                1.0
            } else {
                0.0
            },
            t,
        );

        // SoccerGet bools / floats (AIA_Debug In.* naming).
        for (label, v) in &home_api.bools {
            let safe = label.replace(' ', "_");
            push_f(
                self,
                &format!("In.Bool.{safe}"),
                "#FFA500",
                if *v { 1.0 } else { 0.0 },
                t,
            );
        }
        for (label, v) in &home_api.floats {
            let safe = label.replace(' ', "_");
            push_f(self, &format!("In.Float.{safe}"), "#FFA500", *v, t);
        }

        // Aim.* mirrors AIA_Debug (Present + XZ) for clear/goal dirs.
        for id in PlayerId::ALL {
            let n = id.0;
            push_aim_vec(
                self,
                &format!("Aim.OppGoal.T{n}"),
                home_api.get_vector3(&format!(
                    "Direction of opponent goal from Teammate {n}"
                )),
                t,
            );
            push_aim_vec(
                self,
                &format!("Aim.ClearMate.T{n}"),
                home_api.get_vector3(&format!(
                    "Direction of clear teammate from Teammate {n}"
                )),
                t,
            );
            push_aim_vec(
                self,
                &format!("Clear.T{n}"),
                home_api.get_vector3(&format!("Clear direction from Teammate {n}")),
                t,
            );
        }
    push_aim_vec(
        self,
        "Clear.Carrier",
        home_api.get_vector3("Clear direction from team carrier"),
        t,
    );
    push_aim_vec(
        self,
        "Clear.CarrierAvoidAll",
        home_api.get_vector3("Clear direction from team carrier (avoid all walls)"),
        t,
    );
    push_aim_vec(
        self,
        "Clear.CarrierAvoidGoals",
        home_api.get_vector3("Clear direction from team carrier (avoid goal lines)"),
        t,
    );
    push_aim_vec(
        self,
        "Clear.CarrierAvoidSides",
        home_api.get_vector3("Clear direction from team carrier (avoid sidelines)"),
        t,
    );
    push_aim_vec(
        self,
        "Clear.CarrierBack",
        home_api.get_vector3("Backwards clear direction from team carrier"),
        t,
    );

        // Timing / score mirrors.
        push_f(self, "SimTime", "#808080", t, t);
        push_f(self, "FixedDt", "#808080", dt, t);
        push_f(
            self,
            "TeamScore",
            "#FFFFFF",
            world.match_state.score_home as f32,
            t,
        );
        push_f(
            self,
            "OppScore",
            "#FFFFFF",
            world.match_state.score_away as f32,
            t,
        );

        let home_has = home_api
            .get_bool("Team Has Ball")
            .unwrap_or(false);
        let opp_has = home_api
            .get_bool("Opponent Has Ball")
            .unwrap_or(false);
        let poss = if home_has {
            1u8
        } else if opp_has {
            2u8
        } else {
            0u8
        };
        let home_charge = home_api
            .get_float("Ball Carrier Shot Charge")
            .or_else(|| home_api.get_float("Teammate 1 Shot Charge"))
            .unwrap_or(0.0);
        let carrier = world.possession.carrier;
        let phase = world.match_state.phase;
        let sh = world.match_state.score_home;
        let sa = world.match_state.score_away;

        // Continuous event-ish channels (also in Unity AIA_Debug DB31+).
        push_f(self, "Event.PossCode", "#FFFF00", poss as f32, t);
        push_f(
            self,
            "Event.PhaseCode",
            "#FFFF00",
            match phase {
                MatchPhase::Kickoff => 0.0,
                MatchPhase::Play => 1.0,
                MatchPhase::GoalPause => 2.0,
            },
            t,
        );

        // One-sample pulses (0 most frames, 1 on edge).
        let mut goal_h = 0.0;
        let mut goal_a = 0.0;
        let mut kickoff = 0.0;
        let mut play = 0.0;
        let mut pause = 0.0;
        let mut poss_team = 0.0;
        let mut poss_opp = 0.0;
        let mut poss_loose = 0.0;
        let mut kick_rel = 0.0;
        let mut steal = 0.0;

        if let Some(prev) = &self.prev {
            if sh > prev.score_home {
                goal_h = 1.0;
            }
            if sa > prev.score_away {
                goal_a = 1.0;
            }
            if phase != prev.phase {
                match phase {
                    MatchPhase::Kickoff => kickoff = 1.0,
                    MatchPhase::Play => play = 1.0,
                    MatchPhase::GoalPause => pause = 1.0,
                }
            }
            if poss != prev.poss {
                match poss {
                    1 => poss_team = 1.0,
                    2 => poss_opp = 1.0,
                    _ => poss_loose = 1.0,
                }
            }
            // Kick release: team held ball with charge, then lost possession.
            if prev.home_has && !home_has && prev.home_charge >= 0.15 {
                kick_rel = 1.0;
            }
            // Steal: carrier team flips without a charged release this tick.
            if let (Some((pt, _)), Some((ct, _))) = (prev.carrier, carrier) {
                if pt != ct && kick_rel < 0.5 {
                    steal = 1.0;
                }
            }
        }

        push_f(self, "Event.Goal.Home", "#00FF00", goal_h, t);
        push_f(self, "Event.Goal.Away", "#FF0000", goal_a, t);
        push_f(self, "Event.Kickoff", "#FFFFFF", kickoff, t);
        push_f(self, "Event.Play", "#808080", play, t);
        push_f(self, "Event.GoalPause", "#FF00FF", pause, t);
        push_f(self, "Event.Poss.Team", "#00FFFF", poss_team, t);
        push_f(self, "Event.Poss.Opp", "#FFA500", poss_opp, t);
        push_f(self, "Event.Poss.Loose", "#808080", poss_loose, t);
        push_f(self, "Event.Kick.Release", "#FF00FF", kick_rel, t);
        push_f(self, "Event.Steal", "#FFFF00", steal, t);

        self.prev = Some(PlotPrev {
            score_home: sh,
            score_away: sa,
            phase,
            poss,
            home_has,
            home_charge,
            carrier,
        });
    }

    pub fn write_json(&self, path: &Path) -> Result<(), String> {
        let series: Vec<TimePlotSeries> = self
            .series
            .iter()
            .map(|(name, s)| TimePlotSeries {
                name: name.clone(),
                color: s.color.clone(),
                x: s.x.clone(),
                y: s.y.clone(),
            })
            .collect();
        let file = TimePlotFile {
            sim_time: self.sim_time,
            series,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string(&file).map_err(|e| e.to_string())?;
        fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn sim_time(&self) -> f32 {
        self.sim_time
    }
}

fn push_xyz(rec: &mut TimePlotRecorder, prefix: &str, color: &str, v: Vec2, t: f32) {
    // Unity Y height unused in sim — log 0 so channels exist for compare.
    push_f(rec, &format!("{prefix}.X"), color, v.x, t);
    push_f(rec, &format!("{prefix}.Y"), "#808080", 0.0, t);
    push_f(rec, &format!("{prefix}.Z"), color, v.y, t);
}

fn push_aim_vec(
    rec: &mut TimePlotRecorder,
    prefix: &str,
    v: Option<Option<Vec2>>,
    t: f32,
) {
    let present = matches!(v, Some(Some(_)));
    push_f(
        rec,
        &format!("{prefix}.Present"),
        "#00FF00",
        if present { 1.0 } else { 0.0 },
        t,
    );
    let vec = match v {
        Some(Some(v)) => v,
        _ => Vec2::ZERO,
    };
    push_f(rec, &format!("{prefix}.X"), "#00FF00", vec.x, t);
    push_f(rec, &format!("{prefix}.Y"), "#808080", 0.0, t);
    push_f(rec, &format!("{prefix}.Z"), "#00FF00", vec.y, t);
}

fn push_f(rec: &mut TimePlotRecorder, name: &str, color: &str, y: f32, t: f32) {
    let s = rec.series.entry(name.to_string()).or_insert_with(|| SeriesBuf {
        color: color.to_string(),
        x: Vec::new(),
        y: Vec::new(),
    });
    s.x.push(t);
    s.y.push(y);
}
