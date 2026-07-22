//! Interactive viewer: scale-correct pitch, solid white posts, R=restart,
//! click player for debug outlines, Load Team A/B from Soccer saves folder.

use std::path::{Path, PathBuf};

use bevy::picking::prelude::*;
use bevy::prelude::*;
use aicomp_soccer_sim::brain::{BrainOutput, ChaseBallBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph::{load_team_graph, GraphBrain};
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

const PPM: f32 = 10.0;

fn main() {
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });
    let saves = soccer_saves_dir();
    let aia = saves.join("AIA.txt");
    let home_brain = load_brain(&aia);
    let away_brain = load_brain(&aia);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "AIComp Soccer Sim".into(),
                resolution: (1100, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.10, 0.40, 0.16)))
        .insert_resource(Time::<Fixed>::from_hz((1.0 / FIXED_DT) as f64))
        .insert_resource(ViewerWorld {
            world: MatchWorld::new_kickoff(params),
            home: home_brain,
            away: away_brain,
            last_home: BrainOutput::default(),
            last_away: BrainOutput::default(),
        })
        .insert_resource(TeamScripts {
            home_path: aia.clone(),
            away_path: aia,
            status: "graph interpreter MVP (consts/math/getters/controllers)".into(),
        })
        .insert_resource(DebugSelection::default())
        .insert_resource(SimPaused(false))
        .add_systems(Startup, (setup_board, setup_ui))
        .add_systems(FixedUpdate, sim_tick)
        .add_systems(
            Update,
            (
                handle_hotkeys,
                handle_ui_buttons,
                handle_player_click,
                sync_visuals,
                draw_debug,
                refresh_pause_ui,
            ),
        )
        .run();
}

/// `…/AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer` on any Windows user.
fn soccer_saves_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("Unicorn One")
        .join("AIComp")
        .join("Saves")
        .join("Soccer")
}

#[derive(Resource)]
struct ViewerWorld {
    world: MatchWorld,
    home: ActiveBrain,
    away: ActiveBrain,
    last_home: BrainOutput,
    last_away: BrainOutput,
}

/// Graph if loadable, else chase fallback.
enum ActiveBrain {
    Graph(GraphBrain),
    Chase(ChaseBallBrain),
}

impl ActiveBrain {
    fn think(&mut self, api: &aicomp_soccer_sim::api::TeamApi) -> BrainOutput {
        match self {
            ActiveBrain::Graph(g) => g.think(api),
            ActiveBrain::Chase(c) => c.think(api),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ActiveBrain::Graph(_) => "graph",
            ActiveBrain::Chase(_) => "chase-fallback",
        }
    }
}

fn load_brain(path: &Path) -> ActiveBrain {
    match load_team_graph(path) {
        Ok(g) => {
            info!("loaded team graph {} ({} nodes)", path.display(), g.nodes.len());
            ActiveBrain::Graph(GraphBrain::new(g))
        }
        Err(e) => {
            warn!("graph load failed ({path:?}): {e} — using ChaseBallBrain");
            ActiveBrain::Chase(ChaseBallBrain)
        }
    }
}

#[derive(Resource)]
struct TeamScripts {
    home_path: PathBuf,
    away_path: PathBuf,
    status: String,
}

#[derive(Resource, Default)]
struct DebugSelection {
    /// Clicked player; debug outlines drawn for them.
    selected: Option<(TeamId, PlayerId)>,
}

/// When true, FixedUpdate sim steps are skipped (Space / Pause button).
#[derive(Resource, Default)]
struct SimPaused(bool);

#[derive(Component)]
struct PitchMark;

#[derive(Component)]
struct PlayerDisc {
    team: TeamId,
    id: PlayerId,
}

#[derive(Component)]
struct PlayerNum {
    team: TeamId,
    id: PlayerId,
}

#[derive(Component)]
struct BallDisc;

#[derive(Resource)]
struct BallMat(Handle<ColorMaterial>);

