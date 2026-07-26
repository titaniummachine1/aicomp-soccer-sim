//! Log anti-tackle TimePlot channels + possession steals for forensics.
//!
//! cargo run --release --bin at_tackle_log -- \
//!   --home graph:../titanim-socker-engine/out/Titanium_challenger.txt \
//!   --away graph:data/titanium/Haialand-v2.txt --secs 180

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use aicomp_soccer_sim::batch::{BrainInput, ProgramCache};
use aicomp_soccer_sim::brain::{TeamBrain, TeamId};
use aicomp_soccer_sim::debug_draw;
use aicomp_soccer_sim::graph_vm::RuntimeBrain;
use aicomp_soccer_sim::params::{default_params_path, SimParams};
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn build(input: &BrainInput, cache: &ProgramCache) -> Result<Box<dyn TeamBrain>, String> {
    match input {
        BrainInput::Graph(p) => {
            let g = cache
                .get_or_compile(p)
                .map_err(|e| format!("load {p:?}: {e}"))?;
            Ok(Box::new(RuntimeBrain::from_cached(g)))
        }
        other => Err(format!("at_tackle_log needs graph:<path>, got {other:?}")),
    }
}

fn plot_val(frame: &debug_draw::DebugDrawFrame, name: &str) -> Option<f32> {
    frame
        .plots
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| *v)
}

fn fail_label(v: f32) -> &'static str {
    match v as i32 {
        0 => "ok",
        1 => "no_safe_angle",
        2 => "miscalculation",
        _ => "unknown",
    }
}

fn is_titanium_path(path: &PathBuf) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .contains("titanium")
}

