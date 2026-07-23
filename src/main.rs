//! Interactive Bevy viewer (window). For batch AI eval use `soccer_headless`.
//!
//! ```text
//! cargo run --release
//! cargo run --release -- --fast --home chase --away idle
//! cargo run --release -- --both aia --fast --opening home
//! cargo run --release -- --home graph:C:\path\Team.txt --away chase
//! cargo run --release --bin soccer_headless -- --help
//! ```
//!
//! See README.md / AGENTS.md.
//! Hotkeys: Space=pause, F=fast (skip idle wait after tick lock), R=restart
//! (same opening side — not random).
//!
//! Timing model:
//! - **Tick lock** always: both brains must finish before physics / next tick.
//! - **Paced mode**: after a tick, idle-wait until FIXED_DT / timescale wall time.
//! - **Fast mode**: skip that idle wait; fire next tick as soon as tick lock clears.

use std::path::{Path, PathBuf};

use aicomp_soccer_sim::batch::BrainInput;
use aicomp_soccer_sim::brain::{BrainOutput, ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
use aicomp_soccer_sim::graph::load_team_graph;
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::keypress;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::player::PlayerId;
use aicomp_soccer_sim::probe_brains::{PerfectControllerBrain, Test1Brain, Test2Brain};
use aicomp_soccer_sim::scenario::MatchScenario;
use aicomp_soccer_sim::team_threads::{think_barrier, ThinkTimings};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};
use aicomp_soccer_sim::train::TrainedBrain;
use aicomp_soccer_sim::titanium::{
    apply_1v1_freeze, repark_1v1_inactive, setup_1v1_harness, TitaniumBrain,
};
use bevy::picking::prelude::*;
use bevy::prelude::*;
use bevy::ui::{FocusPolicy, RelativeCursorPosition};
use bevy::window::PresentMode;

const PPM: f32 = 10.0;
/// Fast / zero-idle: burst this many FIXED_DT ticks per render frame.
const MAX_TICKS_PER_FRAME_FAST: u32 = 64;
/// Paced catch-up when idle budget < one render frame (needed for >realtime).
const MAX_TICKS_PER_FRAME_PACED: u32 = 64;
/// Status / tick HUD text refresh rate (keep readable).
const UI_HZ: f32 = 10.0;
/// Left end of speed scrubber: 1 sim tick per wall-second.
const IDLE_BUDGET_MAX_S: f32 = 1.0;
/// Log floor just before far-right snap: 0.01 ms idle (matches fast tick cost).
const IDLE_BUDGET_LOG_FLOOR_S: f32 = 0.01 / 1000.0;

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

struct ViewerArgs {
    fast: bool,
    /// Scrubber t∈[0,1]: 0 = 1 tick/s, 1 = zero idle (as-fast-as-processed).
    speed_t: f32,
    home: BrainInput,
    away: BrainInput,
    /// Deterministic opening kickoff (default Home). R restart keeps this.
    opening: TeamId,
    /// Viewer scenario; Scenario 1 keeps the full roster parked and frozen.
    scenario: MatchScenario,
    /// Attacker is Home when true (default).
    titanium_1v1_attack_home: bool,
    /// Sideline bias for attacker spawn (±3).
    titanium_1v1_z_bias: f32,
}

fn print_viewer_help() {
    eprintln!(
        "\
soccer_sim — interactive AIComp Soccer viewer

USAGE:
  cargo run --release -- [OPTIONS]

OPTIONS:
  --home <brain>        Home / Team A brain (default: aia)
  --away <brain>        Away / Team B brain (default: aia)
  --both <brain>        Set home and away to the same brain
  --scenario 1          Scenario 1: full roster; inactive players parked far + frozen
  --scenario scenario1  Alias for --scenario 1
  --titanium-1v1        Compatibility alias for --scenario 1
  --atk-away            With Scenario 1, Away attacks (default Home)
  --z-bias <+|->        Attacker spawn Z bias (default +)
  --opening home|away   Opening kickoff side (default: home; R keeps it)
  --seed <u64>          Opening from seed: even=home, odd=away
  --fast, -f            Zero idle (same as scrubber all the way right)
  --speed <0..1>        Scrubber: 0=1 tick/s … 1=no idle (default ≈ realtime)
  -h, --help            Show this help

BRAINS:
  chase | idle | test1 | test2 | perfect (kb|keyboard) | aia | titanium | graph:<path>

SCENARIO 1:
  Default brains: attacker = keyboard (perfect), GK = titanium.
  Override with --home / --away (e.g. both titanium for AI vs AI).
  WASD move · Shift sprint · E interact/shoot · R reset bout

TIMING:
  Tick lock always waits for both brains.
  Speed scrubber sets idle budget after that (log): left slow, right no wait.
  Opening is fixed (not random) so Fast/R replays stay comparable.
  Omit --fast for realtime pacing (default).

HOTKEYS:
  Space  pause / resume
  F      toggle Fast (zero idle)
  R      restart match (same opening / re-arms 1v1 harness)
"
    );
}

