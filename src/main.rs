//! Top-down 2D only. No other camera modes.
//!
//! Players = filled discs at nav-agent radius.
//! Small ring in facing direction = BallHoldLocation (capture / shoot / look).

use bevy::color::palettes::css;
use bevy::prelude::*;
use aicomp_soccer_sim::ball::{step_free_ball, Ball, EndReason};
use aicomp_soccer_sim::brain::{
    BrainCommand, BrainOutput, ChaseBallBrain, TeamBrain, TeamId, WorldView,
};
use aicomp_soccer_sim::match_state::{
    kickoff_control_allowed, place_kickoff, MatchPhase, MatchState,
};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::{
    default_facing, faceoff_world, step_mover, Player, PlayerId, SimpleMover,
};
use aicomp_soccer_sim::possession::{
    apply_interact, sync_held_ball, tick_possession_timers, Possession,
};

const FIXED_HZ: f32 = 60.0;
/// Pixels per meter — pitch ~80×50 m fits a 1100×700 window with margin.
const PPM: f32 = 12.0;

fn main() {
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "AIComp Soccer Sim — top-down 2D".into(),
                resolution: (1100, 700).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.08, 0.14, 0.08)))
        .insert_resource(Time::<Fixed>::from_hz(FIXED_HZ as f64))
        .insert_resource(params)
        .insert_resource(MatchState::default())
        .insert_resource(Possession::default())
        .insert_resource(Brains::default())
        .add_systems(Startup, setup)
        .add_systems(Update, (hot_reload_params, draw_topdown))
        .add_systems(FixedUpdate, sim_tick)
        .run();
}

#[derive(Resource)]
struct Brains {
    home: ChaseBallBrain,
    away: ChaseBallBrain,
}

impl Default for Brains {
    fn default() -> Self {
        Self {
            home: ChaseBallBrain,
            away: ChaseBallBrain,
        }
    }
}

fn setup(mut commands: Commands, params: Res<SimParams>, mut match_state: ResMut<MatchState>) {
    // Orthographic top-down only — Camera2d, no orbit / perspective.
    commands.spawn(Camera2d);

    commands.spawn(Ball::default());

    let mover = SimpleMover::from_params(&params);
    for team in [TeamId::Home, TeamId::Away] {
        for id in PlayerId::ALL {
            commands.spawn((
                Player {
                    team,
                    id,
                    pos: faceoff_world(team, id),
                    vel: Vec2::ZERO,
                    facing: default_facing(team),
                    stamina: 1.0,
                    shot_charge: 0.0,
                },
                mover,
            ));
        }
    }

    match_state.kickoff_team = TeamId::Home;
    match_state.opening_kickoff_team = TeamId::Home;
    match_state.phase = MatchPhase::Kickoff;

    info!(
        "top-down 2D | agent_r={:.2} hold_off={:.2} slide_a={}  (R=reload params)",
        params.player_radius, params.hold_offset, params.slide_accel
    );
}

fn hot_reload_params(keys: Res<ButtonInput<KeyCode>>, mut params: ResMut<SimParams>) {
    if keys.just_pressed(KeyCode::KeyR) {
        match params.reload() {
            Ok(()) => info!(
                "reloaded — agent_r={:.2} hold_off={:.2}",
                params.player_radius, params.hold_offset
            ),
            Err(e) => warn!("reload failed: {e}"),
        }
    }
}

fn sim_tick(
    params: Res<SimParams>,
    mut match_state: ResMut<MatchState>,
    mut possession: ResMut<Possession>,
    mut brains: ResMut<Brains>,
    mut q_ball: Query<&mut Ball>,
    mut q_players: Query<(&mut Player, &SimpleMover)>,
) {
    let dt = 1.0 / FIXED_HZ;
    let Ok(mut ball) = q_ball.single_mut() else {
        return;
    };

    if match_state.phase == MatchPhase::GoalPause {
        match_state.phase_timer -= dt;
        if match_state.phase_timer <= 0.0 {
            let mut snap: Vec<Player> = q_players.iter().map(|(p, _)| p.clone()).collect();
            place_kickoff(&mut ball, &mut snap, match_state.kickoff_team);
            possession.carrier = None;
            for (mut p, _) in q_players.iter_mut() {
                if let Some(src) = snap.iter().find(|s| s.team == p.team && s.id == p.id) {
                    *p = src.clone();
                }
            }
            match_state.phase = MatchPhase::Kickoff;
            match_state.phase_timer = 0.0;
        }
        return;
    }

    if match_state.phase == MatchPhase::Kickoff {
        match_state.phase_timer += dt;
        if match_state.phase_timer > 0.5 {
            match_state.phase = MatchPhase::Play;
        }
    } else {
        match_state.clock_s += dt;
    }

    tick_possession_timers(&mut possession, dt);

    let snap: Vec<Player> = q_players.iter().map(|(p, _)| p.clone()).collect();
    let view = WorldView {
        ball: &ball,
        players: &snap,
        dt,
    };
    let home_out = brains.home.think(TeamId::Home, &view);
    let away_out = brains.away.think(TeamId::Away, &view);

    for (mut player, mover) in q_players.iter_mut() {
        let cmd = filter_kickoff(
            &player,
            command_for(&player, &home_out, &away_out),
            &match_state,
            &params,
        );
        step_mover(&mut player, mover, cmd.move_to, cmd.sprint, dt);
        apply_interact(
            &mut player,
            &mut ball,
            &mut possession,
            cmd,
            &params,
            dt,
        );
    }

    let snap: Vec<Player> = q_players.iter().map(|(p, _)| p.clone()).collect();
    sync_held_ball(&mut ball, &snap, &possession, params.hold_offset);

    if !ball.held {
        match step_free_ball(&mut ball, &params, dt) {
            EndReason::GoalHome => {
                info!(
                    "GOAL Away — {}-{}",
                    match_state.score_home,
                    match_state.score_away + 1
                );
                match_state.on_goal(TeamId::Home);
                possession.carrier = None;
            }
            EndReason::GoalAway => {
                info!(
                    "GOAL Home — {}-{}",
                    match_state.score_home + 1,
                    match_state.score_away
                );
                match_state.on_goal(TeamId::Away);
                possession.carrier = None;
            }
            EndReason::None => {}
        }
    }
}

