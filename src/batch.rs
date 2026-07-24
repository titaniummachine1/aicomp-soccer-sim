//! Match-level micropool batch runner.
//!
//! Reserves one logical CPU for the system/UI (`available_parallelism - 1`)
//! and runs full matches in parallel — not a per-node pure DAG.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::available_parallelism;
use std::time::Instant;

use serde::Serialize;

use crate::brain::{ChaseBallBrain, IdleBrain, TeamBrain, TeamId};
use crate::graph::{load_team_graph, GraphBrain};
use crate::graph_vm::{CachedProgram, RuntimeBrain};
use crate::params::SimParams;
use crate::probe_brains::{PerfectControllerBrain, Test1Brain, Test2Brain};
#[cfg(feature = "nn_train")]
use crate::train::TrainedBrain;
use crate::titanium::TitaniumBrain;

/// Clear copy when `nn_train` is gated off (default). Not a training bug.
pub const NN_TRAIN_GATED_MSG: &str = "brain 'trained' is temporarily disabled \
(feature nn_train OFF) to isolate Bevy cold builds — training code is intact; \
rebuild with --features nn_train";
use crate::world::{MatchWorld, FIXED_DT};

/// Leave one logical CPU free ("core 0" reservation for system/UI).
pub fn reserved_worker_threads() -> usize {
    available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1)
}

/// Install a match-level thread pool sized for batch jobs.
pub fn install_match_pool(jobs: Option<usize>) -> micropool::ThreadPool {
    let n = jobs.unwrap_or_else(reserved_worker_threads);
    micropool::ThreadPoolBuilder::default()
        .num_threads(n.max(1))
        .build()
}

/// Thread-safe cache of compiled RuntimePrograms keyed by canonical absolute path.
#[derive(Default)]
pub struct ProgramCache {
    inner: Mutex<HashMap<String, CachedProgram>>,
}

impl ProgramCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonical / absolute path string used as the cache key.
    pub fn cache_key(path: &Path) -> String {
        path.canonicalize()
            .unwrap_or_else(|_| {
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(path)
                }
            })
            .to_string_lossy()
            .into_owned()
    }

    /// Load + lower + O1-opt once per path; subsequent calls clone [`CachedProgram`].
    pub fn get_or_compile(&self, path: &Path) -> Result<CachedProgram, String> {
        let key = Self::cache_key(path);
        if let Some(hit) = self.inner.lock().map_err(|e| e.to_string())?.get(&key) {
            return Ok(hit.clone());
        }
        let graph = load_team_graph(path).map_err(|e| format!("load graph {path:?}: {e}"))?;
        let cached = RuntimeBrain::compile_cached(graph);
        let mut map = self.inner.lock().map_err(|e| e.to_string())?;
        Ok(map.entry(key).or_insert(cached).clone())
    }
}

/// Built-in / graph brain selector for batch jobs.
#[derive(Debug, Clone)]
pub enum BrainInput {
    Chase,
    Idle,
    Test1,
    Test2,
    Perfect,
    Aia,
    Titanium,
    #[cfg(feature = "nn_train")]
    Trained,
    Graph(PathBuf),
}

impl BrainInput {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix("graph:") {
            let path = PathBuf::from(rest);

            if path.exists() {
                return Ok(Self::Graph(path));
            }

            let saves_dir = soccer_saves_dir();
            let saves_path = saves_dir.join(&path);
            if saves_path.exists() && saves_path.is_file() {
                return Ok(Self::Graph(saves_path));
            }

            let txt_path = saves_dir.join(&path).with_extension("txt");
            if txt_path.exists() && txt_path.is_file() {
                return Ok(Self::Graph(txt_path));
            }