fn parse_viewer_args() -> ViewerArgs {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut fast = false;
    let mut speed_t = idle_budget_to_fill(FIXED_DT);
    let mut home = BrainInput::Aia;
    let mut away = BrainInput::Aia;
    let mut brains_explicit = false;
    let mut opening = TeamId::Home;
    let mut scenario = MatchScenario::Full;
    let mut titanium_1v1_attack_home = true;
    let mut titanium_1v1_z_bias = 1.0f32;
    let mut i = 0usize;
    while i < argv.len() {
        match argv[i].as_str() {
            "--fast" | "-f" => fast = true,
            "--titanium-1v1" => scenario = MatchScenario::Scenario1AtkVsGk,
            "--scenario" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --scenario needs 1|scenario1|full");
                    print_viewer_help();
                    std::process::exit(1);
                });
                scenario = match v {
                    "1" | "scenario1" | "Scenario1" => MatchScenario::Scenario1AtkVsGk,
                    "full" | "Full" => MatchScenario::Full,
                    other => {
                        eprintln!("error: --scenario expected 1|scenario1|full, got {other}");
                        std::process::exit(1);
                    }
                };
            }
            "--atk-away" => {
                scenario = MatchScenario::Scenario1AtkVsGk;
                titanium_1v1_attack_home = false;
            }
            "--z-bias" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --z-bias needs + or -");
                    print_viewer_help();
                    std::process::exit(1);
                });
                titanium_1v1_z_bias = match v {
                    "+" | "+1" | "plus" | "1" => 1.0,
                    "-" | "-1" | "minus" => -1.0,
                    other => other.parse::<f32>().unwrap_or_else(|_| {
                        eprintln!("error: --z-bias expected +|- or a number");
                        std::process::exit(1);
                    }),
                };
            }
            "--speed" | "--timescale" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --speed needs a value 0..1");
                    print_viewer_help();
                    std::process::exit(1);
                });
                speed_t = v.parse::<f32>().unwrap_or_else(|_| {
                    eprintln!("error: --speed must be a number");
                    std::process::exit(1);
                });
                speed_t = speed_t.clamp(0.0, 1.0);
            }
            "--opening" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --opening needs home|away");
                    print_viewer_help();
                    std::process::exit(1);
                });
                opening = match v {
                    "home" | "Home" | "a" | "A" => TeamId::Home,
                    "away" | "Away" | "b" | "B" => TeamId::Away,
                    other => {
                        eprintln!("error: --opening expected home|away, got {other}");
                        std::process::exit(1);
                    }
                };
            }
            "--seed" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --seed needs a u64");
                    print_viewer_help();
                    std::process::exit(1);
                });
                let s: u64 = v.parse().unwrap_or_else(|_| {
                    eprintln!("error: --seed must be an integer");
                    std::process::exit(1);
                });
                opening = if s % 2 == 0 {
                    TeamId::Home
                } else {
                    TeamId::Away
                };
            }
            "--home" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --home needs a value");
                    print_viewer_help();
                    std::process::exit(1);
                });
                home = BrainInput::parse(v).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });
                brains_explicit = true;
            }
            "--away" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --away needs a value");
                    print_viewer_help();
                    std::process::exit(1);
                });
                away = BrainInput::parse(v).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });
                brains_explicit = true;
            }
            "--both" => {
                i += 1;
                let v = argv.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("error: --both needs a value");
                    print_viewer_help();
                    std::process::exit(1);
                });
                let brain = BrainInput::parse(v).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });
                home = brain.clone();
                away = brain;
                brains_explicit = true;
            }
            "-h" | "--help" => {
                print_viewer_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_viewer_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }
    if scenario.is_scenario1() {
        opening = if titanium_1v1_attack_home {
            TeamId::Home
        } else {
            TeamId::Away
        };
        // Default: you drive the attacker (P1), Titanium GKs. Explicit
        // --home/--away/--both keep AI-vs-AI / custom setups.
        if !brains_explicit {
            if titanium_1v1_attack_home {
                home = BrainInput::Perfect;
                away = BrainInput::Titanium;
            } else {
                home = BrainInput::Titanium;
                away = BrainInput::Perfect;
            }
        }
    }
    ViewerArgs {
        fast,
        speed_t,
        home,
        away,
        opening,
        scenario,
        titanium_1v1_attack_home,
        titanium_1v1_z_bias,
    }
}

fn main() {
    let args = parse_viewer_args();
    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_else(|e| {
        eprintln!("params load failed ({e}); using fallbacks");
        SimParams::default()
    });
    let (home_brain, home_path) = resolve_brain(&args.home);
    let (away_brain, away_path) = resolve_brain(&args.away);

    let asset_root = portable_asset_root();
    eprintln!(
        "viewer home={} away={} opening={:?} fast={} speed_t={:.3} idle≈{:.3}s scenario={}",
        args.home.label(),
        args.away.label(),
        args.opening,
        args.fast,
        args.speed_t,
        fill_to_idle_budget(args.speed_t),
        args.scenario.label()
    );
    if args.scenario.is_scenario1() {
        eprintln!(
            "Scenario 1: attack={} z_bias={:+} | attacker brain={} GK brain={}",
            if args.titanium_1v1_attack_home {
                "home"
            } else {
                "away"
            },
            args.titanium_1v1_z_bias,
            args.home.label(),
            args.away.label(),
        );
        eprintln!("Controls: WASD move · Shift sprint · E interact/shoot · R reset");
    }
    if args.fast || fill_to_idle_budget(args.speed_t) <= 0.0 {
        eprintln!("viewer zero-idle ON — tick advances as soon as both brains finish");
    }
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: if args.scenario.is_scenario1() {
                            "AIComp Soccer Sim — S1 Atk vs GK".into()
                        } else {
                            "AIComp Soccer Sim".into()
                        },
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
        .insert_resource(ViewerOpening(args.opening))
        .insert_resource(Titanium1v1Mode {
            scenario: args.scenario,
            attack_home: args.titanium_1v1_attack_home,
            z_bias: args.titanium_1v1_z_bias,
        })
        .insert_resource(Scenario1Round::default())
        .insert_resource({
            let mut world = MatchWorld::new_kickoff_opening(params, args.opening);
            // Viewer: no post-goal freeze (looks like lag on stream/demo).
            // Strip cosmetic GoalPause wait for playable viewer. Kickoff-phase
            // events (free ball, faceoff walk-in, hold, first kick) still run.
            world.params.kickoff_delay_s = 0.0;
            if args.scenario.is_scenario1() {
                setup_1v1_harness(
                    &mut world,
                    args.titanium_1v1_attack_home,
                    args.titanium_1v1_z_bias,
                );
            }
            ViewerWorld {
                world,
                home: home_brain,
                away: away_brain,
                last_home: BrainOutput::default(),
                last_away: BrainOutput::default(),
            }
        })
        .insert_resource(TeamScripts {
            home_path,
            away_path,
            status: if args.scenario.is_scenario1() {
                format!("{} titanium harness", args.scenario.label())
            } else {
                format!("cli home={} away={}", args.home.label(), args.away.label())
            },
        })
        .insert_resource(DebugSelection::default())
        .insert_resource(SimPaused(false))
        .insert_resource(SimFast(args.fast))
        .insert_resource(SimTimeScale(args.speed_t))
        .insert_resource(TimeScaleDragging(false))
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
                handle_timescale_slider,
                handle_player_click,
                sync_visuals,
                sync_stamina_arcs,
                sync_team_buttons,
                draw_debug,
                refresh_pause_ui,
                refresh_tick_hud,
                refresh_timescale_ui,
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

/// Built-in brain or compiled Graph VM (O1). Graph load failure → chase fallback.
enum ActiveBrain {
    Runtime(RuntimeBrain),
    Chase(ChaseBallBrain),
    Idle(IdleBrain),
    Test1(Test1Brain),
    Test2(Test2Brain),
    Perfect(PerfectControllerBrain),
    Titanium(TitaniumBrain),
    Trained(TrainedBrain),
}

impl ActiveBrain {
    fn label(&self) -> &'static str {
        match self {
            ActiveBrain::Runtime(_) => "runtime",
            ActiveBrain::Chase(_) => "chase",
            ActiveBrain::Idle(_) => "idle",
            ActiveBrain::Test1(_) => "test1",
            ActiveBrain::Test2(_) => "test2",
            ActiveBrain::Perfect(_) => "perfect",
            ActiveBrain::Titanium(_) => "titanium",
            ActiveBrain::Trained(_) => "trained",
        }
    }
}