#[derive(Component)]
enum UiAction {
    LoadHome,
    LoadAway,
    Restart,
    TogglePause,
}

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct PauseButtonText;

fn setup_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
    viewer: Res<ViewerWorld>,
) {
    commands.spawn((Camera2d, Transform::from_xyz(0.0, -20.0, 0.0)));

    let p = &viewer.world.params;
    let pitch_w = (p.x_max - p.x_min) * PPM;
    let pitch_h = (p.z_max - p.z_min) * PPM;

    // Game `grass_2` texture (extracted). One image = dark+light pair (~10m / ground_line*2).
    let stripe_pair_m = 10.0;
    let tiles_x = (p.x_max - p.x_min) / stripe_pair_m;
    let grass: Handle<Image> = asset_server
        .load_builder()
        .with_settings(|s: &mut bevy::image::ImageLoaderSettings| {
            s.sampler = bevy::image::ImageSampler::Descriptor(
                bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::Repeat,
                    ..Default::default()
                },
            );
        })
        .load("grass.png");
    commands.spawn((
        PitchMark,
        Mesh2d(meshes.add(uv_rect(pitch_w, pitch_h, tiles_x, 1.0))),
        MeshMaterial2d(materials.add(ColorMaterial {
            texture: Some(grass),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    spawn_pitch_lines(&mut commands, &mut meshes, &mut materials, p, pitch_w, pitch_h);

    // Goals: game `net` texture + white frame + solid white posts (collider body).
    let white = materials.add(Color::WHITE);
    let frame = materials.add(Color::srgb(0.97, 0.95, 0.90));
    let net_img: Handle<Image> = asset_server.load("goal_net.png");
    let net_mat = materials.add(ColorMaterial {
        texture: Some(net_img),
        color: Color::WHITE,
        ..default()
    });
    let mouth_h = p.goal_half_width * 2.0 * PPM;
    let post_r_px = p.post_radius * PPM;
    // Visual net depth (top-down goal box) — slightly deeper than post offset so it reads.
    let net_depth = (2.8_f32).max((p.posts_x - p.goal_line_x).abs() + 1.5) * PPM;
    let frame_t = 2.5_f32;

    for sign in [-1.0_f32, 1.0] {
        let line_x = sign * p.goal_line_x * PPM;
        // Outward from pitch: Home goal −X, Away +X.
        let box_cx = sign * (p.goal_line_x.abs() * PPM + net_depth * 0.5);

        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(uv_rect(net_depth, mouth_h, 2.5, 3.0))),
            MeshMaterial2d(net_mat.clone()),
            Transform::from_xyz(box_cx, 0.0, 0.15),
        ));
        // Frame: back + two sides
        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(Rectangle::new(frame_t, mouth_h + frame_t * 2.0))),
            MeshMaterial2d(frame.clone()),
            Transform::from_xyz(sign * (p.goal_line_x.abs() * PPM + net_depth), 0.0, 0.18),
        ));
        for side in [-1.0_f32, 1.0] {
            commands.spawn((
                PitchMark,
                Mesh2d(meshes.add(Rectangle::new(net_depth, frame_t))),
                MeshMaterial2d(frame.clone()),
                Transform::from_xyz(
                    box_cx,
                    side * (p.goal_half_width * PPM + frame_t * 0.5),
                    0.18,
                ),
            ));
        }
        // Goal-line mouth bar
        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(Rectangle::new(frame_t, mouth_h))),
            MeshMaterial2d(white.clone()),
            Transform::from_xyz(line_x, 0.0, 0.2),
        ));
        for pz in [-p.goal_half_width, p.goal_half_width] {
            commands.spawn((
                PitchMark,
                Mesh2d(meshes.add(Circle::new(1.0))),
                MeshMaterial2d(white.clone()),
                Transform::from_xyz(sign * p.posts_x * PPM, pz * PPM, 0.3)
                    .with_scale(Vec3::splat(post_r_px)),
            ));
        }
    }

    let disc_mesh = meshes.add(Circle::new(1.0));
    let interact_px = p.interact_radius * PPM;
    let ball_px = p.ball_radius * PPM;
    let home_mat = materials.add(Color::srgb(0.85, 0.88, 0.95));
    let away_mat = materials.add(Color::srgb(0.12, 0.12, 0.14));
    let ball_mat = materials.add(Color::WHITE);
    commands.insert_resource(BallMat(ball_mat.clone()));

    for player in &viewer.world.players {
        let mat = match player.team {
            TeamId::Home => home_mat.clone(),
            TeamId::Away => away_mat.clone(),
        };
        let num_color = match player.team {
            TeamId::Home => Color::srgb(0.1, 0.1, 0.15),
            TeamId::Away => Color::WHITE,
        };
        let pos = Vec3::new(player.pos.x * PPM, player.pos.y * PPM, 1.0);
        commands.spawn((
            PlayerDisc {
                team: player.team,
                id: player.id,
            },
            Mesh2d(disc_mesh.clone()),
            MeshMaterial2d(mat),
            Transform::from_translation(pos).with_scale(Vec3::splat(interact_px)),
            Pickable::default(),
        ));
        commands.spawn((
            PlayerNum {
                team: player.team,
                id: player.id,
            },
            Text2d::new(format!("{}", player.id.0)),
            TextFont::from_font_size(20.0),
            TextColor(num_color),
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            Transform::from_translation(pos + Vec3::Z * 0.2),
        ));
    }

    commands.spawn((
        BallDisc,
        Mesh2d(disc_mesh),
        MeshMaterial2d(ball_mat),
        Transform::from_xyz(0.0, 0.0, 2.0).with_scale(Vec3::splat(ball_px)),
    ));

    info!(
        "board | grass+net from game assets | interact_r={:.2} post_r={:.2} saves={:?}",
        p.interact_radius,
        p.post_radius,
        soccer_saves_dir()
    );
}

