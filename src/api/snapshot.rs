//! Build team-relative API snapshots from the 2D world each tick.

use std::sync::OnceLock;

use bevy::prelude::*;
use super::clear::{clear_dir_into_goal_mouth, clear_dir_toward_teammate, first_clear_dir};
use super::{bool_index, float_index, transform_index, vector_index, ApiFieldMask, TeamApi, UNKNOWN_ID};
use crate::ball::Ball;
use crate::brain::TeamId;
use crate::match_state::{MatchPhase, MatchState};
use crate::params::SimParams;
use crate::player::{Player, PlayerId};
use crate::possession::Possession;

pub struct WorldSensors<'a> {
    pub ball: &'a Ball,
    pub players: &'a [Player],
    pub possession: &'a Possession,
    pub match_state: &'a MatchState,
    pub params: &'a SimParams,
}

fn all_mask() -> &'static ApiFieldMask {
    ApiFieldMask::all_static()
}

struct PrecomputedIds {
    bool_ids: [(u16, u16, u16, u16, u16, u16, u16, u16); 4],
    float_ids: [(u16, u16, u16, u16, u16); 4],
    xform_ids: [(u16, u16, u16, u16); 4],
    // Top-level (non-per-player) label indices.
    top_bools: TopBoolIds,
    top_floats: TopFloatIds,
    top_xforms: TopXformIds,
    // Per-player distances: [team_player][opponent_player] and [team_player][teammate_player].
    // dist_opp[pi][oi] = distance from Team Player pi+1 to Opponent oi+1.
    dist_opp: [[u16; 4]; 4],
    // dist_mate[pi][mi] = distance from Team Player pi+1 to Teammate mi+1 (mi != pi).
    // Stored as full 4x4 with UNKNOWN_ID on diagonal.
    dist_mate: [[u16; 4]; 4],
}

struct TopBoolIds {
    is_home: u16, is_away: u16, is_active: u16, is_loose: u16,
    team_has: u16, opp_has: u16, is_kickoff: u16,
    team_kicking: u16, opp_kicking: u16,
    ball_team_side: u16, ball_opp_side: u16,
    team_winning: u16, opp_winning: u16,
    team_scored: u16, opp_scored: u16,
    headed_team: u16, headed_opp: u16,
    can_pass: [u16; 4],
    opp_can_intercept: [u16; 4],
}

struct TopFloatIds {
    interact_r: u16, team_score: u16, opp_score: u16,
    field_w: u16, field_d: u16, kickoff_r: u16, goal_w: u16,
    goal_h: u16, pi: u16, ground_line: u16, area_d: u16, arena_d: u16,
    team_shots: u16, opp_shots: u16,
    poss_team: u16, poss_opp: u16,
    atk_team: u16, atk_opp: u16,
    ball_speed: u16, sim_time: u16, max_time: u16, time_left: u16,
    dt: u16, fixed_dt: u16,
    carrier_charge: u16, carrier_charge_pct: u16, carrier_stam: u16,
    last_def_stam: u16,
    best_intercept_slot: u16,
}

struct TopXformIds {
    ball: u16, team_goal: u16, opp_goal: u16,
    team_left: u16, team_right: u16,
    opp_left: u16, opp_right: u16,
    opp_nearest_team_goal: u16, opp_nearest_opp_goal: u16,
    mate_nearest_team_goal: u16, mate_nearest_opp_goal: u16,
    pass_dir: [u16; 4],
    ball_stop_pos: u16,
    intercept_pos: [u16; 4],
}