impl TeamBrain for ActiveBrain {
    fn think(&mut self, api: &aicomp_soccer_sim::api::TeamApi) -> BrainOutput {
        match self {
            ActiveBrain::Runtime(g) => g.think(api),
            ActiveBrain::Chase(c) => c.think(api),
            ActiveBrain::Idle(b) => b.think(api),
            ActiveBrain::Test1(b) => b.think(api),
            ActiveBrain::Test2(b) => b.think(api),
            ActiveBrain::Perfect(b) => b.think(api),
            ActiveBrain::Titanium(b) => b.think(api),
            ActiveBrain::Trained(b) => b.think(api),
        }
    }
}

/// Resolve CLI / button brain → ActiveBrain + display path for the status strip.
fn resolve_brain(input: &BrainInput) -> (ActiveBrain, PathBuf) {
    match input {
        BrainInput::Chase => (ActiveBrain::Chase(ChaseBallBrain), PathBuf::from("chase")),
        BrainInput::Idle => (ActiveBrain::Idle(IdleBrain), PathBuf::from("idle")),
        BrainInput::Test1 => (
            ActiveBrain::Test1(Test1Brain::default()),
            PathBuf::from("test1"),
        ),
        BrainInput::Test2 => (
            ActiveBrain::Test2(Test2Brain::default()),
            PathBuf::from("test2"),
        ),
        BrainInput::Perfect => (
            ActiveBrain::Perfect(PerfectControllerBrain),
            PathBuf::from("perfect"),
        ),
        BrainInput::Aia => {
            let path = soccer_saves_dir().join("AIA.txt");
            (load_graph_brain(&path), path)
        }
        BrainInput::Graph(path) => (load_graph_brain(path), path.clone()),
        BrainInput::Titanium => (
            ActiveBrain::Titanium(TitaniumBrain::default()),
            PathBuf::from("titanium"),
        ),
        BrainInput::Trained => (
            ActiveBrain::Trained(TrainedBrain::default()),
            PathBuf::from("trained")
        )
    }
}

fn load_graph_brain(path: &Path) -> ActiveBrain {
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

/// Fast = skip **wall-clock idle** between ticks (process ASAP).
///
/// Invariant (do not break): Fast / speed scrubber must be unobservable from
/// inside the sim — same paradox as slowing real-world time 50% with no clock
/// that can see it. Always step with [`FIXED_DT`]; never scale `dt`, match
/// clocks, charge, stamina, or SoccerGet `Delta Time` / `Fixed Delta Time`.
/// Tick lock still waits for both brains before each step.
#[derive(Resource, Default)]
struct SimFast(bool);

/// Scrubber t∈[0,1]: left=1 tick/s wall idle, right=zero idle (process ASAP).
/// Fast button forces zero idle regardless of scrubber.
/// Same unobservability invariant as [`SimFast`] — wall pacing only.
#[derive(Resource)]
struct SimTimeScale(f32);

impl Default for SimTimeScale {
    fn default() -> Self {
        Self(idle_budget_to_fill(FIXED_DT))
    }
}

/// Log map: t=0 → 1s idle (1 tick/s), t→1 → 0.01ms, only the tip → 0 idle.
fn fill_to_idle_budget(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // Tiny tip at the far right = true no-idle (same as Fast).
    if t >= 0.997 {
        return 0.0;
    }
    // t∈[0,0.997] → 1s … 0.01ms on a log curve.
    let u = t / 0.997;
    IDLE_BUDGET_MAX_S * (IDLE_BUDGET_LOG_FLOOR_S / IDLE_BUDGET_MAX_S).powf(u)
}

fn idle_budget_to_fill(budget: f32) -> f32 {
    if budget <= 0.0 {
        return 1.0;
    }
    if budget <= IDLE_BUDGET_LOG_FLOOR_S {
        return 0.997;
    }
    if budget >= IDLE_BUDGET_MAX_S {
        return 0.0;
    }
    let u = (budget.ln() - IDLE_BUDGET_MAX_S.ln())
        / (IDLE_BUDGET_LOG_FLOOR_S.ln() - IDLE_BUDGET_MAX_S.ln());
    (u * 0.997).clamp(0.0, 0.997)
}

fn run_label(paused: bool, fast: bool) -> &'static str {
    if paused {
        "PAUSED"
    } else if fast {
        "FAST"
    } else {
        "RUNNING"
    }
}

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

/// World-space score digit(s) parked behind each goal (Home=left, Away=right).
#[derive(Component)]
struct GoalScoreText {
    side: TeamId,
}

#[derive(Resource)]
struct BallMat(Handle<ColorMaterial>);

#[derive(Component)]
enum UiAction {
    LoadHome,
    LoadAway,
    Restart,
    CycleScenario,
    TogglePause,
    ToggleFast,
}

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct PauseButtonText;

#[derive(Component)]
struct FastButtonText;

#[derive(Component)]
struct ScenarioButtonText;

#[derive(Component)]
struct TeamNameText {
    side: TeamId,
}

#[derive(Component)]
struct TimeScaleTrack;

#[derive(Component)]
struct TimeScaleFill;

#[derive(Component)]
struct TimeScaleThumb;

#[derive(Component)]
struct TimeScaleLabel;

/// True while LMB drag started on the timescale track.
#[derive(Resource, Default)]
struct TimeScaleDragging(bool);

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

        // Score sits just outside the back net (left goal = left team, right = right).
        let score_x = sign * (p.goal_line_x.abs() * PPM + net_depth + 22.0);
        let (side, color, initial) = if sign < 0.0 {
            (
                TeamId::Home,
                Color::srgb(0.75, 0.85, 1.0),
                viewer.world.match_state.score_home,
            )
        } else {
            (
                TeamId::Away,
                Color::srgb(1.0, 0.72, 0.68),
                viewer.world.match_state.score_away,
            )
        };
        commands.spawn((
            GoalScoreText { side },
            Text2d::new(format!("{initial}")),
            TextFont::from_font_size(48.0),
            TextColor(color),
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            Transform::from_xyz(score_x, 0.0, 4.0),
        ));
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