/// Rectangle mesh with custom UVs for repeating textures.
fn uv_rect(w: f32, h: f32, tiles_u: f32, tiles_v: f32) -> Mesh {
    let hw = w * 0.5;
    let hh = h * 0.5;
    Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-hw, -hh, 0.0],
            [hw, -hh, 0.0],
            [hw, hh, 0.0],
            [-hw, hh, 0.0],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![
            [0.0, 0.0],
            [tiles_u, 0.0],
            [tiles_u, tiles_v],
            [0.0, tiles_v],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    )
    .with_inserted_indices(bevy::mesh::Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

fn spawn_pitch_lines(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    p: &SimParams,
    pitch_w: f32,
    pitch_h: f32,
) {
    // Pitch lines are geometry in AIComp (no dedicated chalk texture) — draw to scale.
    let chalk = materials.add(Color::srgba(1.0, 1.0, 1.0, 0.92));
    let lw = 1.8_f32; // line thickness in px
    let x_min = p.x_min * PPM;
    let x_max = p.x_max * PPM;
    let z_min = p.z_min * PPM;
    let z_max = p.z_max * PPM;
    let area_d = 12.5 * PPM; // Area Depth
    let goal_area_d = 5.5 * PPM;
    let pen_half = 10.0 * PPM;
    let goal_box_half = 7.0 * PPM;
    let d_r = 2.5 * PPM; // Arena Semicircle Depth

    let line = |meshes: &mut Assets<Mesh>,
                commands: &mut Commands,
                w: f32,
                h: f32,
                x: f32,
                y: f32| {
        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(Rectangle::new(w, h))),
            MeshMaterial2d(chalk.clone()),
            Transform::from_xyz(x, y, 0.12),
        ));
    };

    // Outer boundary
    line(meshes, commands, pitch_w, lw, 0.0, z_max);
    line(meshes, commands, pitch_w, lw, 0.0, z_min);
    line(meshes, commands, lw, pitch_h, x_min, 0.0);
    line(meshes, commands, lw, pitch_h, x_max, 0.0);
    // Halfway line
    line(meshes, commands, lw, pitch_h, 0.0, 0.0);
    // Center circle
    commands.spawn((
        PitchMark,
        Mesh2d(meshes.add(Annulus::new(
            p.kickoff_circle_r * PPM - lw,
            p.kickoff_circle_r * PPM,
        ))),
        MeshMaterial2d(chalk.clone()),
        Transform::from_xyz(0.0, 0.0, 0.12),
    ));
    // Center spot
    commands.spawn((
        PitchMark,
        Mesh2d(meshes.add(Circle::new(2.0))),
        MeshMaterial2d(chalk.clone()),
        Transform::from_xyz(0.0, 0.0, 0.13),
    ));

    for sign in [-1.0_f32, 1.0] {
        let goal_x = sign * p.goal_line_x * PPM;
        let pen_inner = goal_x - sign * area_d;
        let ga_inner = goal_x - sign * goal_area_d;
        // Penalty box
        line(
            meshes,
            commands,
            area_d,
            lw,
            goal_x - sign * area_d * 0.5,
            pen_half,
        );
        line(
            meshes,
            commands,
            area_d,
            lw,
            goal_x - sign * area_d * 0.5,
            -pen_half,
        );
        line(meshes, commands, lw, pen_half * 2.0, pen_inner, 0.0);
        // Goal area
        line(
            meshes,
            commands,
            goal_area_d,
            lw,
            goal_x - sign * goal_area_d * 0.5,
            goal_box_half,
        );
        line(
            meshes,
            commands,
            goal_area_d,
            lw,
            goal_x - sign * goal_area_d * 0.5,
            -goal_box_half,
        );
        line(meshes, commands, lw, goal_box_half * 2.0, ga_inner, 0.0);
        // Penalty spot
        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(Circle::new(1.8))),
            MeshMaterial2d(chalk.clone()),
            Transform::from_xyz(goal_x - sign * 11.0 * PPM, 0.0, 0.13),
        ));
        // Penalty arc (approx annulus segment via full thin circle clipped visually)
        let arc_cx = pen_inner;
        commands.spawn((
            PitchMark,
            Mesh2d(meshes.add(Annulus::new(
                9.15 * PPM - lw,
                9.15 * PPM,
            ))),
            MeshMaterial2d(materials.add(Color::srgba(1.0, 1.0, 1.0, 0.55))),
            Transform::from_xyz(arc_cx + sign * d_r * 0.2, 0.0, 0.11),
        ));
    }
}

