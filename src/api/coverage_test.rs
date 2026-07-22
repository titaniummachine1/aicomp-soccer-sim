//! Coverage test: every dump label is present on a TeamApi snapshot.

#[cfg(test)]
mod tests {
    use crate::api::{
        build_team_api, WorldSensors, GET_BOOL, GET_FLOAT, GET_TRANSFORM, GET_VECTOR3,
    };
    use crate::ball::Ball;
    use crate::brain::TeamId;
    use crate::match_state::MatchState;
    use crate::params::SimParams;
    use crate::player::{default_facing, faceoff_world, Player, PlayerId};
    use crate::possession::Possession;
    use bevy::prelude::Vec2;

    fn sample_players() -> Vec<Player> {
        let mut out = Vec::new();
        for team in [TeamId::Home, TeamId::Away] {
            for id in PlayerId::ALL {
                out.push(Player {
                    team,
                    id,
                    pos: faceoff_world(team, id, TeamId::Home),
                    vel: Vec2::ZERO,
                    facing: default_facing(team),
                    stamina: 1.0,
                    stamina_regen_lock_left: 0.0,
                    shot_charge: 0.0,
                    charge_warmup_left: 0.0,
                });
            }
        }
        out
    }

    #[test]
    fn api_covers_all_dump_labels() {
        let params = SimParams::default();
        let ball = Ball::default();
        let players = sample_players();
        let possession = Possession::default();
        let match_state = MatchState::default();
        let sensors = WorldSensors {
            ball: &ball,
            players: &players,
            possession: &possession,
            match_state: &match_state,
            params: &params,
        };
        let api = build_team_api(TeamId::Home, &sensors);

        for label in GET_BOOL {
            assert!(
                api.get_bool(label).is_some(),
                "missing bool {label}"
            );
        }
        for label in GET_FLOAT {
            assert!(
                api.get_float(label).is_some(),
                "missing float {label}"
            );
        }
        for label in GET_TRANSFORM {
            assert!(
                api.get_transform(label).is_some(),
                "missing transform {label}"
            );
        }
        for label in GET_VECTOR3 {
            assert!(
                api.get_vector3(label).is_some(),
                "missing vector3 {label}"
            );
        }
    }

    #[test]
    fn is_open_is_no_opponent_within_two_interact_radii() {
        let mut params = SimParams::default();
        params.interact_radius = 2.0; // open_r = 4.0
        let open_r = params.interact_radius * 2.0;

        let mut players = sample_players();
        for p in &mut players {
            if p.team == TeamId::Home && p.id == PlayerId(1) {
                p.pos = Vec2::new(0.0, 0.0);
            } else if p.team == TeamId::Away {
                p.pos = Vec2::new(100.0, 0.0);
            }
        }

        let ball = Ball::default();
        let possession = Possession::default();
        let match_state = MatchState::default();
        let api = |players: &[Player]| {
            build_team_api(
                TeamId::Home,
                &WorldSensors {
                    ball: &ball,
                    players,
                    possession: &possession,
                    match_state: &match_state,
                    params: &params,
                },
            )
        };

        assert_eq!(api(&players).get_bool("Is Team Player 1 Open"), Some(true));

        // Just inside 2×R → marked.
        players
            .iter_mut()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(1))
            .unwrap()
            .pos = Vec2::new(open_r - 0.01, 0.0);
        assert_eq!(api(&players).get_bool("Is Team Player 1 Open"), Some(false));

        // Exactly at 2×R is still "within" → not open.
        players
            .iter_mut()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(1))
            .unwrap()
            .pos = Vec2::new(open_r, 0.0);
        assert_eq!(api(&players).get_bool("Is Team Player 1 Open"), Some(false));

        // Just outside → open.
        players
            .iter_mut()
            .find(|p| p.team == TeamId::Away && p.id == PlayerId(1))
            .unwrap()
            .pos = Vec2::new(open_r + 0.01, 0.0);
        assert_eq!(api(&players).get_bool("Is Team Player 1 Open"), Some(true));
    }
}