fn setup_ui(
    mut commands: Commands,
    scripts: Res<TeamScripts>,
    fast: Res<SimFast>,
    timescale: Res<SimTimeScale>,
    scenario: Res<Titanium1v1Mode>,
) {
    // Top bar: team pickers + transport (score lives behind the goals).
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(64.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.92)),
        ))
        .with_children(|parent| {
            team_slot_btn(
                parent,
                TeamId::Home,
                "Left",
                &file_stem(&scripts.home_path),
                UiAction::LoadHome,
                Color::srgb(0.18, 0.32, 0.55),
            );
            team_slot_btn(
                parent,
                TeamId::Away,
                "Right",
                &file_stem(&scripts.away_path),
                UiAction::LoadAway,
                Color::srgb(0.55, 0.22, 0.18),
            );

            transport_btn(parent, UiAction::Restart, "Restart\n(R)");
            scenario_btn(parent, scenario.scenario);
            parent
                .spawn((
                    Button,
                    UiAction::TogglePause,
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        min_width: Val::Px(88.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.22, 0.22, 0.30)),
                ))
                .with_children(|b| {
                    b.spawn((
                        PauseButtonText,
                        Text::new("Pause\n(Space)"),
                        TextFont::from_font_size(13.0),
                        TextColor(Color::WHITE),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    ));
                });
            parent
                .spawn((
                    Button,
                    UiAction::ToggleFast,
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        min_width: Val::Px(88.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(if fast.0 {
                        Color::srgb(0.55, 0.40, 0.12)
                    } else {
                        Color::srgb(0.22, 0.22, 0.30)
                    }),
                ))
                .with_children(|b| {
                    b.spawn((
                        FastButtonText,
                        Text::new(if fast.0 {
                            "Fast ON\n(F)"
                        } else {
                            "Fast OFF\n(F)"
                        }),
                        TextFont::from_font_size(13.0),
                        TextColor(Color::WHITE),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    ));
                });

            parent.spawn((
                StatusText,
                Text::new(format!("[{}]", run_label(false, fast.0))),
                TextFont::from_font_size(18.0),
                TextColor(Color::srgb(0.95, 0.95, 0.95)),
                Node {
                    margin: UiRect::left(Val::Px(8.0)),
                    ..default()
                },
            ));
        });

    // Bottom-left: timescale scrubber (click / drag).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(12.0),
                width: Val::Px(280.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.04, 0.88)),
        ))
        .with_children(|p| {
            p.spawn((
                TimeScaleLabel,
                Text::new(speed_label(timescale.0, false)),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.9, 0.93, 0.98)),
            ));
            // Tall hit target — click anywhere / hold-drag to scrub.
            p.spawn((
                Button,
                TimeScaleTrack,
                RelativeCursorPosition::default(),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(28.0),
                    justify_content: JustifyContent::FlexStart,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.12, 0.13, 0.18)),
            ))
            .with_children(|track| {
                let fill = timescale.0.clamp(0.0, 1.0);
                track.spawn((
                    TimeScaleFill,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(fill * 100.0),
                        height: Val::Percent(100.0),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.30, 0.65, 0.95)),
                    FocusPolicy::Pass,
                ));
                track.spawn((
                    TimeScaleThumb,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent((fill * 100.0).clamp(0.0, 100.0)),
                        margin: UiRect::left(Val::Px(-8.0)),
                        width: Val::Px(16.0),
                        height: Val::Px(16.0),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.95, 0.97, 1.0)),
                    FocusPolicy::Pass,
                ));
            });
            p.spawn((
                Text::new("1 tick/s  ←  drag  →  0.01ms  ·  no wait"),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.6, 0.65, 0.7)),
            ));
        });

    // Top-right brain timing (no render/UI spam).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(76.0),
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

fn speed_label(t: f32, fast: bool) -> String {
    if fast || fill_to_idle_budget(t) <= 0.0 {
        "Speed  no wait".into()
    } else {
        let idle = fill_to_idle_budget(t);
        let idle_ms = idle * 1000.0;
        if idle >= 0.95 {
            "Speed  1 tick/s".into()
        } else if (idle - FIXED_DT).abs() < FIXED_DT * 0.15 {
            format!("Speed  realtime (~{:.2}ms)", FIXED_DT * 1000.0)
        } else {
            format!("Speed  {:.2}ms sim tick", idle_ms)
        }
    }
}

fn timescale_to_fill(ts: f32) -> f32 {
    // SimTimeScale now stores scrubber t directly.
    ts.clamp(0.0, 1.0)
}

fn fill_to_timescale(fill: f32) -> f32 {
    fill.clamp(0.0, 1.0)
}

fn team_slot_btn(
    parent: &mut ChildSpawnerCommands,
    side: TeamId,
    side_label: &str,
    team_name: &str,
    action: UiAction,
    accent: Color,
) {
    parent
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                min_width: Val::Px(140.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(1.0),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(accent),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(side_label),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
            ));
            b.spawn((
                TeamNameText { side },
                Text::new(team_name.to_string()),
                TextFont::from_font_size(16.0),
                TextColor(Color::WHITE),
            ));
            b.spawn((
                Text::new("(click to change)"),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
            ));
        });
}

fn transport_btn(parent: &mut ChildSpawnerCommands, action: UiAction, label: &str) {
    parent
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                min_width: Val::Px(80.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.22, 0.22, 0.30)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont::from_font_size(13.0),
                TextColor(Color::WHITE),
                TextLayout::new(Justify::Center, LineBreak::NoWrap),
            ));
        });
}

fn scenario_btn(parent: &mut ChildSpawnerCommands, scenario: MatchScenario) {
    parent
        .spawn((
            Button,
            UiAction::CycleScenario,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                min_width: Val::Px(120.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.22, 0.22, 0.30)),
        ))
        .with_children(|b| {
            b.spawn((
                ScenarioButtonText,
                Text::new(scenario.label()),
                TextFont::from_font_size(13.0),
                TextColor(Color::WHITE),
                TextLayout::new(Justify::Center, LineBreak::NoWrap),
            ));
        });
}