fn setup_ui(mut commands: Commands, scripts: Res<TeamScripts>) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(48.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.92)),
        ))
        .with_children(|parent| {
            ui_btn(parent, "Load Team A (home/left)", UiAction::LoadHome);
            ui_btn(parent, "Load Team B (away/right)", UiAction::LoadAway);
            ui_btn(parent, "Restart (R)", UiAction::Restart);
            parent
                .spawn((
                    Button,
                    UiAction::TogglePause,
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.2, 0.2, 0.28)),
                ))
                .with_children(|b| {
                    b.spawn((
                        PauseButtonText,
                        Text::new("Pause (Space)"),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::WHITE),
                    ));
                });
            parent.spawn((
                StatusText,
                Text::new(format!(
                    "A: {} | B: {} | {}",
                    file_stem(&scripts.home_path),
                    file_stem(&scripts.away_path),
                    scripts.status
                )),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));
        });
}

fn ui_btn(parent: &mut ChildSpawnerCommands, label: &str, action: UiAction) {
    parent
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.2, 0.2, 0.28)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont::from_font_size(14.0),
                TextColor(Color::WHITE),
            ));
        });
}

fn file_stem(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

fn sim_tick(paused: Res<SimPaused>, mut viewer: ResMut<ViewerWorld>) {
    if paused.0 {
        return;
    }
    let (home_api, away_api) = viewer.world.build_apis();
    let home_out = viewer.home.think(&home_api);
    let away_out = viewer.away.think(&away_api);
    viewer.last_home = home_out.clone();
    viewer.last_away = away_out.clone();
    viewer
        .world
        .step_with_commands(&home_out, &away_out, FIXED_DT);
}

fn handle_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut viewer: ResMut<ViewerWorld>,
    mut selection: ResMut<DebugSelection>,
    mut paused: ResMut<SimPaused>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        restart_match(&mut viewer);
        selection.selected = None;
    }
    if keys.just_pressed(KeyCode::Space) {
        toggle_pause(&mut paused);
    }
}