fn precomputed_ids() -> &'static PrecomputedIds {
    static IDS: OnceLock<PrecomputedIds> = OnceLock::new();
    IDS.get_or_init(|| {
        let resolve_bool = |n: u8| {
            let has = format!("Team Player {n} Has Ball");
            let near = format!("Is Ball Nearby Team Player {n}");
            let closest = format!("Is Team Player {n} Closest Teammate to Ball");
            let open = format!("Is Team Player {n} Open");
            let opp_open = format!("Is Opponent Player {n} Open");
            let opp_has = format!("Opponent Player {n} Has Ball");
            let opp_near = format!("Is Ball Nearby Opponent Player {n}");
            let opp_closest = format!("Is Opponent Player {n} Closest Opponent to Ball");
            (
                bool_index(&has).unwrap_or(UNKNOWN_ID),
                bool_index(&near).unwrap_or(UNKNOWN_ID),
                bool_index(&closest).unwrap_or(UNKNOWN_ID),
                bool_index(&open).unwrap_or(UNKNOWN_ID),
                bool_index(&opp_open).unwrap_or(UNKNOWN_ID),
                bool_index(&opp_has).unwrap_or(UNKNOWN_ID),
                bool_index(&opp_near).unwrap_or(UNKNOWN_ID),
                bool_index(&opp_closest).unwrap_or(UNKNOWN_ID),
            )
        };
        let resolve_float = |n: u8| {
            let stam = format!("Team Player {n} Stamina");
            let charge = format!("Teammate {n} Shot Charge");
            let opp_stam = format!("Opponent Player {n} Stamina");
            let dist = format!("Distance from Team Player {n} to nearest Opponent");
            let near_stam = format!("Opponent Nearest Teammate Player {n} Stamina");
            (
                float_index(&stam).unwrap_or(UNKNOWN_ID),
                float_index(&charge).unwrap_or(UNKNOWN_ID),
                float_index(&opp_stam).unwrap_or(UNKNOWN_ID),
                float_index(&dist).unwrap_or(UNKNOWN_ID),
                float_index(&near_stam).unwrap_or(UNKNOWN_ID),
            )
        };
        let resolve_xform = |n: u8| {
            let me = format!("Team Player {n}");
            let opp = format!("Opponent Player {n}");
            let near = format!("Teammate Nearest Team Player {n}");
            let near_opp = format!("Opponent Nearest Team Player {n}");
            (
                transform_index(&me).unwrap_or(UNKNOWN_ID),
                transform_index(&opp).unwrap_or(UNKNOWN_ID),
                transform_index(&near).unwrap_or(UNKNOWN_ID),
                transform_index(&near_opp).unwrap_or(UNKNOWN_ID),
            )
        };
        PrecomputedIds {
            bool_ids: [resolve_bool(1), resolve_bool(2), resolve_bool(3), resolve_bool(4)],
            float_ids: [resolve_float(1), resolve_float(2), resolve_float(3), resolve_float(4)],
            xform_ids: [resolve_xform(1), resolve_xform(2), resolve_xform(3), resolve_xform(4)],
            top_bools: TopBoolIds {
                is_home: bool_index("Is Home Team").unwrap_or(UNKNOWN_ID),
                is_away: bool_index("Is Away Team").unwrap_or(UNKNOWN_ID),
                is_active: bool_index("Is Active Graph").unwrap_or(UNKNOWN_ID),
                is_loose: bool_index("Is Ball Loose").unwrap_or(UNKNOWN_ID),
                team_has: bool_index("Team Has Ball").unwrap_or(UNKNOWN_ID),
                opp_has: bool_index("Opponent Has Ball").unwrap_or(UNKNOWN_ID),
                is_kickoff: bool_index("Is Kickoff").unwrap_or(UNKNOWN_ID),
                team_kicking: bool_index("Is Team Kicking off").unwrap_or(UNKNOWN_ID),
                opp_kicking: bool_index("Is Opponent Kicking off").unwrap_or(UNKNOWN_ID),
                ball_team_side: bool_index("Ball On Team Side").unwrap_or(UNKNOWN_ID),
                ball_opp_side: bool_index("Ball On Opponent Side").unwrap_or(UNKNOWN_ID),
                team_winning: bool_index("Team Is Winning").unwrap_or(UNKNOWN_ID),
                opp_winning: bool_index("Opponent Is Winning").unwrap_or(UNKNOWN_ID),
                team_scored: bool_index("Team Scored Last Point").unwrap_or(UNKNOWN_ID),
                opp_scored: bool_index("Opponent Scored Last Point").unwrap_or(UNKNOWN_ID),
                headed_team: bool_index("Is Ball Headed Towards Team Goal").unwrap_or(UNKNOWN_ID),
                headed_opp: bool_index("Is Ball Headed Towards Opponent Goal").unwrap_or(UNKNOWN_ID),
                can_pass: [
                    bool_index("Can Pass to Teammate 1").unwrap_or(UNKNOWN_ID),
                    bool_index("Can Pass to Teammate 2").unwrap_or(UNKNOWN_ID),
                    bool_index("Can Pass to Teammate 3").unwrap_or(UNKNOWN_ID),
                    bool_index("Can Pass to Teammate 4").unwrap_or(UNKNOWN_ID),
                ],
                opp_can_intercept: [
                    bool_index("Opponent Can Intercept Loose Ball 1").unwrap_or(UNKNOWN_ID),
                    bool_index("Opponent Can Intercept Loose Ball 2").unwrap_or(UNKNOWN_ID),
                    bool_index("Opponent Can Intercept Loose Ball 3").unwrap_or(UNKNOWN_ID),
                    bool_index("Opponent Can Intercept Loose Ball 4").unwrap_or(UNKNOWN_ID),
                ],
            },
            top_floats: TopFloatIds {
                interact_r: float_index("Player Interact Radius").unwrap_or(UNKNOWN_ID),
                team_score: float_index("Team Score").unwrap_or(UNKNOWN_ID),
                opp_score: float_index("Opponent Score").unwrap_or(UNKNOWN_ID),
                field_w: float_index("Field Width").unwrap_or(UNKNOWN_ID),
                field_d: float_index("Field Depth").unwrap_or(UNKNOWN_ID),
                kickoff_r: float_index("Kickoff Circle Radius").unwrap_or(UNKNOWN_ID),
                goal_w: float_index("Goal Width").unwrap_or(UNKNOWN_ID),
                goal_h: float_index("Goal Height").unwrap_or(UNKNOWN_ID),
                pi: float_index("Pi").unwrap_or(UNKNOWN_ID),
                ground_line: float_index("ground_line").unwrap_or(UNKNOWN_ID),
                area_d: float_index("Area Depth").unwrap_or(UNKNOWN_ID),
                arena_d: float_index("Arena Semicircle Depth").unwrap_or(UNKNOWN_ID),
                team_shots: float_index("Team Shots").unwrap_or(UNKNOWN_ID),
                opp_shots: float_index("Opponent Shots").unwrap_or(UNKNOWN_ID),
                poss_team: float_index("Team Possession %").unwrap_or(UNKNOWN_ID),
                poss_opp: float_index("Opponent Possession %").unwrap_or(UNKNOWN_ID),
                atk_team: float_index("Team Attacking %").unwrap_or(UNKNOWN_ID),
                atk_opp: float_index("Opponent Attacking %").unwrap_or(UNKNOWN_ID),
                ball_speed: float_index("Ball Speed").unwrap_or(UNKNOWN_ID),
                sim_time: float_index("Current Simulation Time").unwrap_or(UNKNOWN_ID),
                max_time: float_index("Max Simulation Time").unwrap_or(UNKNOWN_ID),
                time_left: float_index("Simulation Time Remaining").unwrap_or(UNKNOWN_ID),
                dt: float_index("Delta Time").unwrap_or(UNKNOWN_ID),
                fixed_dt: float_index("Fixed Delta Time").unwrap_or(UNKNOWN_ID),
                carrier_charge: float_index("Ball Carrier Shot Charge").unwrap_or(UNKNOWN_ID),
                carrier_charge_pct: float_index("Player With Ball Shot Charge %").unwrap_or(UNKNOWN_ID),
                carrier_stam: float_index("Ball Carrier Stamina").unwrap_or(UNKNOWN_ID),
                last_def_stam: float_index("Stamina of last defending opponent").unwrap_or(UNKNOWN_ID),
                best_intercept_slot: float_index("Best Intercept Slot").unwrap_or(UNKNOWN_ID),
            },
            top_xforms: TopXformIds {
                ball: transform_index("Ball").unwrap_or(UNKNOWN_ID),
                team_goal: transform_index("Team Goal Center").unwrap_or(UNKNOWN_ID),
                opp_goal: transform_index("Opponent Goal Center").unwrap_or(UNKNOWN_ID),
                team_left: transform_index("Team Goal Left Post").unwrap_or(UNKNOWN_ID),
                team_right: transform_index("Team Goal Right Post").unwrap_or(UNKNOWN_ID),
                opp_left: transform_index("Opponent Goal Left Post").unwrap_or(UNKNOWN_ID),
                opp_right: transform_index("Opponent Goal Right Post").unwrap_or(UNKNOWN_ID),
                opp_nearest_team_goal: transform_index("Opponent Nearest Team Goal").unwrap_or(UNKNOWN_ID),
                opp_nearest_opp_goal: transform_index("Opponent Nearest Opponent Goal").unwrap_or(UNKNOWN_ID),
                mate_nearest_team_goal: transform_index("Teammate Nearest Team Goal").unwrap_or(UNKNOWN_ID),
                mate_nearest_opp_goal: transform_index("Teammate Nearest Opponent Goal").unwrap_or(UNKNOWN_ID),
                pass_dir: [
                    vector_index("Perfect Pass Direction to Teammate 1").unwrap_or(UNKNOWN_ID),
                    vector_index("Perfect Pass Direction to Teammate 2").unwrap_or(UNKNOWN_ID),
                    vector_index("Perfect Pass Direction to Teammate 3").unwrap_or(UNKNOWN_ID),
                    vector_index("Perfect Pass Direction to Teammate 4").unwrap_or(UNKNOWN_ID),
                ],
                ball_stop_pos: vector_index("Ball Stop Position").unwrap_or(UNKNOWN_ID),
                intercept_pos: [
                    vector_index("Loose Ball Intercept Position Teammate 1").unwrap_or(UNKNOWN_ID),
                    vector_index("Loose Ball Intercept Position Teammate 2").unwrap_or(UNKNOWN_ID),
                    vector_index("Loose Ball Intercept Position Teammate 3").unwrap_or(UNKNOWN_ID),
                    vector_index("Loose Ball Intercept Position Teammate 4").unwrap_or(UNKNOWN_ID),
                ],
            },
            dist_opp: {
                let mut d = [[UNKNOWN_ID; 4]; 4];
                for pi in 0..4u8 {
                    for oi in 0..4u8 {
                        let label = format!("Distance from Team Player {} to Opponent {}", pi + 1, oi + 1);
                        d[pi as usize][oi as usize] = float_index(&label).unwrap_or(UNKNOWN_ID);
                    }
                }
                d
            },
            dist_mate: {
                let mut d = [[UNKNOWN_ID; 4]; 4];
                for pi in 0..4u8 {
                    for mi in 0..4u8 {
                        if pi == mi { continue; }
                        let label = format!("Distance from Team Player {} to Teammate {}", pi + 1, mi + 1);
                        d[pi as usize][mi as usize] = float_index(&label).unwrap_or(UNKNOWN_ID);
                    }
                }
                d
            },
        }
    })
}