fn file_stem(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

/// One locked tick: both brains finish, then physics.
/// Always advances sim by [`FIXED_DT`] — Fast/scrubber only change how often
/// this is called in wall time, never the dt passed into physics/brains.
/// Returns wall ms for the whole tick (HUD only).
fn step_locked_tick(
    viewer: &mut ViewerWorld,
    clock: &mut TickClock,
    interp: &mut InterpState,
    one_v_one: Titanium1v1Mode,
) -> f32 {
    let tick0 = std::time::Instant::now();
    let ViewerWorld {
        world,
        home,
        away,
        last_home,
        last_away,
    } = viewer;
    let (home_api, away_api) = world.build_apis();
    // Tick lock: both brains must finish before physics.
    let (mut home_out, mut away_out, timings) = think_barrier(home, away, home_api, away_api);
    if one_v_one.enabled() {
        apply_1v1_freeze(&mut home_out, &mut away_out, world, one_v_one.attack_home);
    }
    *last_home = home_out.clone();
    *last_away = away_out.clone();

    let phys0 = std::time::Instant::now();
    world.step_with_commands(&home_out, &away_out, FIXED_DT);
    if one_v_one.enabled() {
        repark_1v1_inactive(world, one_v_one.attack_home);
    }
    clock.physics_ms = phys0.elapsed().as_secs_f32() * 1000.0;
    interp.push_tick(world);
    clock.last = timings;
    tick0.elapsed().as_secs_f32() * 1000.0
}

fn sync_keypress_from_bevy(keyboard: &ButtonInput<KeyCode>) {
    // Space stays a viewer hotkey only (pause) — never feed it to Keypress nodes.
    let pressed: Vec<&'static str> = keyboard
        .get_pressed()
        .filter_map(|code| keypress::bevy_key_name(*code))
        .filter(|name| *name != "Space")
        .collect();
    keypress::set_pressed(pressed);
}

fn sim_tick_barrier(
    time: Res<Time>,
    paused: Res<SimPaused>,
    fast: Res<SimFast>,
    timescale: Res<SimTimeScale>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut viewer: ResMut<ViewerWorld>,
    mut clock: ResMut<TickClock>,
    mut interp: ResMut<InterpState>,
    one_v_one: Res<Titanium1v1Mode>,
    mut round: ResMut<Scenario1Round>,
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

    // Feed Unity Keypress nodes before brains think (WASD / sprint / interact).
    sync_keypress_from_bevy(&keyboard);

    clock.ticks_this_frame = 0;

    let idle_budget = if fast.0 {
        0.0
    } else {
        fill_to_idle_budget(timescale.0)
    };

    // Zero idle (Fast button or scrubber tip): fire ASAP after tick lock.
    if idle_budget <= 0.0 {
        let mut last_ms = 0.0f32;
        for _ in 0..MAX_TICKS_PER_FRAME_FAST {
            last_ms = step_locked_tick(&mut viewer, &mut clock, &mut interp, *one_v_one);
            clock.ticks_this_frame += 1;
            apply_scenario1_scoring(&mut viewer.world, *one_v_one, &mut round);
        }
        clock.tick_ms = last_ms;
        clock.accumulator = 0.0;
        clock.backlog_ticks = 0.0;
        clock.alpha = 1.0;
        return;
    }

    // Paced: each tick costs `idle_budget` wall seconds from the accumulator.
    // If budget < one render frame, run several ticks this frame (proportional speed).
    // Still tick-locked; never faster than brains+physics can compute.
    clock.accumulator += time.delta_secs();
    let mut last_ms = 0.0f32;
    let mut ticks = 0u32;
    while clock.accumulator >= idle_budget && ticks < MAX_TICKS_PER_FRAME_PACED {
        clock.accumulator -= idle_budget;
        last_ms = step_locked_tick(&mut viewer, &mut clock, &mut interp, *one_v_one);
        ticks += 1;
        apply_scenario1_scoring(&mut viewer.world, *one_v_one, &mut round);
    }
    clock.ticks_this_frame = ticks;
    clock.tick_ms = last_ms;
    clock.backlog_ticks = clock.accumulator / idle_budget;
    clock.alpha = (clock.accumulator / idle_budget).clamp(0.0, 1.0);
}

/// Scenario 1 continuous scoring (viewer): never ends the match.
///
/// - Attacker goal: physics already increments score → reset bout layout
/// - GK P4 holds/tackles ball → award a goal to the GK side, then reset bout
fn apply_scenario1_scoring(
    world: &mut MatchWorld,
    one_v_one: Titanium1v1Mode,
    round: &mut Scenario1Round,
) {
    if !one_v_one.enabled() {
        return;
    }
    let attack_home = one_v_one.attack_home;
    let gk_team = if attack_home {
        TeamId::Away
    } else {
        TeamId::Home
    };

    let sh = world.match_state.score_home;
    let sa = world.match_state.score_away;
    let physics_goal = sh > round.start_home || sa > round.start_away;
    let gk_hold = world.possession.carrier == Some((gk_team, 4)) && world.ball.held;

    if !physics_goal && !gk_hold {
        return;
    }

    let mut keep_h = sh;
    let mut keep_a = sa;
    if gk_hold && !physics_goal {
        // Goal for the goalkeeper's team (attacker is scored on).
        if attack_home {
            keep_a += 1;
        } else {
            keep_h += 1;
        }
        info!(
            home = keep_h,
            away = keep_a,
            "scenario 1: GK hold → GK team goal"
        );
    } else {
        info!(
            home = keep_h,
            away = keep_a,
            "scenario 1: bout reset after goal"
        );
    }

    setup_1v1_harness(world, attack_home, one_v_one.z_bias);
    world.match_state.score_home = keep_h;
    world.match_state.score_away = keep_a;
    *round = Scenario1Round {
        start_home: keep_h,
        start_away: keep_a,
    };
}

fn handle_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut viewer: ResMut<ViewerWorld>,
    mut selection: ResMut<DebugSelection>,
    mut paused: ResMut<SimPaused>,
    mut fast: ResMut<SimFast>,
    mut interp: ResMut<InterpState>,
    mut clock: ResMut<TickClock>,
    opening: Res<ViewerOpening>,
    one_v_one: Res<Titanium1v1Mode>,
    mut round: ResMut<Scenario1Round>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        restart_match(
            &mut viewer,
            &mut interp,
            &mut clock,
            opening.0,
            *one_v_one,
            &mut round,
        );
        selection.selected = None;
        paused.0 = false;
    }
    if keys.just_pressed(KeyCode::Space) {
        toggle_pause(&mut paused);
    }
    if keys.just_pressed(KeyCode::KeyF) {
        toggle_fast(&mut fast, &mut clock);
    }
}

fn toggle_pause(paused: &mut SimPaused) {
    paused.0 = !paused.0;
    info!(paused = paused.0, "simulation pause toggled");
}

