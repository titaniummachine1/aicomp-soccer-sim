//! Build team-relative API snapshots from the 2D world each tick.

use bevy::prelude::*;
use std::collections::HashMap;

use super::clear::{clear_dir_into_goal_mouth, clear_dir_toward_teammate, first_clear_dir};
use super::TeamApi;
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

pub fn build_team_api(team: TeamId, world: &WorldSensors<'_>) -> TeamApi {
    let opp = team.other();
    let is_home = team == TeamId::Home;
    let params = world.params;

    let team_players: Vec<&Player> = world.players.iter().filter(|p| p.team == team).collect();
    let opp_players: Vec<&Player> = world.players.iter().filter(|p| p.team == opp).collect();

    let carrier = world.possession.carrier;
    let team_has_ball = matches!(carrier, Some((t, _)) if t == team);
    let opp_has_ball = matches!(carrier, Some((t, _)) if t == opp);
    let ball_loose = !world.ball.held && carrier.is_none();

    let team_goal = team_goal_center(team, params);
    let opp_goal = team_goal_center(opp, params);
    let (left_post, right_post) = team_goal_posts(team, params);

    let mut bools = HashMap::new();
    bools.insert("Is Home Team", is_home);
    bools.insert("Is Away Team", !is_home);
    bools.insert("Is Active Graph", true);
    bools.insert("Is Ball Loose", ball_loose);
    bools.insert("Team Has Ball", team_has_ball);
    bools.insert("Opponent Has Ball", opp_has_ball);
    bools.insert("Is Kickoff", world.match_state.phase == MatchPhase::Kickoff);
    bools.insert(
        "Is Team Kicking off",
        world.match_state.phase == MatchPhase::Kickoff && world.match_state.kickoff_team == team,
    );
    bools.insert(
        "Is Opponent Kicking off",
        world.match_state.phase == MatchPhase::Kickoff && world.match_state.kickoff_team != team,
    );
    // While the receiving team is circle-locked after kickoff, treat Away's
    // "Ball On Team Side" as false so Defender stays on State0 hold (x≈6+BallX)
    // instead of chasing Ball onto the ring in the carrier's C-lane (that kept
    // Clear=H through release and launched deep −Z kicks).
    bools.insert(
        "Ball On Team Side",
        if is_home {
            world.ball.pos.x <= 0.0
        } else if world.match_state.kickoff_suppress_away_team_side {
            false
        } else {
            world.ball.pos.x >= 0.0
        },
    );
    bools.insert(
        "Ball On Opponent Side",
        if is_home {
            world.ball.pos.x > 0.0
        } else if world.match_state.kickoff_suppress_away_team_side {
            // Mirror of suppressed team-side: treat as on opponent half.
            true
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
    bools.insert("Team Is Winning", team_score > opp_score);
    bools.insert("Opponent Is Winning", opp_score > team_score);
    bools.insert(
        "Team Scored Last Point",
        world.match_state.last_scorer == Some(team),
    );
    bools.insert(
        "Opponent Scored Last Point",
        world.match_state.last_scorer == Some(opp),
    );
    bools.insert(
        "Is Ball Headed Towards Team Goal",
        ball_headed_toward(world.ball, team_goal),
    );
    bools.insert(
        "Is Ball Headed Towards Opponent Goal",
        ball_headed_toward(world.ball, opp_goal),
    );

    // AIA: player is open iff no opposing player is within 2× interact radius.
    let open_r = params.interact_radius * 2.0;

    for id in PlayerId::ALL {
        let label_has = match id.0 {
            1 => "Team Player 1 Has Ball",
            2 => "Team Player 2 Has Ball",
            3 => "Team Player 3 Has Ball",
            _ => "Team Player 4 Has Ball",
        };
        let label_near = match id.0 {
            1 => "Is Ball Nearby Team Player 1",
            2 => "Is Ball Nearby Team Player 2",
            3 => "Is Ball Nearby Team Player 3",
            _ => "Is Ball Nearby Team Player 4",
        };
        let label_closest = match id.0 {
            1 => "Is Team Player 1 Closest Teammate to Ball",
            2 => "Is Team Player 2 Closest Teammate to Ball",
            3 => "Is Team Player 3 Closest Teammate to Ball",
            _ => "Is Team Player 4 Closest Teammate to Ball",
        };
        let label_open = match id.0 {
            1 => "Is Team Player 1 Open",
            2 => "Is Team Player 2 Open",
            3 => "Is Team Player 3 Open",
            _ => "Is Team Player 4 Open",
        };
        let label_opp_open = match id.0 {
            1 => "Is Opponent Player 1 Open",
            2 => "Is Opponent Player 2 Open",
            3 => "Is Opponent Player 3 Open",
            _ => "Is Opponent Player 4 Open",
        };
        let label_opp_has = match id.0 {
            1 => "Opponent Player 1 Has Ball",
            2 => "Opponent Player 2 Has Ball",
            3 => "Opponent Player 3 Has Ball",
            _ => "Opponent Player 4 Has Ball",
        };
        let label_opp_near = match id.0 {
            1 => "Is Ball Nearby Opponent Player 1",
            2 => "Is Ball Nearby Opponent Player 2",
            3 => "Is Ball Nearby Opponent Player 3",
            _ => "Is Ball Nearby Opponent Player 4",
        };
        let label_opp_closest = match id.0 {
            1 => "Is Opponent Player 1 Closest Opponent to Ball",
            2 => "Is Opponent Player 2 Closest Opponent to Ball",
            3 => "Is Opponent Player 3 Closest Opponent to Ball",
            _ => "Is Opponent Player 4 Closest Opponent to Ball",
        };
        bools.insert(
            label_has,
            matches!(carrier, Some((t, pid)) if t == team && pid == id.0),
        );
        bools.insert(
            label_opp_has,
            matches!(carrier, Some((t, pid)) if t == opp && pid == id.0),
        );
        if let Some(p) = team_players.iter().find(|p| p.id == id) {
            let hold = p.hold_pos(params.hold_offset);
            let dist = (hold - world.ball.pos)
                .length()
                .min((p.pos - world.ball.pos).length());
            // Fat nearby during post-kick opponent reclaim window so Interact
            // stays true out to the same reach as possession hot_opp_window.
            let since_kick = if world.possession.kick_exclude_left > 0.0 {
                (2.5 - world.possession.kick_exclude_left).clamp(0.0, 2.5)
            } else {
                999.0
            };
            let hot_near = !matches!(world.possession.kick_exclude_team, Some(t) if t == team)
                && since_kick < 0.25;
            let near_r = if hot_near {
                params.interact_radius + 1.0
            } else {
                params.interact_radius
            };
            bools.insert(label_near, dist <= near_r);
            bools.insert(label_open, is_open(p.pos, &opp_players, open_r));
        } else {
            bools.insert(label_near, false);
            bools.insert(label_open, false);
        }
        if let Some(p) = opp_players.iter().find(|p| p.id == id) {
            bools.insert(label_opp_open, is_open(p.pos, &team_players, open_r));
            let hold = p.hold_pos(params.hold_offset);
            let dist = (hold - world.ball.pos)
                .length()
                .min((p.pos - world.ball.pos).length());
            let since_kick = if world.possession.kick_exclude_left > 0.0 {
                (2.5 - world.possession.kick_exclude_left).clamp(0.0, 2.5)
            } else {
                999.0
            };
            let hot_near = !matches!(world.possession.kick_exclude_team, Some(t) if t == opp)
                && since_kick < 0.25;
            let near_r = if hot_near {
                params.interact_radius + 1.0
            } else {
                params.interact_radius
            };
            bools.insert(label_opp_near, dist <= near_r);
        } else {
            bools.insert(label_opp_open, false);
            bools.insert(label_opp_near, false);
        }
        bools.insert(
            label_opp_closest,
            closest_teammate_to_ball(&opp_players, world.ball) == Some(id),
        );
        let is_closest = closest_teammate_to_ball(&team_players, world.ball) == Some(id);
        // During opening suppress, Away Defender chase also keys off Closest_P3
        // (OR Ball On Team Side). Forcing P3 not-closest keeps State0 hold so O3
        // doesn't walk onto the carrier C-lane (X dropping toward the ball).
        let is_closest =
            if !is_home && world.match_state.kickoff_suppress_away_team_side && id.0 == 3 {
                false
            } else {
                is_closest
            };
        bools.insert(label_closest, is_closest);
    }

    let mut floats = HashMap::new();
    floats.insert("Player Interact Radius", params.interact_radius);
    floats.insert(
        "Team Score",
        match team {
            TeamId::Home => world.match_state.score_home as f32,
            TeamId::Away => world.match_state.score_away as f32,
        },
    );
    floats.insert(
        "Opponent Score",
        match team {
            TeamId::Home => world.match_state.score_away as f32,
            TeamId::Away => world.match_state.score_home as f32,
        },
    );
    floats.insert("Field Width", 50.0);
    floats.insert("Field Depth", 80.0);
    floats.insert("Kickoff Circle Radius", params.kickoff_circle_r);
    floats.insert("Goal Width", params.goal_half_width * 2.0);
    // Visual/API height; pitch physics is 2D XZ. FIFA-ish default until TimePlot.
    floats.insert("Goal Height", 2.44);
    floats.insert("Pi", std::f32::consts::PI);
    floats.insert("ground_line", 5.0);
    floats.insert("Area Depth", 12.5);
    floats.insert("Arena Semicircle Depth", 2.5);

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
    floats.insert("Team Shots", team_shots as f32);
    floats.insert("Opponent Shots", opp_shots as f32);

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
    floats.insert(
        "Team Possession %",
        if poss_tot > 1e-6 {
            100.0 * poss_team / poss_tot
        } else {
            50.0
        },
    );
    floats.insert(
        "Opponent Possession %",
        if poss_tot > 1e-6 {
            100.0 * poss_opp / poss_tot
        } else {
            50.0
        },
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
    floats.insert(
        "Team Attacking %",
        if atk_tot > 1e-6 {
            100.0 * atk_team / atk_tot
        } else {
            50.0
        },
    );
    floats.insert(
        "Opponent Attacking %",
        if atk_tot > 1e-6 {
            100.0 * atk_opp / atk_tot
        } else {
            50.0
        },
    );

    floats.insert("Ball Speed", world.ball.vel.length());
    floats.insert(
        "Current Simulation Time",
        world.match_state.clock_s,
    );
    floats.insert("Max Simulation Time", world.match_state.duration_s);
    floats.insert(
        "Simulation Time Remaining",
        (world.match_state.duration_s - world.match_state.clock_s).max(0.0),
    );
    floats.insert("Delta Time", crate::world::FIXED_DT);
    floats.insert("Fixed Delta Time", crate::world::FIXED_DT);

    let (carrier_charge, carrier_stam) = if let Some((ct, cid)) = carrier {
        if let Some(p) = world.players.iter().find(|p| p.team == ct && p.id.0 == cid) {
            (p.shot_charge, p.stamina)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };
    floats.insert("Ball Carrier Shot Charge", carrier_charge);
    // Unity label says "%"; value is still 0..1 (same as Ball Carrier Shot Charge).
    floats.insert("Player With Ball Shot Charge %", carrier_charge);
    floats.insert("Ball Carrier Stamina", carrier_stam);
    floats.insert(
        "Stamina of last defending opponent",
        opp_players
            .iter()
            .map(|p| p.stamina)
            .fold(0.0_f32, f32::max),
    );

    for id in PlayerId::ALL {
        let stam_label = match id.0 {
            1 => "Team Player 1 Stamina",
            2 => "Team Player 2 Stamina",
            3 => "Team Player 3 Stamina",
            _ => "Team Player 4 Stamina",
        };
        let charge_label = match id.0 {
            1 => "Teammate 1 Shot Charge",
            2 => "Teammate 2 Shot Charge",
            3 => "Teammate 3 Shot Charge",
            _ => "Teammate 4 Shot Charge",
        };
        let opp_stam_label = match id.0 {
            1 => "Opponent Player 1 Stamina",
            2 => "Opponent Player 2 Stamina",
            3 => "Opponent Player 3 Stamina",
            _ => "Opponent Player 4 Stamina",
        };
        let dist_label = match id.0 {
            1 => "Distance from Team Player 1 to nearest Opponent",
            2 => "Distance from Team Player 2 to nearest Opponent",
            3 => "Distance from Team Player 3 to nearest Opponent",
            _ => "Distance from Team Player 4 to nearest Opponent",
        };
        let near_opp_stam_label = match id.0 {
            1 => "Opponent Nearest Teammate Player 1 Stamina",
            2 => "Opponent Nearest Teammate Player 2 Stamina",
            3 => "Opponent Nearest Teammate Player 3 Stamina",
            _ => "Opponent Nearest Teammate Player 4 Stamina",
        };
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
        floats.insert(stam_label, stam);
        floats.insert(charge_label, charge);
        floats.insert(opp_stam_label, opp_stam);
        let dist = team_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| nearest_dist(p.pos, &opp_players))
            .unwrap_or(0.0);
        floats.insert(dist_label, dist);
        let near_opp_stam = team_players
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| nearest_player(p.pos, &opp_players))
            .map(|p| p.stamina)
            .unwrap_or(0.0);
        floats.insert(near_opp_stam_label, near_opp_stam);
    }

    let mut transforms = HashMap::new();
    transforms.insert("Ball", world.ball.pos);
    transforms.insert("Team Goal Center", team_goal);
    transforms.insert("Opponent Goal Center", opp_goal);
    transforms.insert("Team Goal Left Post", left_post);
    transforms.insert("Team Goal Right Post", right_post);
    let (opp_left_post, opp_right_post) = team_goal_posts(opp, params);
    transforms.insert("Opponent Goal Left Post", opp_left_post);
    transforms.insert("Opponent Goal Right Post", opp_right_post);
    if let Some(p) = player_nearest_to(team_goal, &opp_players) {
        transforms.insert("Opponent Nearest Team Goal", p.pos);
    } else {
        transforms.insert("Opponent Nearest Team Goal", opp_goal);
    }
    if let Some(p) = player_nearest_to(opp_goal, &opp_players) {
        transforms.insert("Opponent Nearest Opponent Goal", p.pos);
    } else {
        transforms.insert("Opponent Nearest Opponent Goal", opp_goal);
    }
    if let Some(p) = player_nearest_to(team_goal, &team_players) {
        transforms.insert("Teammate Nearest Team Goal", p.pos);
    } else {
        transforms.insert("Teammate Nearest Team Goal", team_goal);
    }
    if let Some(p) = player_nearest_to(opp_goal, &team_players) {
        transforms.insert("Teammate Nearest Opponent Goal", p.pos);
    } else {
        transforms.insert("Teammate Nearest Opponent Goal", opp_goal);
    }
    for id in PlayerId::ALL {
        let key = match id.0 {
            1 => "Team Player 1",
            2 => "Team Player 2",
            3 => "Team Player 3",
            _ => "Team Player 4",
        };
        let pos = team_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.pos)
            .unwrap_or(Vec2::ZERO);
        transforms.insert(key, pos);

        let opp_key = match id.0 {
            1 => "Opponent Player 1",
            2 => "Opponent Player 2",
            3 => "Opponent Player 3",
            _ => "Opponent Player 4",
        };
        let opp_pos = opp_players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.pos)
            .unwrap_or(Vec2::ZERO);
        transforms.insert(opp_key, opp_pos);

        let near_key = match id.0 {
            1 => "Teammate Nearest Team Player 1",
            2 => "Teammate Nearest Team Player 2",
            3 => "Teammate Nearest Team Player 3",
            _ => "Teammate Nearest Team Player 4",
        };
        let nearest = nearest_teammate_pos(id, &team_players).unwrap_or(pos);
        transforms.insert(near_key, nearest);

        let near_opp_key = match id.0 {
            1 => "Opponent Nearest Team Player 1",
            2 => "Opponent Nearest Team Player 2",
            3 => "Opponent Nearest Team Player 3",
            _ => "Opponent Nearest Team Player 4",
        };
        let nearest_opp = nearest_player(pos, &opp_players)
            .map(|p| p.pos)
            .unwrap_or(opp_pos);
        transforms.insert(near_opp_key, nearest_opp);
    }

    let mut vectors: HashMap<&'static str, Option<Vec2>> = HashMap::new();
    vectors.insert("Ball Velocity", Some(world.ball.vel));
    vectors.insert("Center Field", Some(Vec2::ZERO));

    let (upper_home, lower_home) = (
        Vec2::new(-params.x_max, params.z_max),
        Vec2::new(-params.x_max, -params.z_max),
    );
    let (upper_away, lower_away) = (
        Vec2::new(params.x_max, params.z_max),
        Vec2::new(params.x_max, -params.z_max),
    );
    vectors.insert("Upper Corner Home Side", Some(upper_home));
    vectors.insert("Lower Corner Home Side", Some(lower_home));
    vectors.insert("Upper Corner Away Side", Some(upper_away));
    vectors.insert("Lower Corner Away Side", Some(lower_away));
    vectors.insert(
        "Upper Corner Team Side",
        Some(if is_home { upper_home } else { upper_away }),
    );
    vectors.insert(
        "Lower Corner Team Side",
        Some(if is_home { lower_home } else { lower_away }),
    );
    vectors.insert(
        "Upper Corner Opposing Side",
        Some(if is_home { upper_away } else { upper_home }),
    );
    vectors.insert(
        "Lower Corner Opposing Side",
        Some(if is_home { lower_away } else { lower_home }),
    );
    vectors.insert("Upper Midfield", Some(Vec2::new(0.0, params.z_max * 0.5)));
    vectors.insert("Lower Midfield", Some(Vec2::new(0.0, -params.z_max * 0.5)));

    // ~0.75 m (body*1.5): real Clear stays C until ~t=0.25; *1.75 flipped ~0.20.
    let blocker_r = params.body_radius * 1.5;
    let range = 12.0;
    let all_others = |except: Option<(TeamId, u8)>| -> Vec<Vec2> {
        world
            .players
            .iter()
            .filter(|p| except != Some((p.team, p.id.0)))
            .map(|p| p.pos)
            .collect()
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
            vectors.insert(from_ball, Some(dir(p.pos, world.ball.pos)));
            // Null unless an 8-way lane into the goal mouth is clear.
            // Goal-dir uses interact-scale probe (not slim clear-dir body*1.1):
            // midfield E often misses opponents by <1m with slim r → OppGoal
            // Present → Ready at charge≈0.5 → early dump. Real OppGoal null
            // ~90% until near box (AIA: null if no clear sensor dir of goal).
            let goal_blocker_r = params.interact_radius;
            vectors.insert(
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
            vectors.insert(
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
            vectors.insert(
                from_teammate,
                nearest_teammate_pos(id, &team_players).map(|t| dir(p.pos, t)),
            );
        } else {
            vectors.insert(from_ball, None);
            vectors.insert(from_opp_goal, None);
            vectors.insert(from_team_goal, None);
            vectors.insert(from_teammate, None);
        }

        let from_ball_opp = match id.0 {
            1 => "Direction of ball from Opponent 1",
            2 => "Direction of ball from Opponent 2",
            3 => "Direction of ball from Opponent 3",
            _ => "Direction of ball from Opponent 4",
        };
        if let Some(p) = opp_players.iter().find(|p| p.id == id) {
            vectors.insert(from_ball_opp, Some(dir(p.pos, world.ball.pos)));
        } else {
            vectors.insert(from_ball_opp, None);
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
        vectors.insert(label, clear);
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
    vectors.insert("Clear direction from team carrier", clear_main);
    vectors.insert(
        "Clear direction from team carrier (avoid all walls)",
        clear_from(true, true),
    );
    vectors.insert(
        "Clear direction from team carrier (avoid goal lines)",
        clear_from(false, true),
    );
    vectors.insert(
        "Clear direction from team carrier (avoid sidelines)",
        clear_from(true, false),
    );
    vectors.insert(
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
    vectors.insert("Get nearest open teammate", open_team_near);
    vectors.insert("Get most open teammate", open_team_most);
    vectors.insert("Get furthest open teammate", open_team_far);
    vectors.insert("Get nearest open opponent", open_opp_near);
    vectors.insert("Get most open opponent", open_opp_most);
    vectors.insert("Get furthest open opponent", open_opp_far);

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
        let mate_positions: Vec<Vec2> = team_players
            .iter()
            .filter(|p| p.id != id)
            .map(|p| p.pos)
            .collect();
        // Opponents only here; clear_dir_toward_teammate also treats *other*
        // mates as blockers (not the target mate).
        let opp_blockers: Vec<Vec2> = opp_players.iter().map(|p| p.pos).collect();
        let mate_blocker_r = params.body_radius * 2.8;
        vectors.insert(
            t_label,
            from_t.and_then(|o| {
                clear_dir_toward_teammate(o, &mate_positions, &opp_blockers, mate_blocker_r)
            }),
        );
        let from_o = opp_players.iter().find(|p| p.id == id).map(|p| p.pos);
        let their_mates: Vec<Vec2> = opp_players
            .iter()
            .filter(|p| p.id != id)
            .map(|p| p.pos)
            .collect();
        let our_blockers: Vec<Vec2> = team_players.iter().map(|p| p.pos).collect();
        vectors.insert(
            o_label,
            from_o.and_then(|o| {
                clear_dir_toward_teammate(o, &their_mates, &our_blockers, mate_blocker_r)
            }),
        );
    }

    TeamApi {
        team,
        bools,
        floats,
        transforms,
        vectors,
    }
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