pub fn build_team_api(team: TeamId, world: &WorldSensors<'_>) -> TeamApi {
    build_team_api_inner(team, world, None)
}

pub fn build_team_api_masked(
    team: TeamId,
    world: &WorldSensors<'_>,
    mask: &ApiFieldMask,
) -> TeamApi {
    build_team_api_inner(team, world, Some(mask))
}

fn build_team_api_inner(
    team: TeamId,
    world: &WorldSensors<'_>,
    mask: Option<&ApiFieldMask>,
) -> TeamApi {
    let opp = team.other();
    let is_home = team == TeamId::Home;
    let params = world.params;
    let m: &ApiFieldMask = match mask {
        Some(m) => m,
        None => all_mask(),
    };
    let pid = precomputed_ids();
    let tb = &pid.top_bools;
    let tf = &pid.top_floats;
    let tx = &pid.top_xforms;

    let mut team_players_arr: [&Player; 4] = [&world.players[0]; 4];
    let mut opp_players_arr: [&Player; 4] = [&world.players[0]; 4];
    {
        let mut ti = 0;
        let mut oi = 0;
        for p in world.players {
            if p.team == team {
                team_players_arr[ti] = p;
                ti += 1;
            } else {
                opp_players_arr[oi] = p;
                oi += 1;
            }
        }
    }
    let team_players: &[&Player] = &team_players_arr;
    let opp_players: &[&Player] = &opp_players_arr;

    let carrier = world.possession.carrier;
    let team_has_ball = matches!(carrier, Some((t, _)) if t == team);
    let opp_has_ball = matches!(carrier, Some((t, _)) if t == opp);
    let ball_loose = !world.ball.held && carrier.is_none();

    let team_goal = team_goal_center(team, params);
    let opp_goal = team_goal_center(opp, params);
    let (left_post, right_post) = team_goal_posts(team, params);

    let mut api = TeamApi::empty(team);
    api.set_bool_id(tb.is_home, is_home);
    api.set_bool_id(tb.is_away, !is_home);
    api.set_bool_id(tb.is_active, true);
    api.set_bool_id(tb.is_loose, ball_loose);
    api.set_bool_id(tb.team_has, team_has_ball);
    api.set_bool_id(tb.opp_has, opp_has_ball);
    api.set_bool_id(tb.is_kickoff, world.match_state.phase == MatchPhase::Kickoff);
    api.set_bool_id(
        tb.team_kicking,
        world.match_state.phase == MatchPhase::Kickoff && world.match_state.kickoff_team == team,
    );
    api.set_bool_id(
        tb.opp_kicking,
        world.match_state.phase == MatchPhase::Kickoff && world.match_state.kickoff_team != team,
    );
    let suppress_team_side = world.match_state.kickoff_suppress_away_team_side
        && world.match_state.kickoff_team != team;
    api.set_bool_id(
        tb.ball_team_side,
        if suppress_team_side {
            false
        } else if is_home {
            world.ball.pos.x <= 0.0
        } else {
            world.ball.pos.x >= 0.0
        },
    );
    api.set_bool_id(
        tb.ball_opp_side,
        if suppress_team_side {
            true
        } else if is_home {
            world.ball.pos.x > 0.0
        } else {
            world.ball.pos.x < 0.0
        },
    );

    let team_score = match team {
        TeamId::Home => world.match_state.score_home,
        TeamId::Away => world.match_state.score_away,
    };
    let opp_score = match team {
        TeamId::Home => world.match_state.score_away,
        TeamId::Away => world.match_state.score_home,
    };
    api.set_bool_id(tb.team_winning, team_score > opp_score);
    api.set_bool_id(tb.opp_winning, opp_score > team_score);
    api.set_bool_id(tb.team_scored, world.match_state.last_scorer == Some(team));
    api.set_bool_id(tb.opp_scored, world.match_state.last_scorer == Some(opp));
    api.set_bool_id(tb.headed_team, ball_headed_toward(world.ball, team_goal));
    api.set_bool_id(tb.headed_opp, ball_headed_toward(world.ball, opp_goal));

    // AIA: player is open iff no opposing player is within 2× interact radius.
    let open_r = params.interact_radius * 2.0;
    let bool_ids = &pid.bool_ids;

    for (pi, &id) in PlayerId::ALL.iter().enumerate() {
        let (i_has, i_near, i_closest, i_open, i_opp_open, i_opp_has, i_opp_near, i_opp_closest) =
            bool_ids[pi];
        if i_has != UNKNOWN_ID {
            api.set_bool_id(i_has, matches!(carrier, Some((t, pid)) if t == team && pid == id.0));
        }
        if i_opp_has != UNKNOWN_ID {
            api.set_bool_id(i_opp_has, matches!(carrier, Some((t, pid)) if t == opp && pid == id.0));
        }
        let need_near = m.needs_bool_id(i_near);
        let need_open = m.needs_bool_id(i_open);
        if need_near || need_open {
            if let Some(p) = team_players.iter().find(|p| p.id == id) {
                if need_near {
                    let hold = p.hold_pos(params.hold_offset);
                    let dist = (hold - world.ball.pos)
                        .length()
                        .min((p.pos - world.ball.pos).length());
                    let since_kick = if world.possession.kick_exclude_left > 0.0 {
                        (3.0 - world.possession.kick_exclude_left).clamp(0.0, 3.0)
                    } else {
                        999.0
                    };
                    let hot_near = !matches!(
                        world.possession.kick_exclude_shooter,
                        Some((t, _)) if t == team
                    ) && since_kick < 0.25;
                    let near_r = if hot_near {
                        params.interact_radius + 1.0
                    } else {
                        params.interact_radius
                    };
                    api.set_bool_id(i_near, dist <= near_r);
                }
                if need_open {
                    api.set_bool_id(i_open, is_open(p.pos, &opp_players, open_r));
                }
            } else {
                if need_near { api.set_bool_id(i_near, false); }
                if need_open { api.set_bool_id(i_open, false); }
            }
        }
        let need_opp_open = m.needs_bool_id(i_opp_open);
        let need_opp_near = m.needs_bool_id(i_opp_near);
        if need_opp_open || need_opp_near {
            if let Some(p) = opp_players.iter().find(|p| p.id == id) {
                if need_opp_open {
                    api.set_bool_id(i_opp_open, is_open(p.pos, &team_players, open_r));
                }
                if need_opp_near {
                    let hold = p.hold_pos(params.hold_offset);
                    let dist = (hold - world.ball.pos)
                        .length()
                        .min((p.pos - world.ball.pos).length());
                    let since_kick = if world.possession.kick_exclude_left > 0.0 {
                        (3.0 - world.possession.kick_exclude_left).clamp(0.0, 3.0)
                    } else {
                        999.0
                    };
                    let hot_near = !matches!(
                        world.possession.kick_exclude_shooter,
                        Some((t, _)) if t == opp
                    ) && since_kick < 0.25;
                    let near_r = if hot_near {
                        params.interact_radius + 1.0
                    } else {
                        params.interact_radius
                    };
                    api.set_bool_id(i_opp_near, dist <= near_r);
                }
            } else {
                if need_opp_open { api.set_bool_id(i_opp_open, false); }
                if need_opp_near { api.set_bool_id(i_opp_near, false); }
            }
        }
        if m.needs_bool_id(i_opp_closest) {
            api.set_bool_id(
                i_opp_closest,
                closest_teammate_to_ball(&opp_players, world.ball) == Some(id),
            );
        }
        if m.needs_bool_id(i_closest) {
            let is_closest = closest_teammate_to_ball(&team_players, world.ball) == Some(id);
            let suppress_recv = world.match_state.kickoff_suppress_away_team_side
                && world.match_state.kickoff_team != team;
            let opp_has = matches!(
                world.possession.carrier,
                Some((t, _)) if t != team
            );
            let suppress_closest = suppress_recv
                && (id.0 == 3 || (id.0 == 1 && opp_has));
            let is_closest = if suppress_closest { false } else { is_closest };
            api.set_bool_id(i_closest, is_closest);
        }
    }

    api.set_float_id(tf.interact_r, params.interact_radius);
    api.set_float_id(
        tf.team_score,
        match team {
            TeamId::Home => world.match_state.score_home as f32,
            TeamId::Away => world.match_state.score_away as f32,
        },
    );
    api.set_float_id(
        tf.opp_score,
        match team {
            TeamId::Home => world.match_state.score_away as f32,
            TeamId::Away => world.match_state.score_home as f32,
        },
    );
    api.set_float_id(tf.field_w, 50.0);
    api.set_float_id(tf.field_d, 80.0);
    api.set_float_id(tf.kickoff_r, params.kickoff_circle_r);
    api.set_float_id(tf.goal_w, params.goal_half_width * 2.0);
    api.set_float_id(tf.goal_h, 4.0);
    api.set_float_id(tf.pi, std::f32::consts::PI);
    api.set_float_id(tf.ground_line, 5.0);
    api.set_float_id(tf.area_d, 12.5);
    api.set_float_id(tf.arena_d, 2.5);

    let (team_shots, opp_shots) = match team {
        TeamId::Home => (
            world.match_state.shots_home,
            world.match_state.shots_away,
        ),
        TeamId::Away => (
            world.match_state.shots_away,
            world.match_state.shots_home,
        ),
    };
    api.set_float_id(tf.team_shots, team_shots as f32);
    api.set_float_id(tf.opp_shots, opp_shots as f32);

    let (poss_team, poss_opp) = match team {
        TeamId::Home => (
            world.match_state.possession_s_home,
            world.match_state.possession_s_away,
        ),
        TeamId::Away => (
            world.match_state.possession_s_away,
            world.match_state.possession_s_home,
        ),
    };
    let poss_tot = poss_team + poss_opp;
    api.set_float_id(
        tf.poss_team,
        if poss_tot > 1e-6 { poss_team / poss_tot } else { 0.0 },
    );
    api.set_float_id(
        tf.poss_opp,
        if poss_tot > 1e-6 { poss_opp / poss_tot } else { 0.0 },
    );

    let (atk_team, atk_opp) = match team {
        TeamId::Home => (
            world.match_state.attacking_s_home,
            world.match_state.attacking_s_away,
        ),
        TeamId::Away => (
            world.match_state.attacking_s_away,
            world.match_state.attacking_s_home,
        ),
    };
    let atk_tot = atk_team + atk_opp;
    api.set_float_id(
        tf.atk_team,
        if atk_tot > 1e-6 { atk_team / atk_tot } else { 0.0 },
    );
    api.set_float_id(
        tf.atk_opp,
        if atk_tot > 1e-6 { atk_opp / atk_tot } else { 0.0 },
    );

    api.set_float_id(tf.ball_speed, world.ball.vel.length());
    api.set_float_id(tf.sim_time, world.match_state.clock_s);
    api.set_float_id(tf.max_time, world.match_state.duration_s);
    api.set_float_id(
        tf.time_left,
        (world.match_state.duration_s - world.match_state.clock_s).max(0.0),
    );
    api.set_float_id(tf.dt, crate::world::FIXED_DT);
    api.set_float_id(tf.fixed_dt, crate::world::FIXED_DT);

    let (carrier_charge, carrier_stam) = if let Some((ct, cid)) = carrier {
        if let Some(p) = world.players.iter().find(|p| p.team == ct && p.id.0 == cid) {
            (p.shot_charge, p.stamina)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };
    api.set_float_id(tf.carrier_charge, carrier_charge);
    api.set_float_id(tf.carrier_charge_pct, carrier_charge);
    api.set_float_id(tf.carrier_stam, carrier_stam);
    if m.needs_float_id(tf.last_def_stam) {
        api.set_float_id(
            tf.last_def_stam,
            opp_players
                .iter()
                .map(|p| p.stamina)
                .fold(0.0_f32, f32::max),
        );
    }

    // Perfect pass directions + interceptable flags (only when team has ball).
    let need_any_pass = tb.can_pass.iter().any(|&id| m.needs_bool_id(id))
        || tx.pass_dir.iter().any(|&id| m.needs_vector_id(id));
    if need_any_pass && team_has_ball {
        let carrier_pos_vec = carrier.and_then(|(ct, cid)| {
            world.players.iter().find(|p| p.team == ct && p.id.0 == cid).map(|p| p.pos)
        });
        if let Some(cpos) = carrier_pos_vec {
            let opp_pos: Vec<Vec2> = opp_players.iter().map(|p| p.pos).collect();
            for mate_i in 0..4u8 {
                let mate_idx = mate_i as usize;
                let pass_dir_id = tx.pass_dir[mate_idx];
                let can_pass_id = tb.can_pass[mate_idx];
                if pass_dir_id == UNKNOWN_ID && can_pass_id == UNKNOWN_ID {
                    continue;
                }
                // Find teammate by slot (1-indexed).
                let mate = team_players.iter().find(|p| p.id.0 == mate_i + 1);
                if let Some(mate) = mate {
                    let delta = mate.pos - cpos;
                    if delta.length() < 1.0 {
                        if can_pass_id != UNKNOWN_ID {
                            api.set_bool_id(can_pass_id, false);
                        }
                        if pass_dir_id != UNKNOWN_ID {
                            api.set_vector_id(pass_dir_id, None);
                        }
                        continue;
                    }
                    let dir = delta.normalize();
                    let segments = crate::deterministic::pass_trajectory(cpos, mate.pos, params);
                    let interceptable = crate::deterministic::any_can_intercept_pure_math(
                        &segments, cpos, &opp_pos, params,
                    );
                    if can_pass_id != UNKNOWN_ID {
                        api.set_bool_id(can_pass_id, !interceptable);
                    }
                    if pass_dir_id != UNKNOWN_ID {
                        api.set_vector_id(pass_dir_id, if interceptable { None } else { Some(dir) });
                    }
                } else {
                    if can_pass_id != UNKNOWN_ID {
                        api.set_bool_id(can_pass_id, false);
                    }
                    if pass_dir_id != UNKNOWN_ID {
                        api.set_vector_id(pass_dir_id, None);
                    }
                }
            }
        } else {
            for mate_i in 0..4u8 {
                let mate_idx = mate_i as usize;
                if tb.can_pass[mate_idx] != UNKNOWN_ID {
                    api.set_bool_id(tb.can_pass[mate_idx], false);
                }
                if tx.pass_dir[mate_idx] != UNKNOWN_ID {
                    api.set_vector_id(tx.pass_dir[mate_idx], None);
                }
            }
        }
    } else if need_any_pass {
        for mate_i in 0..4u8 {
            let mate_idx = mate_i as usize;
            if tb.can_pass[mate_idx] != UNKNOWN_ID {
                api.set_bool_id(tb.can_pass[mate_idx], false);
            }
            if tx.pass_dir[mate_idx] != UNKNOWN_ID {
                api.set_vector_id(tx.pass_dir[mate_idx], None);
            }
        }
    }

    // Loose ball analysis: ball stop position, intercept positions, who's first.
    let need_any_loose = tb.opp_can_intercept.iter().any(|&id| m.needs_bool_id(id))
        || tx.intercept_pos.iter().any(|&id| m.needs_vector_id(id))
        || m.needs_vector_id(tx.ball_stop_pos)
        || m.needs_float_id(tf.best_intercept_slot);
    if need_any_loose && ball_loose && world.ball.vel.length() > 0.5 {
        let loose_path = crate::deterministic::analytic_trajectory(
            world.ball.pos,
            world.ball.vel,
            0.0,
            params,
        );
        if loose_path.scored.is_none() {
            if tx.ball_stop_pos != UNKNOWN_ID && m.needs_vector_id(tx.ball_stop_pos) {
                api.set_vector_id(tx.ball_stop_pos, Some(loose_path.end_pos));
            }
        } else if tx.ball_stop_pos != UNKNOWN_ID && m.needs_vector_id(tx.ball_stop_pos) {
            api.set_vector_id(tx.ball_stop_pos, None);
        }

        let mut best_time = f32::MAX;
        let mut best_slot = 0.0f32;
        for i in 0..4 {
            let did = tx.intercept_pos[i];
            if did == UNKNOWN_ID && tb.opp_can_intercept[i] == UNKNOWN_ID {
                continue;
            }
            let tp = team_players.iter().find(|p| p.id.0 == i as u8 + 1).map(|p| p.pos);
            if let Some(tp) = tp {
                if let Some((pos, t)) = crate::deterministic::earliest_intercept(&loose_path.segments, tp, params) {
                    if did != UNKNOWN_ID && m.needs_vector_id(did) {
                        api.set_vector_id(did, Some(pos));
                    }
                    if t < best_time {
                        best_time = t;
                        best_slot = (i + 1) as f32;
                    }
                } else if did != UNKNOWN_ID && m.needs_vector_id(did) {
                    api.set_vector_id(did, None);
                }
            } else if did != UNKNOWN_ID && m.needs_vector_id(did) {
                api.set_vector_id(did, None);
            }
        }

        if tf.best_intercept_slot != UNKNOWN_ID && m.needs_float_id(tf.best_intercept_slot) {
            api.set_float_id(tf.best_intercept_slot, best_slot);
        }

        // Opponent interception: can any opponent beat our best teammate?
        let opp_pos_arr: [Vec2; 4] = [
            opp_players.iter().find(|p| p.id.0 == 1).map(|p| p.pos).unwrap_or(Vec2::ZERO),
            opp_players.iter().find(|p| p.id.0 == 2).map(|p| p.pos).unwrap_or(Vec2::ZERO),
            opp_players.iter().find(|p| p.id.0 == 3).map(|p| p.pos).unwrap_or(Vec2::ZERO),
            opp_players.iter().find(|p| p.id.0 == 4).map(|p| p.pos).unwrap_or(Vec2::ZERO),
        ];
        for i in 0..4 {
            let bid = tb.opp_can_intercept[i];
            if bid == UNKNOWN_ID || !m.needs_bool_id(bid) {
                continue;
            }
            if let Some((_, t)) = crate::deterministic::earliest_intercept(&loose_path.segments, opp_pos_arr[i], params) {
                api.set_bool_id(bid, t < best_time);
            } else {
                api.set_bool_id(bid, false);
            }
        }
    } else if need_any_loose {
        if tx.ball_stop_pos != UNKNOWN_ID && m.needs_vector_id(tx.ball_stop_pos) {
            api.set_vector_id(tx.ball_stop_pos, None);
        }
        for i in 0..4 {
            if tx.intercept_pos[i] != UNKNOWN_ID && m.needs_vector_id(tx.intercept_pos[i]) {
                api.set_vector_id(tx.intercept_pos[i], None);
            }
            if tb.opp_can_intercept[i] != UNKNOWN_ID && m.needs_bool_id(tb.opp_can_intercept[i]) {
                api.set_bool_id(tb.opp_can_intercept[i], false);
            }
        }
        if tf.best_intercept_slot != UNKNOWN_ID && m.needs_float_id(tf.best_intercept_slot) {
            api.set_float_id(tf.best_intercept_slot, 0.0);
        }
    }

    let float_ids = &pid.float_ids;

    for (pi, &id) in PlayerId::ALL.iter().enumerate() {
        let (i_stam, i_charge, i_opp_stam, i_dist, i_near_stam) = float_ids[pi];
        let stam = team_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.stamina)
            .unwrap_or(0.0);
        let charge = team_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.shot_charge)
            .unwrap_or(0.0);
        let opp_stam = opp_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.stamina)
            .unwrap_or(0.0);
        if i_stam != UNKNOWN_ID { api.set_float_id(i_stam, stam); }
        if i_charge != UNKNOWN_ID { api.set_float_id(i_charge, charge); }
        if i_opp_stam != UNKNOWN_ID { api.set_float_id(i_opp_stam, opp_stam); }
        if m.needs_float_id(i_dist) {
            let dist = team_players
                .iter()
                .find(|p| p.id == id)
                .map(|p| nearest_dist(p.pos, &opp_players))
                .unwrap_or(0.0);
            api.set_float_id(i_dist, dist);
        }
        if m.needs_float_id(i_near_stam) {
            let near_opp_stam = team_players
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| nearest_player(p.pos, &opp_players))
                .map(|p| p.stamina)
                .unwrap_or(0.0);
            api.set_float_id(i_near_stam, near_opp_stam);
        }

        // Per-player distances to each opponent and each teammate.
        let my_pos = team_players.iter().find(|p| p.id == id).map(|p| p.pos);
        if let Some(my_pos) = my_pos {
            for oi in 0..4 {
                let did = pid.dist_opp[pi][oi];
                if did != UNKNOWN_ID && m.needs_float_id(did) {
                    let opp_pos = opp_players.iter().find(|p| p.id.0 == oi as u8 + 1).map(|p| p.pos);
                    if let Some(op) = opp_pos {
                        api.set_float_id(did, my_pos.distance(op));
                    }
                }
            }
            for mi in 0..4 {
                let did = pid.dist_mate[pi][mi];
                if did != UNKNOWN_ID && m.needs_float_id(did) {
                    let mate_pos = team_players.iter().find(|p| p.id.0 == mi as u8 + 1).map(|p| p.pos);
                    if let Some(mp) = mate_pos {
                        api.set_float_id(did, my_pos.distance(mp));
                    }
                }
            }
        }
    }

    api.set_transform_id(tx.ball, world.ball.pos);
    api.set_transform_id(tx.team_goal, team_goal);
    api.set_transform_id(tx.opp_goal, opp_goal);
    api.set_transform_id(tx.team_left, left_post);
    api.set_transform_id(tx.team_right, right_post);
    let (opp_left_post, opp_right_post) = team_goal_posts(opp, params);
    api.set_transform_id(tx.opp_left, opp_left_post);
    api.set_transform_id(tx.opp_right, opp_right_post);
    if m.needs_transform_id(tx.opp_nearest_team_goal) {
        if let Some(p) = player_nearest_to(team_goal, &opp_players) {
            api.set_transform_id(tx.opp_nearest_team_goal, p.pos);
        } else {
            api.set_transform_id(tx.opp_nearest_team_goal, opp_goal);
        }
    }
    if m.needs_transform_id(tx.opp_nearest_opp_goal) {
        if let Some(p) = player_nearest_to(opp_goal, &opp_players) {
            api.set_transform_id(tx.opp_nearest_opp_goal, p.pos);
        } else {
            api.set_transform_id(tx.opp_nearest_opp_goal, opp_goal);
        }
    }
    if m.needs_transform_id(tx.mate_nearest_team_goal) {
        if let Some(p) = player_nearest_to(team_goal, &team_players) {
            api.set_transform_id(tx.mate_nearest_team_goal, p.pos);
        } else {
            api.set_transform_id(tx.mate_nearest_team_goal, team_goal);
        }
    }
    if m.needs_transform_id(tx.mate_nearest_opp_goal) {
        if let Some(p) = player_nearest_to(opp_goal, &team_players) {
            api.set_transform_id(tx.mate_nearest_opp_goal, p.pos);
        } else {
            api.set_transform_id(tx.mate_nearest_opp_goal, opp_goal);
        }
    }
    let xform_ids = &pid.xform_ids;

    for (pi, &id) in PlayerId::ALL.iter().enumerate() {
        let (i_me, i_opp, i_near, i_near_opp) = xform_ids[pi];
        let pos = team_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.pos)
            .unwrap_or(Vec2::ZERO);
        if i_me != UNKNOWN_ID { api.set_transform_id(i_me, pos); }

        let opp_pos = opp_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.pos)
            .unwrap_or(Vec2::ZERO);
        if i_opp != UNKNOWN_ID { api.set_transform_id(i_opp, opp_pos); }

        if m.needs_transform_id(i_near) {
            let nearest = nearest_teammate_pos(id, &team_players).unwrap_or(pos);
            api.set_transform_id(i_near, nearest);
        }
        if m.needs_transform_id(i_near_opp) {
            let nearest_opp = nearest_player(pos, &opp_players)
                .map(|p| p.pos)
                .unwrap_or(opp_pos);
            api.set_transform_id(i_near_opp, nearest_opp);
        }
    }

    if m.needs_any_vector() {
    api.set_vector("Ball Velocity", Some(world.ball.vel));
    api.set_vector("Center Field", Some(Vec2::ZERO));

    let (upper_home, lower_home) = (
        Vec2::new(-params.x_max, params.z_max),
        Vec2::new(-params.x_max, -params.z_max),
    );
    let (upper_away, lower_away) = (
        Vec2::new(params.x_max, params.z_max),
        Vec2::new(params.x_max, -params.z_max),
    );
    api.set_vector("Upper Corner Home Side", Some(upper_home));
    api.set_vector("Lower Corner Home Side", Some(lower_home));
    api.set_vector("Upper Corner Away Side", Some(upper_away));
    api.set_vector("Lower Corner Away Side", Some(lower_away));
    api.set_vector(
        "Upper Corner Team Side",
        Some(if is_home { upper_home } else { upper_away }),
    );
    api.set_vector(
        "Lower Corner Team Side",
        Some(if is_home { lower_home } else { lower_away }),
    );
    api.set_vector(
        "Upper Corner Opposing Side",
        Some(if is_home { upper_away } else { upper_home }),
    );
    api.set_vector(
        "Lower Corner Opposing Side",
        Some(if is_home { lower_away } else { lower_home }),
    );
    api.set_vector("Upper Midfield", Some(Vec2::new(0.0, params.z_max * 0.5)));
    api.set_vector("Lower Midfield", Some(Vec2::new(0.0, -params.z_max * 0.5)));

    // AIA Spherecast Floats: radius=0.25, distance=20 (graph + Debug rays).
    // Effective hit radius measured from real-game sensor data: 0.769m
    // consistently across all 8 directions (player collider + spherecast).
    let blocker_r = crate::api::SPHERECAST_HIT_RADIUS;
    let range = crate::api::SPHERECAST_DISTANCE;
    let all_others = |except: Option<(TeamId, u8)>| -> [Vec2; 7] {
        let mut result = [Vec2::ZERO; 7];
        let mut idx = 0;
        for p in world.players {
            if except == Some((p.team, p.id.0)) {
                continue;
            }
            if idx < 7 {
                result[idx] = p.pos;
                idx += 1;
            }
        }
        result
    };

    for id in PlayerId::ALL {
        let me = team_players.iter().find(|p| p.id == id);
        let from_ball = match id.0 {
            1 => "Direction of ball from Teammate 1",
            2 => "Direction of ball from Teammate 2",
            3 => "Direction of ball from Teammate 3",
            _ => "Direction of ball from Teammate 4",
        };
        let from_opp_goal = match id.0 {
            1 => "Direction of opponent goal from Teammate 1",
            2 => "Direction of opponent goal from Teammate 2",
            3 => "Direction of opponent goal from Teammate 3",
            _ => "Direction of opponent goal from Teammate 4",
        };
        let from_team_goal = match id.0 {
            1 => "Direction of team goal from Teammate 1",
            2 => "Direction of team goal from Teammate 2",
            3 => "Direction of team goal from Teammate 3",
            _ => "Direction of team goal from Teammate 4",
        };
        let from_teammate = match id.0 {
            1 => "Direction of teammate from Team Player 1",
            2 => "Direction of teammate from Team Player 2",
            3 => "Direction of teammate from Team Player 3",
            _ => "Direction of teammate from Team Player 4",
        };

        if let Some(p) = me {
            let blockers = all_others(Some((team, id.0)));
            api.set_vector(from_ball, Some(dir(p.pos, world.ball.pos)));
            // Null unless an 8-way lane into the goal mouth is clear.
            // Goal-dir uses interact-scale probe (not slim clear-dir body*1.1):
            // midfield E often misses opponents by <1m with slim r → OppGoal
            // Present → Ready at charge≈0.5 → early dump. Real OppGoal null
            // ~90% until near box (AIA: null if no clear sensor dir of goal).
            let goal_blocker_r = crate::api::SPHERECAST_HIT_RADIUS;
            api.set_vector(
                from_opp_goal,
                clear_dir_into_goal_mouth(
                    p.pos,
                    !is_home, // toward Away goal when we are Home
                    &blockers,
                    goal_blocker_r,
                    params.goal_line_x,
                    params.goal_half_width,
                ),
            );
            api.set_vector(
                from_team_goal,
                clear_dir_into_goal_mouth(
                    p.pos,
                    is_home, // toward own (Home) goal when we are Home
                    &blockers,
                    goal_blocker_r,
                    params.goal_line_x,
                    params.goal_half_width,
                ),
            );
            api.set_vector(
                from_teammate,
                nearest_teammate_pos(id, &team_players).map(|t| dir(p.pos, t)),
            );
        } else {
            api.set_vector(from_ball, None);
            api.set_vector(from_opp_goal, None);
            api.set_vector(from_team_goal, None);
            api.set_vector(from_teammate, None);
        }

        let from_ball_opp = match id.0 {
            1 => "Direction of ball from Opponent 1",
            2 => "Direction of ball from Opponent 2",
            3 => "Direction of ball from Opponent 3",
            _ => "Direction of ball from Opponent 4",
        };
        if let Some(p) = opp_players.iter().find(|p| p.id == id) {
            api.set_vector(from_ball_opp, Some(dir(p.pos, world.ball.pos)));
        } else {
            api.set_vector(from_ball_opp, None);
        }
    }

    // Clear directions
    for id in PlayerId::ALL {
        let label = match id.0 {
            1 => "Clear direction from Teammate 1",
            2 => "Clear direction from Teammate 2",
            3 => "Clear direction from Teammate 3",
            _ => "Clear direction from Teammate 4",
        };
        let clear = team_players.iter().find(|p| p.id == id).and_then(|p| {
            first_clear_dir(
                p.pos,
                is_home,
                &all_others(Some((team, id.0))),
                blocker_r,
                range,
                true,
                true,
                params.x_min,
                params.x_max,
                params.z_min,
                params.z_max,
            )
        });
        api.set_vector(label, clear);
    }

    let carrier_pos = carrier.and_then(|(ct, cid)| {
        world
            .players
            .iter()
            .find(|p| p.team == ct && p.id.0 == cid)
            .map(|p| p.pos)
    });

    let clear_from = |avoid_side: bool, avoid_goal: bool| -> Option<Vec2> {
        let origin = carrier_pos.filter(|_| team_has_ball)?;
        first_clear_dir(
            origin,
            is_home,
            &all_others(carrier),
            blocker_r,
            range,
            avoid_side,
            avoid_goal,
            params.x_min,
            params.x_max,
            params.z_min,
            params.z_max,
        )
    };

    let clear_main = clear_from(true, true);
    api.set_vector("Clear direction from team carrier", clear_main);
    api.set_vector(
        "Clear direction from team carrier (avoid all walls)",
        clear_from(true, true),
    );
    api.set_vector(
        "Clear direction from team carrier (avoid goal lines)",
        clear_from(false, true),
    );
    api.set_vector(
        "Clear direction from team carrier (avoid sidelines)",
        clear_from(true, false),
    );
    api.set_vector(
        "Backwards clear direction from team carrier",
        clear_main.map(|d| -d),
    );

    // Open helpers: only players with no opposing body within 2× interact radius.
    let open_team_near = open_player_pos(
        &team_players,
        &opp_players,
        open_r,
        world.ball.pos,
        OpenPick::NearestBall,
    );
    let open_team_far = open_player_pos(
        &team_players,
        &opp_players,
        open_r,
        world.ball.pos,
        OpenPick::FurthestBall,
    );
    let open_team_most = open_player_pos(
        &team_players,
        &opp_players,
        open_r,
        world.ball.pos,
        OpenPick::MostClear,
    );
    let open_opp_near = open_player_pos(
        &opp_players,
        &team_players,
        open_r,
        world.ball.pos,
        OpenPick::NearestBall,
    );
    let open_opp_far = open_player_pos(
        &opp_players,
        &team_players,
        open_r,
        world.ball.pos,
        OpenPick::FurthestBall,
    );
    let open_opp_most = open_player_pos(
        &opp_players,
        &team_players,
        open_r,
        world.ball.pos,
        OpenPick::MostClear,
    );
    api.set_vector("Get nearest open teammate", open_team_near);
    api.set_vector("Get most open teammate", open_team_most);
    api.set_vector("Get furthest open teammate", open_team_far);
    api.set_vector("Get nearest open opponent", open_opp_near);
    api.set_vector("Get most open opponent", open_opp_most);
    api.set_vector("Get furthest open opponent", open_opp_far);

    for id in PlayerId::ALL {
        let t_label = match id.0 {
            1 => "Direction of clear teammate from Teammate 1",
            2 => "Direction of clear teammate from Teammate 2",
            3 => "Direction of clear teammate from Teammate 3",
            _ => "Direction of clear teammate from Teammate 4",
        };
        let o_label = match id.0 {
            1 => "Direction of clear teammate from Opponent 1",
            2 => "Direction of clear teammate from Opponent 2",
            3 => "Direction of clear teammate from Opponent 3",
            _ => "Direction of clear teammate from Opponent 4",
        };
        let from_t = team_players.iter().find(|p| p.id == id).map(|p| p.pos);
        let mut mate_positions = [Vec2::ZERO; 3];
        let mut mi = 0;
        for p in team_players {
            if p.id != id && mi < 3 {
                mate_positions[mi] = p.pos;
                mi += 1;
            }
        }
        let opp_blockers: [Vec2; 4] = [
            opp_players[0].pos,
            opp_players[1].pos,
            opp_players[2].pos,
            opp_players[3].pos,
        ];
        let mate_blocker_r = crate::api::SPHERECAST_HIT_RADIUS;
        api.set_vector(
            t_label,
            from_t.and_then(|o| {
                clear_dir_toward_teammate(o, &mate_positions, &opp_blockers, mate_blocker_r)
            }),
        );
        let from_o = opp_players.iter().find(|p| p.id == id).map(|p| p.pos);
        let mut their_mates = [Vec2::ZERO; 3];
        let mut ti = 0;
        for p in opp_players {
            if p.id != id && ti < 3 {
                their_mates[ti] = p.pos;
                ti += 1;
            }
        }
        let our_blockers: [Vec2; 4] = [
            team_players[0].pos,
            team_players[1].pos,
            team_players[2].pos,
            team_players[3].pos,
        ];
        api.set_vector(
            o_label,
            from_o.and_then(|o| {
                clear_dir_toward_teammate(o, &their_mates, &our_blockers, mate_blocker_r)
            }),
        );
    }

    }

    api
}