            return Err(format!(
                "None of the following paths were valid:\n{}\n{}\n{}",
                path.display(),
                saves_path.display(),
                txt_path.display()
            ));
        }
        match t.to_ascii_lowercase().as_str() {
            "chase" => Ok(Self::Chase),
            "idle" | "park" => Ok(Self::Idle),
            "test1" => Ok(Self::Test1),
            "test2" => Ok(Self::Test2),
            "perfect" | "kb" | "keyboard" => Ok(Self::Perfect),
            "aia" | "aia3" => Ok(Self::Aia),
            "titanium" | "ti" => Ok(Self::Titanium),
            "trained" => {
                #[cfg(feature = "nn_train")]
                {
                    Ok(Self::Trained)
                }
                #[cfg(not(feature = "nn_train"))]
                {
                    Err(NN_TRAIN_GATED_MSG.into())
                }
            }
            other => Err(format!(
                "unknown brain '{other}' (chase|idle|test1|test2|perfect|aia|aia3|titanium|trained|graph:<path>)"
            )),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Chase => "chase".into(),
            Self::Idle => "idle".into(),
            Self::Test1 => "test1".into(),
            Self::Test2 => "test2".into(),
            Self::Perfect => "perfect".into(),
            Self::Aia => {
                let p = soccer_aia_graph_path();
                if p.file_name().and_then(|s| s.to_str()) == Some("AIA3.txt") {
                    "aia3".into()
                } else {
                    "aia".into()
                }
            }
            Self::Titanium => "titanium".into(),
            #[cfg(feature = "nn_train")]
            Self::Trained => "trained".into(),
            Self::Graph(p) => format!("graph:{}", p.display()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEngine {
    /// Slow recursive GraphBrain — **tests / acceptance only**, never match CLIs.
    Reference,
    /// Compiled RuntimeProgram (O0/O1). Required for all match runners.
    Runtime,
}

/// One full-match job for the micropool batch runner.
#[derive(Debug, Clone)]
pub struct MatchJob {
    pub secs: f32,
    pub home: BrainInput,
    pub away: BrainInput,
    pub opening: TeamId,
    pub seed: Option<u64>,
    pub until_goal: bool,
    /// Stop when either side reaches this score (first-to-N). `None` = unused.
    pub win_goals: Option<u32>,
    pub engine: GraphEngine,
    pub params: SimParams,
    pub job_index: Option<usize>,
}

/// Match outcome — same fields as headless `MatchResult`, plus optional batch metadata.
#[derive(Debug, Clone, Serialize)]
pub struct BatchMatchResult {
    pub ok: bool,
    pub fixed_dt: f32,
    pub secs_requested: f32,
    pub clock_s: f32,
    pub ticks: u64,
    pub opening: &'static str,
    pub seed: Option<u64>,
    pub home: String,
    pub away: String,
    pub score_home: u32,
    pub score_away: u32,
    pub phase: String,
    pub until_goal: bool,
    pub goal_stopped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_goals: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
}

pub fn opening_str(t: TeamId) -> &'static str {
    match t {
        TeamId::Home => "home",
        TeamId::Away => "away",
    }
}

pub fn soccer_saves_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("Unicorn One")
        .join("AIComp")
        .join("Saves")
        .join("Soccer")
}

/// Unity Saves graph for `aia` / `aia3`: prefer **AIA3.txt**, else AIA.txt.
pub fn soccer_aia_graph_path() -> PathBuf {
    let saves = soccer_saves_dir();
    let aia3 = saves.join("AIA3.txt");
    if aia3.is_file() {
        return aia3;
    }
    saves.join("AIA.txt")
}

/// Default Full-match brain: AIA3/AIA graph when present, else a random built-in
/// (chase/idle). Never auto-picks Titanium — that is opt-in (`--home titanium`
/// / Scenario 1 GK) so the sim does not silently run the Rust titanium policy.
pub fn default_team_brain() -> BrainInput {
    let saves = soccer_saves_dir();
    if saves.join("AIA3.txt").is_file() || saves.join("AIA.txt").is_file() {
        return BrainInput::Aia;
    }
    let bit = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() & 1)
        .unwrap_or(0);
    if bit == 0 {
        BrainInput::Chase
    } else {
        BrainInput::Idle
    }
}

fn build_brain(
    input: &BrainInput,
    engine: GraphEngine,
    cache: Option<&ProgramCache>,
) -> Result<Box<dyn TeamBrain>, String> {
    Ok(match input {
        BrainInput::Chase => Box::new(ChaseBallBrain),
        BrainInput::Idle => Box::new(IdleBrain),
        BrainInput::Test1 => Box::new(Test1Brain::default()),
        BrainInput::Test2 => Box::new(Test2Brain::default()),
        BrainInput::Perfect => Box::new(PerfectControllerBrain),
        BrainInput::Aia => {
            let path = soccer_aia_graph_path();
            build_graph_brain(&path, engine, cache)?
        }
        BrainInput::Titanium => Box::new(TitaniumBrain::default()),
        #[cfg(feature = "nn_train")]
        BrainInput::Trained => Box::new(TrainedBrain::default()),
        BrainInput::Graph(path) => build_graph_brain(path, engine, cache)?,
    })
}

fn build_graph_brain(
    path: &Path,
    engine: GraphEngine,
    cache: Option<&ProgramCache>,
) -> Result<Box<dyn TeamBrain>, String> {
    match engine {
        GraphEngine::Reference => {
            let g = load_team_graph(path).map_err(|e| format!("load graph {path:?}: {e}"))?;
            Ok(Box::new(GraphBrain::new(g)))
        }
        GraphEngine::Runtime => {
            if let Some(cache) = cache {
                let cached = cache.get_or_compile(path)?;
                Ok(Box::new(RuntimeBrain::from_cached(cached)))
            } else {
                let g = load_team_graph(path).map_err(|e| format!("load graph {path:?}: {e}"))?;
                Ok(Box::new(RuntimeBrain::compile(g)))
            }
        }
    }
}