fn toggle_fast(fast: &mut SimFast, clock: &mut TickClock) {
    fast.0 = !fast.0;
    clock.accumulator = 0.0;
    clock.alpha = 1.0;
    info!(
        fast = fast.0,
        "fast mode toggled (idle-wait skip; tick lock unchanged)"
    );
}

fn format_status(paused: bool, fast: bool) -> String {
    format!("[{}]", run_label(paused, fast))
}

fn refresh_pause_ui(
    paused: Res<SimPaused>,
    fast: Res<SimFast>,
    mut pause_label: Query<&mut Text, With<PauseButtonText>>,
    mut fast_label: Query<&mut Text, (With<FastButtonText>, Without<PauseButtonText>)>,
    mut status_q: Query<
        &mut Text,
        (
            With<StatusText>,
            Without<PauseButtonText>,
            Without<FastButtonText>,
            Without<ScenarioButtonText>,
        ),
    >,
) {
    if !paused.is_changed() && !fast.is_changed() {
        return;
    }
    if paused.is_changed() {
        if let Ok(mut text) = pause_label.single_mut() {
            *text = Text::new(if paused.0 {
                "Resume\n(Space)"
            } else {
                "Pause\n(Space)"
            });
        }
    }
    if fast.is_changed() {
        if let Ok(mut text) = fast_label.single_mut() {
            *text = Text::new(if fast.0 {
                "Fast ON\n(F)"
            } else {
                "Fast OFF\n(F)"
            });
        }
    }
    if let Ok(mut text) = status_q.single_mut() {
        *text = Text::new(format_status(paused.0, fast.0));
    }
}

/// Fixed opening kickoff for this viewer session (R restart keeps it).
#[derive(Resource, Clone, Copy)]
struct ViewerOpening(TeamId);

/// Scripted Titanium P1-vs-P4 harness (viewer only).
#[derive(Resource, Clone, Copy)]
struct Titanium1v1Mode {
    scenario: MatchScenario,
    attack_home: bool,
    z_bias: f32,
}

impl Titanium1v1Mode {
    fn enabled(self) -> bool {
        self.scenario.is_scenario1()
    }
}

/// Scenario 1 bout bookkeeping (viewer): track scores so we know when physics
/// already awarded an attacker goal and we should reset the layout.
#[derive(Resource, Default, Clone, Copy)]
struct Scenario1Round {
    start_home: u32,
    start_away: u32,
}

impl Scenario1Round {
    fn arm(world: &MatchWorld) -> Self {
        Self {
            start_home: world.match_state.score_home,
            start_away: world.match_state.score_away,
        }
    }
}

fn restart_match(
    viewer: &mut ViewerWorld,
    interp: &mut InterpState,
    clock: &mut TickClock,
    opening: TeamId,
    one_v_one: Titanium1v1Mode,
    round: &mut Scenario1Round,
) {
    let mut params = viewer.world.params.clone();
    params.kickoff_delay_s = 0.0;
    viewer.world = MatchWorld::new_kickoff_opening(params, opening);
    if one_v_one.enabled() {
        setup_1v1_harness(
            &mut viewer.world,
            one_v_one.attack_home,
            one_v_one.z_bias,
        );
        *round = Scenario1Round::arm(&viewer.world);
    } else {
        *round = Scenario1Round::default();
    }
    viewer.last_home = BrainOutput::default();
    viewer.last_away = BrainOutput::default();
    interp.reset_from(&viewer.world);
    clock.accumulator = 0.0;
    clock.alpha = 1.0;
    info!(
        ?opening,
        scenario = one_v_one.scenario.label(),
        "match restarted"
    );
}

fn lighten(c: Color, amount: f32) -> Color {
    let s = c.to_srgba();
    Color::srgb(
        (s.red + amount).min(1.0),
        (s.green + amount).min(1.0),
        (s.blue + amount).min(1.0),
    )
}