fn main() -> ExitCode {
    let mut home_in = BrainInput::Graph(PathBuf::from(
        "../titanim-socker-engine/out/Titanium_challenger.txt",
    ));
    let mut away_in = BrainInput::Graph(PathBuf::from("data/titanium/Haialand-v2.txt"));
    let mut secs = 180.0f32;
    let mut opening = TeamId::Home;
    let mut argv = env::args().skip(1);
    while let Some(a) = argv.next() {
        match a.as_str() {
            "--home" => home_in = BrainInput::parse(&argv.next().expect("--home")).unwrap(),
            "--away" => away_in = BrainInput::parse(&argv.next().expect("--away")).unwrap(),
            "--secs" => secs = argv.next().unwrap().parse().unwrap(),
            "--opening" => {
                opening = match argv.next().expect("--opening").as_str() {
                    "home" => TeamId::Home,
                    "away" => TeamId::Away,
                    _ => TeamId::Home,
                }
            }
            _ => {}
        }
    }

    let titanium_side = match (&home_in, &away_in) {
        (BrainInput::Graph(h), _) if is_titanium_path(h) => TeamId::Home,
        (_, BrainInput::Graph(a)) if is_titanium_path(a) => TeamId::Away,
        _ => TeamId::Home,
    };

    let params = SimParams::load_from_disk(&default_params_path()).unwrap_or_default();
    let mut world = MatchWorld::new_kickoff_opening(params, opening);
    let cache = ProgramCache::new();
    let mut home = build(&home_in, &cache).expect("home brain");
    let mut away = build(&away_in, &cache).expect("away brain");

    let mut prev_carrier = world.possession.carrier;
    let mut steals = 0u32;
    let mut miscalc = 0u32;
    let mut no_safe = 0u32;
    let mut carry_ticks = 0u32;
    let mut plot_ticks = 0u32;

    let mut field_steals = 0u32;
    let mut gk_steals = 0u32;
    let mut loose_steals = 0u32;

    println!("at_tackle_log  t=0..{secs}s  titanium={titanium_side:?}\n");
    println!(
        "{:<8} {:<6} {:<22} {:>4} {:>4} {:>4} {:>8} reason",
        "event", "t", "who", "fail", "pred", "any", "offset"
    );

    let mut t = 0.0f32;
    while t < secs {
        debug_draw::begin_frame();
        let (home_api, away_api) = world.build_apis();
        let home_out = home.think(&home_api);
        let away_out = away.think(&away_api);
        let frame = debug_draw::snapshot();

        world.step_with_commands(&home_out, &away_out, FIXED_DT);
        t += FIXED_DT;

        let carrier = world.possession.carrier;
        for slot in 1..=3 {
            let prefix = format!("Titanium.P{slot}.AT");
            let fail_key = format!("{prefix}.FailReason");
            let Some(fail) = plot_val(&frame, &fail_key) else {
                continue;
            };
            plot_ticks += 1;
            if (fail - 0.0).abs() < 0.5 {
                carry_ticks += 1;
            } else if (fail - 1.0).abs() < 0.5 {
                no_safe += 1;
            } else if (fail - 2.0).abs() < 0.5 {
                miscalc += 1;
                let pred = plot_val(&frame, &format!("{prefix}.PredictedSafe")).unwrap_or(-1.0);
                let any = plot_val(&frame, &format!("{prefix}.AnySafeFound")).unwrap_or(-1.0);
                let off = plot_val(&frame, &format!("{prefix}.ChosenOffset")).unwrap_or(0.0);
                println!(
                    "MISCALC  {:6.2} {:<18} {:4.0} {:4.0} {:4.0} {:8.3} P{slot} miscalculation",
                    t - FIXED_DT,
                    format!("{:?}", carrier),
                    fail,
                    pred,
                    any,
                    off,
                );
            }
        }

        if carrier != prev_carrier {
            let lost_titanium = prev_carrier
                .map(|(team, _)| team == titanium_side)
                .unwrap_or(false)
                && carrier.map(|(team, _)| team != titanium_side).unwrap_or(true);

            if lost_titanium {
                steals += 1;
                let prev_pid = prev_carrier.map(|(_, id)| id).unwrap_or(0);
                if prev_pid >= 4 {
                    gk_steals += 1;
                } else if carrier.is_none() {
                    loose_steals += 1;
                    field_steals += 1;
                } else {
                    field_steals += 1;
                }

                let to_label = match carrier {
                    None => "loose".to_string(),
                    Some((team, id)) => format!("{:?} P{id}", team),
                };

                if prev_pid <= 3 {
                    let prefix = format!("Titanium.P{prev_pid}.AT");
                    let fail = plot_val(&frame, &format!("{prefix}.FailReason"));
                    let pred = plot_val(&frame, &format!("{prefix}.PredictedSafe"));
                    let any = plot_val(&frame, &format!("{prefix}.AnySafeFound"));
                    let off = plot_val(&frame, &format!("{prefix}.ChosenOffset"));
                    let f = fail.unwrap_or(-1.0);
                    println!(
                        "STEAL    {:6.2} P{prev_pid} -> {to_label:<12} {:4.0} {:4.0} {:4.0} {:8.3} {}",
                        t - FIXED_DT,
                        f,
                        pred.unwrap_or(-1.0),
                        any.unwrap_or(-1.0),
                        off.unwrap_or(0.0),
                        fail_label(f),
                    );
                } else {
                    println!(
                        "STEAL    {:6.2} P{prev_pid} -> {to_label:<12} (GK — no anti-tackle)",
                        t - FIXED_DT,
                    );
                }
            }
            prev_carrier = carrier;
        }
    }

    println!("\n== summary ==");
    println!("  sim_time          = {t:.1}s");
    println!(
        "  score             = {}-{}",
        world.match_state.score_home, world.match_state.score_away
    );
    println!("  AT plot ticks     = {plot_ticks}");
    println!("  carry ticks (ok)  = {carry_ticks}");
    println!("  no_safe_angle     = {no_safe}");
    println!("  miscalculation    = {miscalc}");
    println!("  titanium steals   = {steals} (field P1-P3: {field_steals}, GK P4: {gk_steals}, incl loose: {loose_steals})");
    if plot_ticks == 0 {
        println!("  WARN: no TimePlot AT channels — rebuild challenger WITHOUT --strip-debug");
    }
    ExitCode::SUCCESS
}