fn toggle_pause(paused: &mut SimPaused) {
    paused.0 = !paused.0;
    info!(paused = paused.0, "simulation pause toggled");
}

fn refresh_pause_ui(
    paused: Res<SimPaused>,
    mut pause_label: Query<&mut Text, With<PauseButtonText>>,
    mut status_q: Query<&mut Text, (With<StatusText>, Without<PauseButtonText>)>,
    scripts: Res<TeamScripts>,
) {
    if !paused.is_changed() {
        return;
    }
    if let Ok(mut text) = pause_label.single_mut() {
        *text = Text::new(if paused.0 {
            "Resume (Space)"
        } else {
            "Pause (Space)"
        });
    }
    if let Ok(mut text) = status_q.single_mut() {
        let run = if paused.0 { "PAUSED" } else { "RUNNING" };
        *text = Text::new(format!(
            "[{}] A: {} | B: {} | {}",
            run,
            file_stem(&scripts.home_path),
            file_stem(&scripts.away_path),
            scripts.status
        ));
    }
}

fn restart_match(viewer: &mut ViewerWorld) {
    let params = viewer.world.params.clone();
    viewer.world = MatchWorld::new_kickoff(params);
    viewer.last_home = BrainOutput::default();
    viewer.last_away = BrainOutput::default();
    info!("match restarted");
}

fn handle_ui_buttons(
    mut interactions: Query<
        (&Interaction, &UiAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut viewer: ResMut<ViewerWorld>,
    mut scripts: ResMut<TeamScripts>,
    mut status_q: Query<&mut Text, (With<StatusText>, Without<PauseButtonText>)>,
    mut selection: ResMut<DebugSelection>,
    mut paused: ResMut<SimPaused>,
) {
    for (interaction, action, mut bg) in &mut interactions {
        match *interaction {
            Interaction::Hovered => *bg = BackgroundColor(Color::srgb(0.3, 0.3, 0.4)),
            Interaction::None => *bg = BackgroundColor(Color::srgb(0.2, 0.2, 0.28)),
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgb(0.15, 0.45, 0.25));
                match action {
                    UiAction::Restart => {
                        restart_match(&mut viewer);
                        selection.selected = None;
                    }
                    UiAction::TogglePause => {
                        toggle_pause(&mut paused);
                    }
                    UiAction::LoadHome => {
                        if let Some(path) = pick_team_script("Load Team A (home / left)") {
                            viewer.home = load_brain(&path);
                            scripts.home_path = path;
                            scripts.status = format!(
                                "Team A {} ({})",
                                viewer.home.label(),
                                file_stem(&scripts.home_path)
                            );
                        }
                    }
                    UiAction::LoadAway => {
                        if let Some(path) = pick_team_script("Load Team B (away / right)") {
                            viewer.away = load_brain(&path);
                            scripts.away_path = path;
                            scripts.status = format!(
                                "Team B {} ({})",
                                viewer.away.label(),
                                file_stem(&scripts.away_path)
                            );
                        }
                    }
                }
                if let Ok(mut text) = status_q.single_mut() {
                    let run = if paused.0 { "PAUSED" } else { "RUNNING" };
                    *text = Text::new(format!(
                        "[{}] A: {} | B: {} | {}",
                        run,
                        file_stem(&scripts.home_path),
                        file_stem(&scripts.away_path),
                        scripts.status
                    ));
                }
            }
        }
    }
}

fn pick_team_script(title: &str) -> Option<PathBuf> {
    let dir = soccer_saves_dir();
    rfd::FileDialog::new()
        .set_title(title)
        .set_directory(&dir)
        .add_filter("AIComp team", &["txt"])
        .pick_file()
}

