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
use crate::probe_brains::{Test1Brain, Test2Brain};
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
    Aia,
    Graph(PathBuf),
}

impl BrainInput {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix("graph:") {
            return Ok(Self::Graph(PathBuf::from(rest)));
        }
        match t.to_ascii_lowercase().as_str() {
            "chase" => Ok(Self::Chase),
            "idle" | "park" => Ok(Self::Idle),
            "test1" => Ok(Self::Test1),
            "test2" => Ok(Self::Test2),
            "aia" => Ok(Self::Aia),
            other => Err(format!(
                "unknown brain '{other}' (chase|idle|test1|test2|aia|graph:<path>)"
            )),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Chase => "chase".into(),
            Self::Idle => "idle".into(),
            Self::Test1 => "test1".into(),
            Self::Test2 => "test2".into(),
            Self::Aia => "aia".into(),
            Self::Graph(p) => format!("graph:{}", p.display()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEngine {
    /// Permanent oracle — recursive GraphBrain.
    Reference,
    /// O0/O1 compiled RuntimeProgram.
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
        BrainInput::Aia => {
            let path = soccer_saves_dir().join("AIA.txt");
            build_graph_brain(&path, engine, cache)?
        }
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

    if !quiet {
        let eng = match job.engine {
            GraphEngine::Reference => "reference",
            GraphEngine::Runtime => "runtime",
        };
        eprintln!(
            "match_job home={} away={} opening={} secs={} until_goal={} engine={eng} FIXED_DT={FIXED_DT} (max-speed)",
            job.home.label(),
            job.away.label(),
            opening_str(job.opening),
            job.secs,
            job.until_goal
        );
    }

    let start_score = world.match_state.score_home + world.match_state.score_away;
    let mut ticks = 0u64;
    let mut goal_stopped = false;
    let max_ticks = ((job.secs / FIXED_DT).ceil() as u64).max(1);

    while ticks < max_ticks {
        world.step_brains(&mut *home, &mut *away, FIXED_DT);
        ticks += 1;
        if job.until_goal {
            let scored = world.match_state.score_home + world.match_state.score_away;
            if scored > start_score {
                goal_stopped = true;
                break;
            }
        }
        if !quiet && ticks % 500 == 0 {
            eprintln!(
                "  t={:.1}s score={}-{} phase={:?}",
                ticks as f32 * FIXED_DT,
                world.match_state.score_home,
                world.match_state.score_away,
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