/// Shared single-match logic (conceptually extracted from `soccer_headless::run`).
pub fn run_match_job(
    job: &MatchJob,
    cache: Option<&ProgramCache>,
    quiet: bool,
) -> Result<BatchMatchResult, String> {
    let wall = Instant::now();
    let mut home = build_brain(&job.home, job.engine, cache)?;
    let mut away = build_brain(&job.away, job.engine, cache)?;
    let mut world = MatchWorld::new_kickoff_opening(job.params.clone(), job.opening);
    // Batch / headless parity: never inherit a viewer-zeroed GoalPause.
    if world.params.kickoff_delay_s < 1.0 {
        world.params.kickoff_delay_s = 4.9;
    }

    if !quiet {
        let eng = match job.engine {
            GraphEngine::Reference => "reference",
            GraphEngine::Runtime => "runtime",
        };
        eprintln!(
            "match_job home={} away={} opening={} secs={} until_goal={} win_goals={:?} engine={eng} FIXED_DT={FIXED_DT} kickoff_delay_s={:.2} (max-speed)",
            job.home.label(),
            job.away.label(),
            opening_str(job.opening),
            job.secs,
            job.until_goal,
            job.win_goals,
            world.params.kickoff_delay_s
        );
    }

    let start_score = world.match_state.score_home + world.match_state.score_away;
    let mut ticks = 0u64;
    let mut goal_stopped = false;
    let mut last_total = start_score;
    let max_ticks = ((job.secs / FIXED_DT).ceil() as u64).max(1);

    while ticks < max_ticks {
        world.step_brains(&mut *home, &mut *away, FIXED_DT);
        ticks += 1;
        let sh = world.match_state.score_home;
        let sa = world.match_state.score_away;
        let total = sh + sa;
        if !quiet && total > last_total {
            eprintln!(
                "  goal t={:.1}s score={sh}-{sa} ko={:?} first_kick={} ko_touch={}",
                ticks as f32 * FIXED_DT,
                world.match_state.kickoff_team,
                world.possession.first_kick_done,
                world.possession.kickoff_touch_done,
            );
            last_total = total;
        }
        if job.until_goal && total > start_score {
            goal_stopped = true;
            break;
        }
        if let Some(win) = job.win_goals {
            if sh >= win || sa >= win {
                goal_stopped = true;
                break;
            }
        }
        if !quiet && ticks % 2000 == 0 {
            eprintln!(
                "  t={:.1}s score={sh}-{sa} phase={:?}",
                ticks as f32 * FIXED_DT,
                world.match_state.phase
            );
        }
    }

    Ok(BatchMatchResult {
        ok: true,
        fixed_dt: FIXED_DT,
        secs_requested: job.secs,
        clock_s: ticks as f32 * FIXED_DT,
        ticks,
        opening: opening_str(job.opening),
        seed: job.seed,
        home: job.home.label(),
        away: job.away.label(),
        score_home: world.match_state.score_home,
        score_away: world.match_state.score_away,
        phase: format!("{:?}", world.match_state.phase),
        until_goal: job.until_goal,
        goal_stopped,
        win_goals: job.win_goals,
        job_index: job.job_index,
        wall_ms: Some(wall.elapsed().as_millis() as u64),
    })
}

/// Run full matches in parallel on a micropool; results preserve job order.
pub fn run_batch_parallel(
    jobs: Vec<MatchJob>,
    pool_threads: Option<usize>,
) -> Vec<Result<BatchMatchResult, String>> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let pool = install_match_pool(pool_threads);
    let cache = Arc::new(ProgramCache::new());
    let handles: Vec<_> = jobs
        .into_iter()
        .enumerate()
        .map(|(i, mut job)| {
            if job.job_index.is_none() {
                job.job_index = Some(i);
            }
            let cache = Arc::clone(&cache);
            pool.spawn_owned(move || run_match_job(&job, Some(&cache), true))
        })
        .collect();
    handles.into_iter().map(|h| h.join()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_worker_threads_at_least_one() {
        assert!(reserved_worker_threads() >= 1);
    }

    #[test]
    fn batch_parallel_four_chase_vs_idle() {
        let params = SimParams::default();
        let jobs: Vec<MatchJob> = (0..4)
            .map(|i| {
                let seed = i as u64;
                MatchJob {
                    secs: 0.5,
                    home: BrainInput::Chase,
                    away: BrainInput::Idle,
                    opening: if seed % 2 == 0 {
                        TeamId::Home
                    } else {
                        TeamId::Away
                    },
                    seed: Some(seed),
                    until_goal: false,
                    win_goals: None,
                    engine: GraphEngine::Runtime,
                    params: params.clone(),
                    job_index: Some(i),
                }
            })
            .collect();

        let results = run_batch_parallel(jobs, Some(2));
        assert_eq!(results.len(), 4);
        for (i, r) in results.iter().enumerate() {
            let r = r.as_ref().expect("match ok");
            assert!(r.ok);
            assert_eq!(r.job_index, Some(i));
            assert!(r.wall_ms.is_some());
            assert!(r.ticks > 0);
        }
    }
}