fn command_for(player: &Player, home: &BrainOutput, away: &BrainOutput) -> BrainCommand {
    match player.team {
        TeamId::Home => home.for_player(player.id),
        TeamId::Away => away.for_player(player.id),
    }
}

fn filter_kickoff(
    player: &Player,
    cmd: BrainCommand,
    match_state: &MatchState,
    params: &SimParams,
) -> BrainCommand {
    if kickoff_control_allowed(player.team, player.pos, match_state, params) {
        cmd
    } else {
        BrainCommand {
            move_to: player.pos,
            sprint: false,
            interact: false,
        }
    }
}

/// Single top-down compositor: pitch, players (agent discs), hold rings, ball.
fn draw_topdown(
    mut gizmos: Gizmos,
    params: Res<SimParams>,
    match_state: Res<MatchState>,
    possession: Res<Possession>,
    q_ball: Query<&Ball>,
    q_players: Query<&Player>,
) {
    let s = PPM;

    // Pitch
    let hw = (params.x_max - params.x_min) * 0.5 * s;
    let hh = (params.z_max - params.z_min) * 0.5 * s;
    gizmos.rect_2d(
        Isometry2d::from_translation(Vec2::ZERO),
        Vec2::new(hw * 2.0, hh * 2.0),
        css::SEA_GREEN,
    );
    gizmos.circle_2d(Vec2::ZERO, params.kickoff_circle_r * s, css::WHITE);
    gizmos.line_2d(Vec2::new(0.0, -hh), Vec2::new(0.0, hh), css::WHITE);

    let mouth = params.goal_half_width * s;
    let gx = params.goal_line_x * s;
    gizmos.line_2d(Vec2::new(gx, -mouth), Vec2::new(gx, mouth), css::GOLD);
    gizmos.line_2d(Vec2::new(-gx, -mouth), Vec2::new(-gx, mouth), css::GOLD);

    for p in &q_players {
        let body = match p.team {
            TeamId::Home => css::DODGER_BLUE,
            TeamId::Away => css::TOMATO,
        };
        let center = Vec2::new(p.pos.x * s, p.pos.y * s);

        // Nav-agent footprint (primary body size for 2D).
        gizmos.circle_2d(center, params.player_radius * s, body);

        // BallHoldLocation — look / capture / shoot point in front of body.
        let hold = p.hold_pos(params.hold_offset);
        let hold_px = Vec2::new(hold.x * s, hold.y * s);
        let hold_color = if matches!(
            possession.carrier,
            Some((t, id)) if t == p.team && id == p.id.0
        ) {
            css::ORANGE
        } else {
            css::LIGHT_GRAY
        };
        gizmos.circle_2d(hold_px, params.hold_marker_radius * s, hold_color);
        gizmos.line_2d(center, hold_px, hold_color);

        // Charge wedge hint
        if p.shot_charge > 0.05 {
            let tip = hold_px + p.facing * (0.4 + p.shot_charge) * s;
            gizmos.line_2d(hold_px, tip, css::YELLOW);
        }
    }

    if let Ok(ball) = q_ball.single() {
        gizmos.circle_2d(
            Vec2::new(ball.pos.x * s, ball.pos.y * s),
            params.ball_radius * s,
            css::ORANGE_RED,
        );
    }

    let phase_color = match match_state.phase {
        MatchPhase::Kickoff => css::CYAN,
        MatchPhase::Play => css::LIME,
        MatchPhase::GoalPause => css::RED,
    };
    gizmos.circle_2d(Vec2::new(0.0, (params.z_max + 2.5) * s), 5.0, phase_color);
}