fn handle_player_click(
    mut clicks: MessageReader<Pointer<Click>>,
    discs: Query<&PlayerDisc>,
    mut selection: ResMut<DebugSelection>,
) {
    for click in clicks.read() {
        if let Ok(disc) = discs.get(click.entity) {
            selection.selected = Some((disc.team, disc.id));
            info!("debug select {:?} player {}", disc.team, disc.id.0);
        }
    }
}

fn sync_visuals(
    viewer: Res<ViewerWorld>,
    ball_mat: Res<BallMat>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut q_disc: Query<(&PlayerDisc, &mut Transform), Without<BallDisc>>,
    mut q_num: Query<(&PlayerNum, &mut Transform), (Without<BallDisc>, Without<PlayerDisc>)>,
    mut q_ball: Query<&mut Transform, (With<BallDisc>, Without<PlayerDisc>)>,
) {
    let w = &viewer.world;
    for (disc, mut tf) in &mut q_disc {
        if let Some(p) = w.players.iter().find(|p| p.team == disc.team && p.id == disc.id) {
            tf.translation.x = p.pos.x * PPM;
            tf.translation.y = p.pos.y * PPM;
        }
    }
    for (num, mut tf) in &mut q_num {
        if let Some(p) = w.players.iter().find(|p| p.team == num.team && p.id == num.id) {
            tf.translation.x = p.pos.x * PPM;
            tf.translation.y = p.pos.y * PPM;
        }
    }
    if let Ok(mut tf) = q_ball.single_mut() {
        tf.translation.x = w.ball.pos.x * PPM;
        tf.translation.y = w.ball.pos.y * PPM;
    }
    let charge = w
        .possession
        .carrier
        .and_then(|(t, id)| {
            w.players
                .iter()
                .find(|p| p.team == t && p.id.0 == id)
                .map(|p| p.shot_charge)
        })
        .unwrap_or(0.0);
    if let Some(mut mat) = materials.get_mut(&ball_mat.0) {
        mat.color = Color::srgb(1.0, 1.0 - charge, 1.0 - charge);
    }
}

fn draw_debug(
    mut gizmos: Gizmos,
    viewer: Res<ViewerWorld>,
    selection: Res<DebugSelection>,
) {
    let Some((team, id)) = selection.selected else {
        return;
    };
    let w = &viewer.world;
    let Some(p) = w.players.iter().find(|p| p.team == team && p.id == id) else {
        return;
    };
    let params = &w.params;
    let c = Vec2::new(p.pos.x * PPM, p.pos.y * PPM);

    // Hollow interact outline
    gizmos.circle_2d(c, params.interact_radius * PPM, Color::WHITE);
    // Hollow contact post scale reference around player (body)
    gizmos.circle_2d(c, params.body_radius * PPM, Color::srgb(0.8, 0.8, 0.8));

    let cmd = match team {
        TeamId::Home => viewer.last_home.for_player(id),
        TeamId::Away => viewer.last_away.for_player(id),
    };
    let dest = Vec2::new(cmd.move_to.x * PPM, cmd.move_to.y * PPM);
    gizmos.line_2d(c, dest, Color::srgb(1.0, 1.0, 0.2));
    gizmos.circle_2d(dest, 3.0, Color::srgb(1.0, 1.0, 0.2));

    let hold = p.hold_pos(params.hold_offset);
    let hold_px = Vec2::new(hold.x * PPM, hold.y * PPM);
    gizmos.circle_2d(hold_px, params.hold_marker_radius * PPM, Color::srgb(1.0, 0.6, 0.1));
    gizmos.line_2d(c, hold_px, Color::srgb(1.0, 0.6, 0.1));

    // Facing
    let tip = c + p.facing * (params.interact_radius * PPM + 8.0);
    gizmos.line_2d(c, tip, Color::srgb(0.2, 1.0, 1.0));
}
