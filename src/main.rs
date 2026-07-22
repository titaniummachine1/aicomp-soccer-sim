//! Interactive Bevy viewer (window). For batch AI eval use `soccer_headless`.
//!
//! ```text
//! cargo run --release
//! cargo run --release --bin soccer_headless -- --help
//! ```
//!
//! See README.md / AGENTS.md. Hotkeys: Space=pause, R=reload params.

use std::path::{Path, PathBuf};

use aicomp_soccer_sim::brain::{BrainOutput, ChaseBallBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph::load_team_graph;
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::team_threads::{think_barrier, ThinkTimings};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use bevy::picking::prelude::*;
use bevy::prelude::*;
use bevy::window::PresentMode;

const PPM: f32 = 10.0;
/// Render target. Sim ticks stay on FIXED_DT (≈52.6 Hz / 19 ms).
const RENDER_HZ: f64 = 60.0;
/// Max sim ticks processed in one render frame while catching up (never skip).
#[allow(dead_code)]
const MAX_TICKS_PER_FRAME: u32 = 1;
/// Status / tick HUD text refresh rate (keep readable).
const UI_HZ: f32 = 10.0;

/// Draw order when players overlap (higher Z draws on top).
/// Left(Home) P1 → Right(Away) P1 → Left P2 → … → Right P4.
fn player_z(team: TeamId, id: PlayerId) -> f32 {
    let slot = id.0.saturating_sub(1).min(3) as u32;
    let team_bit = match team {
        TeamId::Home => 0u32,
        TeamId::Away => 1u32,
    };
    let rank = slot * 2 + team_bit; // 0 = topmost
    10.0 - rank as f32
}

/// Number sits on its own disc (tiny epsilon) but below the next player's disc.
const NUM_Z_EPS: f32 = 0.05;
/// Stamina arcs are **children** of the player disc (unit-space mesh), so they
/// inherit the disc's `player_z` exactly — no separate world-Z that can sort wrong.
/// Half-width of the stroked stamina arc in **pixels** (converted to unit-disc space).
const STAMINA_ARC_HALF_W_PX: f32 = 1.75;
/// Pixel gap between disc edge and arc centerline.
const STAMINA_ARC_PAD_PX: f32 = 3.0;

fn portable_asset_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("assets");
            if beside.is_dir() {
                return beside;
            }
        }
    }
    let cwd = PathBuf::from("assets");
    if cwd.is_dir() {
        return cwd;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn main() {
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });
    let saves = soccer_saves_dir();
    let aia = saves.join("AIA.txt");
    let home_brain = load_brain(&aia);
    let away_brain = load_brain(&aia);

    let asset_root = portable_asset_root();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "AIComp Soccer Sim".into(),
                        resolution: (1100, 720).into(),
                        present_mode: PresentMode::AutoVsync, // ~60 FPS present
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.10, 0.40, 0.16)))
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
        .insert_resource(TickClock::default())
        .insert_resource(InterpState::default())
        .insert_resource(UiPulse::default())
        .add_systems(Startup, (setup_board, setup_ui))
        .add_systems(
            Update,
            (
                sim_tick_barrier,
                tick_ui_pulse,
                handle_hotkeys,
                handle_ui_buttons,
                handle_player_click,
                sync_visuals,
                sync_stamina_arcs,
                draw_debug,
                refresh_pause_ui,
                refresh_tick_hud,
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

/// Compiled Graph VM (O1) if loadable, else chase fallback.
enum ActiveBrain {
    Runtime(RuntimeBrain),
    Chase(ChaseBallBrain),
}

impl ActiveBrain {
    fn label(&self) -> &'static str {
        match self {
            ActiveBrain::Runtime(_) => "runtime",
            ActiveBrain::Chase(_) => "chase-fallback",
        }
    }
}

impl TeamBrain for ActiveBrain {
    fn think(&mut self, api: &aicomp_soccer_sim::api::TeamApi) -> BrainOutput {
        match self {
            ActiveBrain::Runtime(g) => g.think(api),
            ActiveBrain::Chase(c) => c.think(api),
        }
    }
}

fn load_brain(path: &Path) -> ActiveBrain {
    match load_team_graph(path) {
        Ok(g) => {
            info!(
                "loaded team graph {} ({} nodes) → RuntimeBrain O1",
                path.display(),
                g.nodes.len()
            );
            ActiveBrain::Runtime(RuntimeBrain::compile(g))
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

/// When true, sim ticks are skipped (Space / Pause button). Render still runs @ ~60 FPS.
#[derive(Resource, Default)]
struct SimPaused(bool);

/// Fixed-dt accumulator + barrier timings (both teams must finish before step).
#[derive(Resource)]
struct TickClock {
    accumulator: f32,
    /// 0..1 blend prev→curr for rendering between sim ticks.
    alpha: f32,
    last: ThinkTimings,
    physics_ms: f32,
    tick_ms: f32,
    ticks_this_frame: u32,
    backlog_ticks: f32,
}

impl Default for TickClock {
    fn default() -> Self {
        Self {
            accumulator: 0.0,
            alpha: 1.0,
            last: ThinkTimings::default(),
            physics_ms: 0.0,
            tick_ms: 0.0,
            ticks_this_frame: 0,
            backlog_ticks: 0.0,
        }
    }
}

/// Positions at previous / current sim tick for render interpolation.
#[derive(Resource, Clone)]
struct InterpState {
    prev_ball: Vec2,
    curr_ball: Vec2,
    prev_players: Vec<(TeamId, PlayerId, Vec2)>,
    curr_players: Vec<(TeamId, PlayerId, Vec2)>,
    primed: bool,
}

impl Default for InterpState {
    fn default() -> Self {
        Self {
            prev_ball: Vec2::ZERO,
            curr_ball: Vec2::ZERO,
            prev_players: Vec::new(),
            curr_players: Vec::new(),
            primed: false,
        }
    }
}

impl InterpState {
    fn reset_from(&mut self, world: &MatchWorld) {
        let curr_players = world
            .players
            .iter()
            .map(|p| (p.team, p.id, p.pos))
            .collect::<Vec<_>>();
        self.prev_ball = world.ball.pos;
        self.curr_ball = world.ball.pos;
        self.prev_players = curr_players.clone();
        self.curr_players = curr_players;
        self.primed = true;
    }

    fn push_tick(&mut self, world: &MatchWorld) {
        if !self.primed {
            self.reset_from(world);
            return;
        }
        self.prev_ball = self.curr_ball;
        self.prev_players = std::mem::take(&mut self.curr_players);
        self.curr_ball = world.ball.pos;
        self.curr_players = world
            .players
            .iter()
            .map(|p| (p.team, p.id, p.pos))
            .collect();
    }

    fn ball(&self, alpha: f32) -> Vec2 {
        self.prev_ball.lerp(self.curr_ball, alpha)
    }

    fn player(&self, team: TeamId, id: PlayerId, alpha: f32) -> Option<Vec2> {
        let prev = self
            .prev_players
            .iter()
            .find(|(t, i, _)| *t == team && *i == id)
            .map(|(_, _, p)| *p)?;
        let curr = self
            .curr_players
            .iter()
            .find(|(t, i, _)| *t == team && *i == id)
            .map(|(_, _, p)| *p)?;
        Some(prev.lerp(curr, alpha))
    }
}

#[derive(Component)]
struct TickHudText;

/// Throttle text HUD refreshes so they stay readable (~10 Hz).
#[derive(Resource)]
struct UiPulse {
    accum: f32,
    /// Latched true for one Update after interval elapses.
    fire: bool,
}

impl Default for UiPulse {
    fn default() -> Self {
        Self {
            accum: 0.0,
            fire: true,
        }
    }
}

fn tick_ui_pulse(time: Res<Time>, mut pulse: ResMut<UiPulse>) {
    pulse.fire = false;
    pulse.accum += time.delta_secs();
    let period = 1.0 / UI_HZ;
    if pulse.accum >= period {
        pulse.accum %= period;
        pulse.fire = true;
    }
}

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

/// Mesh2d stamina ring segment; `filled` selects empty (black) vs charge (white).
#[derive(Component)]
struct PlayerStaminaArc {
    team: TeamId,
    id: PlayerId,
    filled: bool,
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
            s.sampler =
                bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::Repeat,
                    ..Default::default()
                });
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

    spawn_pitch_lines(
        &mut commands,
        &mut meshes,
        &mut materials,
        p,
        pitch_w,
        pitch_h,
    );

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
    let stamina_empty_mat = materials.add(Color::BLACK);
    let stamina_fill_mat = materials.add(Color::WHITE);
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
        let z = player_z(player.team, player.id);
        let pos = Vec3::new(player.pos.x * PPM, player.pos.y * PPM, z);
        // Arc in unit-disc space (parent scales by interact_px) so children share disc Z.
        let arc_r = 1.0 + STAMINA_ARC_PAD_PX / interact_px;
        let arc_hw = STAMINA_ARC_HALF_W_PX / interact_px;
        let empty_mesh = meshes.add(stroked_arc_mesh(
            arc_r,
            arc_hw,
            std::f32::consts::PI,
            0.0,
            28,
        ));
        let fill_mesh = meshes.add(stroked_arc_mesh(
            arc_r,
            arc_hw,
            std::f32::consts::PI,
            std::f32::consts::PI,
            2,
        ));
        commands
            .spawn((
                PlayerDisc {
                    team: player.team,
                    id: player.id,
                },
                Mesh2d(disc_mesh.clone()),
                MeshMaterial2d(mat),
                Transform::from_translation(pos).with_scale(Vec3::splat(interact_px)),
                Pickable::default(),
            ))
            .with_children(|child| {
                child.spawn((
                    PlayerStaminaArc {
                        team: player.team,
                        id: player.id,
                        filled: false,
                    },
                    Mesh2d(empty_mesh),
                    MeshMaterial2d(stamina_empty_mat.clone()),
                    Transform::IDENTITY,
                ));
                child.spawn((
                    PlayerStaminaArc {
                        team: player.team,
                        id: player.id,
                        filled: true,
                    },
                    Mesh2d(fill_mesh),
                    MeshMaterial2d(stamina_fill_mat.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ));
            });
        commands.spawn((
            PlayerNum {
                team: player.team,
                id: player.id,
            },
            Text2d::new(format!("{}", player.id.0)),
            TextFont::from_font_size(20.0),
            TextColor(num_color),
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            // Same stack as disc so an occluding player covers both circle and number.
            Transform::from_translation(pos + Vec3::Z * NUM_Z_EPS),
        ));
    }

    commands.spawn((
        BallDisc,
        Mesh2d(disc_mesh),
        MeshMaterial2d(ball_mat),
        Transform::from_xyz(0.0, 0.0, 12.0).with_scale(Vec3::splat(ball_px)),
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

    let line =
        |meshes: &mut Assets<Mesh>, commands: &mut Commands, w: f32, h: f32, x: f32, y: f32| {
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
            Mesh2d(meshes.add(Annulus::new(9.15 * PPM - lw, 9.15 * PPM))),
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
                    "[RUNNING] 0-0 | A: {} | B: {} | {}",
                    file_stem(&scripts.home_path),
                    file_stem(&scripts.away_path),
                    scripts.status
                )),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));
        });

    // Top-right tick / brain timing HUD (Unity FIXED_DT budget ≈ 19 ms).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(56.0),
                right: Val::Px(12.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.04, 0.82)),
        ))
        .with_children(|p| {
            p.spawn((
                TickHudText,
                Text::new("tick —"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.85, 0.95, 0.85)),
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

fn sim_tick_barrier(
    time: Res<Time>,
    paused: Res<SimPaused>,
    mut viewer: ResMut<ViewerWorld>,
    mut clock: ResMut<TickClock>,
    mut interp: ResMut<InterpState>,
) {
    if !interp.primed {
        interp.reset_from(&viewer.world);
    }
    if paused.0 {
        clock.ticks_this_frame = 0;
        clock.alpha = 1.0;
        clock.backlog_ticks = 0.0;
        return;
    }

    // Advance sim with FIXED_DT only. Never burst multiple ticks to catch wall-clock:
    // if a tick (brains+physics) is slow, match time slows with it — same tick sequence
    // on every PC, just stretched in real time.
    clock.accumulator += time.delta_secs();
    clock.ticks_this_frame = 0;

    if clock.accumulator >= FIXED_DT {
        let tick0 = std::time::Instant::now();
        let ViewerWorld {
            world,
            home,
            away,
            last_home,
            last_away,
        } = &mut *viewer;
        let (home_api, away_api) = world.build_apis();
        // Barrier: both brains finish before physics (no partial ticks).
        let (home_out, away_out, timings) = think_barrier(home, away, home_api, away_api);
        *last_home = home_out.clone();
        *last_away = away_out.clone();

        let phys0 = std::time::Instant::now();
        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        clock.physics_ms = phys0.elapsed().as_secs_f32() * 1000.0;
        interp.push_tick(world);

        clock.last = timings;
        clock.tick_ms = tick0.elapsed().as_secs_f32() * 1000.0;
        clock.ticks_this_frame = 1;
        clock.accumulator -= FIXED_DT;
        // Discard overtime instead of queueing extra ticks (proportional slowdown).
        if clock.accumulator >= FIXED_DT {
            clock.backlog_ticks = clock.accumulator / FIXED_DT;
            clock.accumulator = 0.0;
        } else {
            clock.backlog_ticks = 0.0;
        }

        #[cfg(debug_assertions)]
        {
            let budget_ms = FIXED_DT * 1000.0;
            let ball = world.ball.pos;
            let near_wall = ball.x.abs() > world.params.x_max - 2.0
                || ball.y.abs() > world.params.z_max - 2.0;
            let overdue = clock.tick_ms > budget_ms * 1.05;
            if overdue || (near_wall && clock.tick_ms > budget_ms * 0.5) {
                eprintln!(
                    "[spike] t={:.2}s tick={:.2}ms home={:.2}ms away={:.2}ms phys={:.2}ms \
                     ball=({:.1},{:.1}) vel={:.1} held={} phase={:?} near_wall={near_wall}",
                    world.match_state.clock_s,
                    clock.tick_ms,
                    clock.last.home_ms(),
                    clock.last.away_ms(),
                    clock.physics_ms,
                    ball.x,
                    ball.y,
                    world.ball.vel.length(),
                    world.ball.held,
                    world.match_state.phase,
                );
            }
        }
    }

    clock.alpha = (clock.accumulator / FIXED_DT).clamp(0.0, 1.0);
}

fn handle_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut viewer: ResMut<ViewerWorld>,
    mut selection: ResMut<DebugSelection>,
    mut paused: ResMut<SimPaused>,
    mut interp: ResMut<InterpState>,
    mut clock: ResMut<TickClock>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        restart_match(&mut viewer, &mut interp, &mut clock);
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
    viewer: Res<ViewerWorld>,
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
            "[{}] {}-{} | A: {} | B: {} | {}",
            run,
            viewer.world.match_state.score_home,
            viewer.world.match_state.score_away,
            file_stem(&scripts.home_path),
            file_stem(&scripts.away_path),
            scripts.status
        ));
    }
}