fn handle_ui_buttons(
    mut interactions: Query<
        (&Interaction, &UiAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut viewer: ResMut<ViewerWorld>,
    mut scripts: ResMut<TeamScripts>,
    mut status_q: Query<&mut Text, (With<StatusText>, Without<ScenarioButtonText>)>,
    mut selection: ResMut<DebugSelection>,
    mut paused: ResMut<SimPaused>,
    mut fast: ResMut<SimFast>,
    mut interp: ResMut<InterpState>,
    mut clock: ResMut<TickClock>,
    opening: Res<ViewerOpening>,
    mut one_v_one: ResMut<Titanium1v1Mode>,
    mut round: ResMut<Scenario1Round>,
    mut scenario_q: Query<&mut Text, (With<ScenarioButtonText>, Without<StatusText>)>,
) {
    for (interaction, action, mut bg) in &mut interactions {
        let base = match action {
            UiAction::LoadHome => Color::srgb(0.18, 0.32, 0.55),
            UiAction::LoadAway => Color::srgb(0.55, 0.22, 0.18),
            UiAction::ToggleFast if fast.0 => Color::srgb(0.55, 0.40, 0.12),
            _ => Color::srgb(0.22, 0.22, 0.30),
        };
        match *interaction {
            Interaction::Hovered => *bg = BackgroundColor(lighten(base, 0.12)),
            Interaction::None => *bg = BackgroundColor(base),
            Interaction::Pressed => {
                *bg = BackgroundColor(lighten(base, 0.20));
                match action {
                    UiAction::CycleScenario => {
                        one_v_one.scenario = one_v_one.scenario.cycle();
                        let mode = *one_v_one;
                        if mode.enabled() {
                            // Attacker = keyboard P1, GK = Titanium.
                            if mode.attack_home {
                                viewer.home =
                                    ActiveBrain::Perfect(PerfectControllerBrain);
                                viewer.away =
                                    ActiveBrain::Titanium(TitaniumBrain::new(false));
                                scripts.home_path = PathBuf::from("keyboard");
                                scripts.away_path = PathBuf::from("titanium");
                            } else {
                                viewer.home =
                                    ActiveBrain::Titanium(TitaniumBrain::new(false));
                                viewer.away =
                                    ActiveBrain::Perfect(PerfectControllerBrain);
                                scripts.home_path = PathBuf::from("titanium");
                                scripts.away_path = PathBuf::from("keyboard");
                            }
                            scripts.status = format!(
                                "{} — WASD/Shift/E attack, Titanium GK",
                                mode.scenario.label()
                            );
                        } else {
                            scripts.status = format!("{} scenario", mode.scenario.label());
                        }
                        restart_match(
                            &mut viewer,
                            &mut interp,
                            &mut clock,
                            opening.0,
                            mode,
                            &mut round,
                        );
                        paused.0 = false;
                        if let Ok(mut text) = scenario_q.single_mut() {
                            *text = Text::new(mode.scenario.label());
                        }
                        selection.selected = None;
                    }
                    UiAction::Restart => {
                        restart_match(
                            &mut viewer,
                            &mut interp,
                            &mut clock,
                            opening.0,
                            *one_v_one,
                            &mut round,
                        );
                        paused.0 = false;
                        selection.selected = None;
                    }
                    UiAction::TogglePause => toggle_pause(&mut paused),
                    UiAction::ToggleFast => toggle_fast(&mut fast, &mut clock),
                    UiAction::LoadHome => {
                        if let Some(path) = pick_team_script("Load Left team") {
                            viewer.home = load_graph_brain(&path);
                            scripts.home_path = path;
                            scripts.status = format!("Left {}", file_stem(&scripts.home_path));
                        }
                    }
                    UiAction::LoadAway => {
                        if let Some(path) = pick_team_script("Load Right team") {
                            viewer.away = load_graph_brain(&path);
                            scripts.away_path = path;
                            scripts.status = format!("Right {}", file_stem(&scripts.away_path));
                        }
                    }
                }
                if let Ok(mut text) = status_q.single_mut() {
                    *text = Text::new(format_status(paused.0, fast.0));
                }
            }
        }
    }
}

fn handle_timescale_slider(
    mouse: Res<ButtonInput<MouseButton>>,
    track_q: Query<(&RelativeCursorPosition, &Interaction), With<TimeScaleTrack>>,
    mut dragging: ResMut<TimeScaleDragging>,
    mut timescale: ResMut<SimTimeScale>,
) {
    let Ok((rel, interaction)) = track_q.single() else {
        return;
    };

    // Start scrub when pressing on the track (Button Interaction::Pressed).
    if mouse.just_pressed(MouseButton::Left)
        && (*interaction == Interaction::Pressed || rel.cursor_over())
    {
        dragging.0 = true;
    }
    if mouse.just_released(MouseButton::Left) {
        dragging.0 = false;
    }
    if !dragging.0 {
        return;
    }

    // Bevy UI normalized: (-0.5,-0.5)=top-left … (+0.5,+0.5)=bottom-right.
    // Keep updating while held even if the cursor leaves the track vertically.
    let Some(pos) = rel.normalized else {
        return;
    };
    let fill = (pos.x + 0.5).clamp(0.0, 1.0);
    let next = fill_to_timescale(fill);
    if (next - timescale.0).abs() > 1e-4 {
        timescale.0 = next;
    }
}

fn refresh_timescale_ui(
    timescale: Res<SimTimeScale>,
    fast: Res<SimFast>,
    mut label: Query<&mut Text, With<TimeScaleLabel>>,
    mut fill: Query<&mut Node, With<TimeScaleFill>>,
    mut thumb: Query<&mut Node, (With<TimeScaleThumb>, Without<TimeScaleFill>)>,
) {
    if !timescale.is_changed() && !fast.is_changed() {
        return;
    }
    let t = timescale_to_fill(timescale.0);
    if let Ok(mut text) = label.single_mut() {
        *text = Text::new(speed_label(timescale.0, fast.0));
    }
    if let Ok(mut node) = fill.single_mut() {
        node.width = Val::Percent(t * 100.0);
    }
    if let Ok(mut node) = thumb.single_mut() {
        node.left = Val::Percent((t * 100.0).clamp(0.0, 100.0));
    }
}

fn sync_team_buttons(scripts: Res<TeamScripts>, mut names: Query<(&TeamNameText, &mut Text)>) {
    if !scripts.is_changed() {
        return;
    }
    for (slot, mut text) in &mut names {
        let name = match slot.side {
            TeamId::Home => file_stem(&scripts.home_path),
            TeamId::Away => file_stem(&scripts.away_path),
        };
        *text = Text::new(name);
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
    mut q_score: Query<(&GoalScoreText, &mut Text2d)>,
    paused: Res<SimPaused>,
    fast: Res<SimFast>,
    mut status_q: Query<&mut Text, With<StatusText>>,
) {
    let w = &viewer.world;
    let alpha = if paused.0 || fast.0 { 1.0 } else { clock.alpha };
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
            *text = Text::new(format_status(paused.0, fast.0));
        }
        for (marker, mut text) in &mut q_score {
            let n = match marker.side {
                TeamId::Home => w.match_state.score_home,
                TeamId::Away => w.match_state.score_away,
            };
            *text = Text2d::new(format!("{n}"));
        }
    }
}

