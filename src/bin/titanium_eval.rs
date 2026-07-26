//! Small strength-evaluation helper for Titanium.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use aicomp_soccer_sim::batch::{
    run_batch_parallel, BrainInput, GraphEngine, MatchJob, BatchMatchResult,
};
use aicomp_soccer_sim::brain::TeamId;
use aicomp_soccer_sim::params::SimParams;

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut trials = 10usize;
    let mut secs = 900.0f32;
    let mut jobs = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--trials" => {
                i += 1;
                trials = argv.get(i).and_then(|v| v.parse().ok()).unwrap_or(trials);
            }
            "--secs" => {
                i += 1;
                secs = argv.get(i).and_then(|v| v.parse().ok()).unwrap_or(secs);
            }
            "--jobs" => {
                i += 1;
                jobs = argv.get(i).and_then(|v| v.parse().ok());
            }
            "--quiet" => {}
            "-h" | "--help" => {
                println!("titanium_eval [--trials N] [--secs S] [--jobs N] [--quiet]");
                return;
            }
            other => {
                eprintln!("unknown arg '{other}'");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let match_set = |home: BrainInput, away: BrainInput| {
        (0..trials)
            .map(|n| MatchJob {
                secs,
                home: home.clone(),
                away: away.clone(),
                opening: if n % 2 == 0 {
                    TeamId::Home
                } else {
                    TeamId::Away
                },
                seed: Some(n as u64),
                until_goal: false,
                win_goals: Some(5),
                engine: GraphEngine::Runtime,
                params: SimParams::default(),
                job_index: Some(n),
                mercy: true,
            })
            .collect::<Vec<_>>()
    };

    let against_aia = run_batch_parallel(
        match_set(BrainInput::Titanium, BrainInput::Aia),
        jobs,
    );
    let self_play = run_batch_parallel(
        match_set(BrainInput::Titanium, BrainInput::Titanium),
        jobs,
    );
    let summary = |results: &[Result<BatchMatchResult, String>]| {
        let mut wins = 0usize;
        let mut losses = 0usize;
        let mut draws = 0usize;
        for result in results {
            if let Ok(r) = result {
                if r.score_home > r.score_away {
                    wins += 1;
                } else if r.score_home < r.score_away {
                    losses += 1;
                } else {
                    draws += 1;
                }
            }
        }
        serde_json::json!({
            "wins": wins,
            "losses": losses,
            "draws": draws,
            "completed": wins + losses + draws,
            "trials": results.len()
        })
    };
    let record = serde_json::json!({
        "trials": trials,
        "secs": secs,
        "titanium_vs_aia": summary(&against_aia),
        "titanium_vs_titanium": summary(&self_play)
    });
    let path = PathBuf::from("data/titanium/eval_log.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create eval log directory");
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open eval log");
    writeln!(file, "{record}").expect("append eval record");
    println!("{record}");
}