fn restart_match(viewer: &mut ViewerWorld, interp: &mut InterpState, clock: &mut TickClock) {
    let params = viewer.world.params.clone();
    viewer.world = MatchWorld::new_kickoff(params);
    viewer.last_home = BrainOutput::default();
    viewer.last_away = BrainOutput::default();
    interp.reset_from(&viewer.world);
    clock.accumulator = 0.0;
    clock.alpha = 1.0;
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
    mut interp: ResMut<InterpState>,
    mut clock: ResMut<TickClock>,
) {
    for (interaction, action, mut bg) in &mut interactions {
        match *interaction {
            Interaction::Hovered => *bg = BackgroundColor(Color::srgb(0.3, 0.3, 0.4)),
            Interaction::None => *bg = BackgroundColor(Color::srgb(0.2, 0.2, 0.28)),
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgb(0.15, 0.45, 0.25));
                match action {
                    UiAction::Restart => {
                        restart_match(&mut viewer, &mut interp, &mut clock);
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
                        "[{}] {}-{} | A: {} | B: {} | {}",
                        run,
                        viewer.world.match_state.score_home,
                        viewer.world.match_state.score_away,
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
    interp: Res<InterpState>,
    clock: Res<TickClock>,
    pulse: Res<UiPulse>,
    ball_mat: Res<BallMat>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut q_disc: Query<(&PlayerDisc, &mut Transform), Without<BallDisc>>,
    mut q_num: Query<(&PlayerNum, &mut Transform), (Without<BallDisc>, Without<PlayerDisc>)>,
    mut q_ball: Query<&mut Transform, (With<BallDisc>, Without<PlayerDisc>)>,
    paused: Res<SimPaused>,
    scripts: Res<TeamScripts>,
    mut status_q: Query<&mut Text, (With<StatusText>, Without<PauseButtonText>)>,
) {
    let w = &viewer.world;
    let alpha = if paused.0 { 1.0 } else { clock.alpha };
    for (disc, mut tf) in &mut q_disc {
        let pos = interp.player(disc.team, disc.id, alpha).or_else(|| {
            w.players
                .iter()
                .find(|p| p.team == disc.team && p.id == disc.id)
                .map(|p| p.pos)
        });
        if let Some(pos) = pos {
            let z = player_z(disc.team, disc.id);
            tf.translation = Vec3::new(pos.x * PPM, pos.y * PPM, z);
        }
    }
    for (num, mut tf) in &mut q_num {
        let pos = interp.player(num.team, num.id, alpha).or_else(|| {
            w.players
                .iter()
                .find(|p| p.team == num.team && p.id == num.id)
                .map(|p| p.pos)
        });
        if let Some(pos) = pos {
            let z = player_z(num.team, num.id) + NUM_Z_EPS;
            tf.translation = Vec3::new(pos.x * PPM, pos.y * PPM, z);
        }
    }
    if let Ok(mut tf) = q_ball.single_mut() {
        let pos = interp.ball(alpha);
        tf.translation.x = pos.x * PPM;
        tf.translation.y = pos.y * PPM;
        tf.translation.z = 12.0;
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
    // Status strip only at 10 Hz.
    if pulse.fire {
        if let Ok(mut text) = status_q.single_mut() {
            let run = if paused.0 { "PAUSED" } else { "RUNNING" };
            *text = Text::new(format!(
                "[{}] {}-{} | A: {} | B: {} | {}",
                run,
                w.match_state.score_home,
                w.match_state.score_away,
                file_stem(&scripts.home_path),
                file_stem(&scripts.away_path),
                scripts.status
            ));
        }
    }
}

fn refresh_tick_hud(
    clock: Res<TickClock>,
    pulse: Res<UiPulse>,
    mut hud: Query<(&mut Text, &mut TextColor), With<TickHudText>>,
) {
    if !pulse.fire {
        return;
    }
    let Ok((mut text, mut color)) = hud.single_mut() else {
        return;
    };
    let budget = FIXED_DT * 1000.0;
    let overdue = clock.tick_ms > budget * 1.05;
    let slow = clock.last.slowest_label();
    let home_ms = clock.last.home_ms();
    let away_ms = clock.last.away_ms();
    let flag = if overdue { " OVER" } else { "" };
    *text = Text::new(format!(
        "render ~{:.0}fps | UI {}Hz | sim dt {:.1}ms\n\
         tick {:.2}ms{flag} (budget {:.1}) | slow-mo if OVER\n\
         Home(A) think {home_ms:.2}ms | Away(B) {away_ms:.2}ms\n\
         slowest: {slow} | phys {:.2}ms | dropped {:.1}t | did {}",
        RENDER_HZ,
        UI_HZ,
        budget,
        clock.tick_ms,
        budget,
        clock.physics_ms,
        clock.backlog_ticks,
        clock.ticks_this_frame,
    ));
    *color = TextColor(if overdue {
        Color::srgb(1.0, 0.35, 0.3)
    } else {
        Color::srgb(0.75, 0.95, 0.75)
    });
}

/// Update fill amount only — arcs are disc children, so XY/Z follow the circle.
fn sync_stamina_arcs(
    viewer: Res<ViewerWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<(&PlayerStaminaArc, &Mesh2d, &mut Visibility)>,
) {
    let interact_px = viewer.world.params.interact_radius * PPM;
    let arc_r = 1.0 + STAMINA_ARC_PAD_PX / interact_px;
    let arc_hw = STAMINA_ARC_HALF_W_PX / interact_px;

    for (arc, mesh2d, mut vis) in &mut q {
        let stamina = viewer
            .world
            .players
            .iter()
            .find(|p| p.team == arc.team && p.id == arc.id)
            .map(|p| p.stamina.clamp(0.0, 1.0))
            .unwrap_or(1.0);

        if !arc.filled {
            continue;
        }
        if stamina > 0.001 {
            *vis = Visibility::Visible;
            let a1 = std::f32::consts::PI * (1.0 - stamina);
            if let Some(mut mesh) = meshes.get_mut(&mesh2d.0) {
                *mesh = stroked_arc_mesh(arc_r, arc_hw, std::f32::consts::PI, a1, 28);
            }
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

/// Thin stroked arc centered at origin (XY), for Mesh2d at the player transform.
fn stroked_arc_mesh(radius: f32, half_w: f32, a0: f32, a1: f32, segments: u32) -> Mesh {
    let segs = segments.max(1);
    // Degenerate: keep a tiny visible stub so the asset stays valid.
    let a1 = if (a1 - a0).abs() < 1e-5 {
        a0 + 1e-4
    } else {
        a1
    };
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((segs as usize + 1) * 2);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((segs as usize + 1) * 2);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity((segs as usize + 1) * 2);
    let mut indices: Vec<u32> = Vec::with_capacity(segs as usize * 6);
    for i in 0..=segs {
        let t = i as f32 / segs as f32;
        let a = a0 + (a1 - a0) * t;
        let (s, c) = a.sin_cos();
        let dir = Vec2::new(c, s);
        let inner = dir * (radius - half_w);
        let outer = dir * (radius + half_w);
        positions.push([inner.x, inner.y, 0.0]);
        positions.push([outer.x, outer.y, 0.0]);
        uvs.push([t, 0.0]);
        uvs.push([t, 1.0]);
        normals.push([0.0, 0.0, 1.0]);
        normals.push([0.0, 0.0, 1.0]);
        if i > 0 {
            let b = (i - 1) * 2;
            indices.extend_from_slice(&[b, b + 2, b + 1, b + 1, b + 2, b + 3]);
        }
    }
    Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

fn draw_debug(mut gizmos: Gizmos, viewer: Res<ViewerWorld>, selection: Res<DebugSelection>) {
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
    gizmos.circle_2d(
        hold_px,
        params.hold_marker_radius * PPM,
        Color::srgb(1.0, 0.6, 0.1),
    );
    gizmos.line_2d(c, hold_px, Color::srgb(1.0, 0.6, 0.1));

    // Facing
    let tip = c + p.facing * (params.interact_radius * PPM + 8.0);
    gizmos.line_2d(c, tip, Color::srgb(0.2, 1.0, 1.0));
}