fn refresh_tick_hud(
    clock: Res<TickClock>,
    pulse: Res<UiPulse>,
    fast: Res<SimFast>,
    timescale: Res<SimTimeScale>,
    mut hud: Query<(&mut Text, &mut TextColor), With<TickHudText>>,
) {
    if !pulse.fire {
        return;
    }
    let Ok((mut text, mut color)) = hud.single_mut() else {
        return;
    };
    let idle = if fast.0 {
        0.0
    } else {
        fill_to_idle_budget(timescale.0)
    };
    let left_ms = clock.last.home_ms();
    let right_ms = clock.last.away_ms();
    let pace = if idle <= 0.0 {
        "no wait".to_string()
    } else {
        format!("{:.2}ms sim tick", idle * 1000.0)
    };
    *text = Text::new(format!(
        "{:.2}ms   L {:.2} · R {:.2}\n{pace}",
        clock.tick_ms, left_ms, right_ms
    ));
    *color = TextColor(if idle <= 0.0 {
        Color::srgb(1.0, 0.85, 0.35)
    } else if clock.tick_ms > idle * 1000.0 * 1.05 {
        Color::srgb(1.0, 0.4, 0.35)
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

fn draw_debug(
    mut gizmos: Gizmos,
    viewer: Res<ViewerWorld>,
    selection: Res<DebugSelection>,
    one_v_one: Res<Titanium1v1Mode>,
) {
    let w = &viewer.world;
    let params = &w.params;

    if one_v_one.enabled() {
        draw_scenario1_cone(&mut gizmos, w, one_v_one.attack_home);
    }

    let Some((team, id)) = selection.selected else {
        return;
    };
    let Some(p) = w.players.iter().find(|p| p.team == team && p.id == id) else {
        return;
    };
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

/// Scenario 1 GK geometry: loose-ball path/stop always; purple when GK is first.
fn draw_scenario1_cone(gizmos: &mut Gizmos, w: &MatchWorld, attack_home: bool) {
    use aicomp_soccer_sim::predict::{
        gk_intercept_cover, predict_ball_path, predict_ball_path_until_intercept, Candidate,
    };

    let params = &w.params;
    let (atk_team, gk_team) = if attack_home {
        (TeamId::Home, TeamId::Away)
    } else {
        (TeamId::Away, TeamId::Home)
    };
    let Some(atk) = w
        .players
        .iter()
        .find(|p| p.team == atk_team && p.id == PlayerId(1))
    else {
        return;
    };
    let Some(gk) = w
        .players
        .iter()
        .find(|p| p.team == gk_team && p.id == PlayerId(4))
    else {
        return;
    };

    let own_goal_x = if attack_home {
        params.goal_line_x.abs()
    } else {
        -params.goal_line_x.abs()
    };
    let half = params.goal_half_width;
    let reach = params.interact_radius;
    let left = Vec2::new(own_goal_x, half);
    let right = Vec2::new(own_goal_x, -half);

    let l_px = Vec2::new(left.x * PPM, left.y * PPM);
    let r_px = Vec2::new(right.x * PPM, right.y * PPM);
    let gk_px = Vec2::new(gk.pos.x * PPM, gk.pos.y * PPM);
    let reach_px = reach * PPM;

    gizmos.circle_2d(gk_px, reach_px, Color::srgb(0.35, 0.75, 1.0));

    if !w.ball.held {
        let ball_state = aicomp_soccer_sim::ball::Ball {
            pos: w.ball.pos,
            vel: w.ball.vel,
            height: w.ball.height,
            vel_y: w.ball.vel_y,
            held: false,
        };
        // Full path → resting stop (always visible while loose).
        let full = predict_ball_path(&ball_state, params, FIXED_DT, 4.0);
        let traj = Color::srgba(0.85, 0.9, 1.0, 0.75);
        for w_pair in full.windows(2) {
            let a = Vec2::new(w_pair[0].pos.x * PPM, w_pair[0].pos.y * PPM);
            let b = Vec2::new(w_pair[1].pos.x * PPM, w_pair[1].pos.y * PPM);
            gizmos.line_2d(a, b, traj);
        }
        if let Some(stop) = full.last() {
            let s_px = Vec2::new(stop.pos.x * PPM, stop.pos.y * PPM);
            gizmos.circle_2d(s_px, 6.0, Color::srgb(1.0, 1.0, 0.35));
            gizmos.circle_2d(s_px, 3.0, Color::srgb(1.0, 0.95, 0.2));
        }

        // First touch among all on-pitch players (sprint). Purple iff GK wins.
        const SPRINT: f32 = 8.0;
        let mut cands: Vec<Candidate> = Vec::with_capacity(8);
        let mut gk_idx = None;
        for p in &w.players {
            if p.pos.length() > 80.0 {
                continue; // parked Scenario-1 extras
            }
            if p.team == gk_team && p.id == PlayerId(4) {
                gk_idx = Some(cands.len());
            }
            cands.push(Candidate {
                pos: p.pos,
                speed: SPRINT,
            });
        }
        let (_cut, first) = predict_ball_path_until_intercept(
            &ball_state,
            params,
            FIXED_DT,
            4.0,
            &cands,
            reach,
        );
        if let Some((ci, hit)) = first {
            let hit_px = Vec2::new(hit.pos.x * PPM, hit.pos.y * PPM);
            let gk_first = gk_idx == Some(ci);
            let col = if gk_first {
                Color::srgb(0.72, 0.25, 1.0) // purple — guaranteed GK
            } else {
                Color::srgb(1.0, 0.55, 0.2)
            };
            let line = if gk_first {
                Color::srgba(0.72, 0.25, 1.0, 0.9)
            } else {
                Color::srgba(1.0, 0.55, 0.2, 0.9)
            };
            gizmos.line_2d(gk_px, hit_px, line);
            gizmos.circle_2d(hit_px, 5.5, col);
            if gk_first {
                // Highlight path from ball to GK touch in purple.
                let mut prev = Vec2::new(w.ball.pos.x * PPM, w.ball.pos.y * PPM);
                for s in full.iter().skip(1) {
                    if s.t > hit.t + 1e-6 {
                        break;
                    }
                    let cur = Vec2::new(s.pos.x * PPM, s.pos.y * PPM);
                    gizmos.line_2d(prev, cur, Color::srgba(0.72, 0.25, 1.0, 0.95));
                    prev = cur;
                }
            }
        }

        let b_px = Vec2::new(w.ball.pos.x * PPM, w.ball.pos.y * PPM);
        gizmos.line_2d(b_px, l_px, Color::srgba(1.0, 0.85, 0.15, 0.45));
        gizmos.line_2d(b_px, r_px, Color::srgba(1.0, 0.85, 0.15, 0.45));
        return;
    }

    let threat = atk.pos;
    let ideal = gk_intercept_cover(threat, own_goal_x, half, params, 8.0);
    let (press, _) = aicomp_soccer_sim::predict::gk_cover_press_target(
        gk.pos,
        threat,
        if w.ball.held { w.ball.vel } else { Vec2::ZERO },
        own_goal_x,
        half,
        params,
        8.0,
    );
    let t_px = Vec2::new(threat.x * PPM, threat.y * PPM);
    let ideal_px = Vec2::new(ideal.x * PPM, ideal.y * PPM);
    let press_px = Vec2::new(press.x * PPM, press.y * PPM);

    let cone = Color::srgba(1.0, 0.85, 0.15, 0.95);
    gizmos.line_2d(t_px, l_px, cone);
    gizmos.line_2d(t_px, r_px, cone);
    gizmos.line_2d(l_px, r_px, Color::srgba(1.0, 0.4, 0.2, 0.8));

    gizmos.circle_2d(ideal_px, 4.0, Color::srgb(0.3, 1.0, 0.45));
    gizmos.line_2d(t_px, ideal_px, Color::srgba(0.3, 1.0, 0.45, 0.55));
    gizmos.circle_2d(ideal_px, reach_px, Color::srgba(0.3, 1.0, 0.45, 0.35));
    let line_half = half * PPM;
    gizmos.line_2d(
        Vec2::new(ideal_px.x, line_half),
        Vec2::new(ideal_px.x, -line_half),
        Color::srgba(0.2, 0.95, 1.0, 0.75),
    );
    // Cover stand (= press target under rule 4).
    gizmos.circle_2d(press_px, 5.0, Color::srgb(1.0, 0.45, 0.15));
    gizmos.line_2d(gk_px, press_px, Color::srgba(1.0, 0.45, 0.15, 0.85));
}
