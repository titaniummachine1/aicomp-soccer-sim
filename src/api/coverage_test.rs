//! Coverage test: every dump label is present on a TeamApi snapshot.

#[cfg(test)]
mod tests {
    use bevy::prelude::Vec2;
    use crate::api::{
        build_team_api, GET_BOOL, GET_FLOAT, GET_TRANSFORM, GET_VECTOR3, WorldSensors,
    };
    use crate::ball::Ball;
    use crate::brain::TeamId;
    use crate::match_state::MatchState;
    use crate::params::SimParams;
    use crate::player::{default_facing, faceoff_world, Player, PlayerId};
    use crate::possession::Possession;

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
                api.bools.contains_key(label),
                "missing bool {label}"
            );
        }
        for label in GET_FLOAT {
            assert!(
                api.floats.contains_key(label),
                "missing float {label}"
            );
        }
        for label in GET_TRANSFORM {
            assert!(
                api.transforms.contains_key(label),
                "missing transform {label}"
            );
        }
        for label in GET_VECTOR3 {
            assert!(
                api.vectors.contains_key(label),
                "missing vector3 {label}"
            );
        }
    }
}