fn team_goal_center(team: TeamId, params: &SimParams) -> Vec2 {
    match team {
        TeamId::Home => Vec2::new(-params.goal_line_x, 0.0),
        TeamId::Away => Vec2::new(params.goal_line_x, 0.0),
    }
}

fn team_goal_posts(team: TeamId, params: &SimParams) -> (Vec2, Vec2) {
    let x = match team {
        TeamId::Home => -params.posts_x,
        TeamId::Away => params.posts_x,
    };
    (
        Vec2::new(x, params.goal_half_width),
        Vec2::new(x, -params.goal_half_width),
    )
}

fn dir(from: Vec2, to: Vec2) -> Vec2 {
    (to - from).normalize_or_zero()
}

fn nearest_dist(from: Vec2, others: &[&Player]) -> f32 {
    others
        .iter()
        .map(|p| (p.pos - from).length())
        .fold(f32::MAX, f32::min)
}

fn nearest_player<'a>(from: Vec2, others: &[&'a Player]) -> Option<&'a Player> {
    others.iter().copied().min_by(|a, b| {
        (a.pos - from)
            .length()
            .partial_cmp(&(b.pos - from).length())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn player_nearest_to<'a>(target: Vec2, players: &[&'a Player]) -> Option<&'a Player> {
    nearest_player(target, players)
}

fn ball_headed_toward(ball: &Ball, goal: Vec2) -> bool {
    let speed = ball.vel.length();
    if speed < 0.5 {
        return false;
    }
    let to_goal = goal - ball.pos;
    if to_goal.length_squared() < 1e-4 {
        return false;
    }
    ball.vel.normalize().dot(to_goal.normalize()) > 0.5
}

/// AIA lock: open = no opposing player within `open_r` (= 2× interact radius).
fn is_open(pos: Vec2, opposing: &[&Player], open_r: f32) -> bool {
    nearest_dist(pos, opposing) > open_r
}

fn closest_teammate_to_ball(team: &[&Player], ball: &Ball) -> Option<PlayerId> {
    team.iter()
        .min_by(|a, b| {
            (a.pos - ball.pos)
                .length()
                .partial_cmp(&(b.pos - ball.pos).length())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.id)
}

fn nearest_teammate_pos(id: PlayerId, team: &[&Player]) -> Option<Vec2> {
    let me = team.iter().find(|p| p.id == id)?;
    team.iter()
        .filter(|p| p.id != id)
        .min_by(|a, b| {
            (a.pos - me.pos)
                .length()
                .partial_cmp(&(b.pos - me.pos).length())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.pos)
}

#[derive(Clone, Copy)]
enum OpenPick {
    NearestBall,
    FurthestBall,
    /// Largest distance to nearest opposing player (among open players).
    MostClear,
}

fn open_player_pos(
    players: &[&Player],
    opposing: &[&Player],
    open_r: f32,
    ball: Vec2,
    pick: OpenPick,
) -> Option<Vec2> {
    let open: Vec<&Player> = players
        .iter()
        .copied()
        .filter(|p| is_open(p.pos, opposing, open_r))
        .collect();
    if open.is_empty() {
        return None;
    }
    let chosen = match pick {
        OpenPick::NearestBall => open.iter().min_by(|a, b| {
            (a.pos - ball)
                .length()
                .partial_cmp(&(b.pos - ball).length())
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        OpenPick::FurthestBall => open.iter().max_by(|a, b| {
            (a.pos - ball)
                .length()
                .partial_cmp(&(b.pos - ball).length())
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        OpenPick::MostClear => open.iter().max_by(|a, b| {
            nearest_dist(a.pos, opposing)
                .partial_cmp(&nearest_dist(b.pos, opposing))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    };
    chosen.map(|p| p.pos)
}
