//! Default worker layout helpers: timed parallel team brains with a barrier.
//!
//! Physics stays on the caller. Both teams must finish `think` before the tick
//! advances — no partial / skipped brain results.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::api::TeamApi;
use crate::brain::{BrainOutput, TeamBrain};

#[derive(Debug, Clone, Copy, Default)]
pub struct ThinkTimings {
    pub home: Duration,
    pub away: Duration,
    pub wall: Duration,
}

impl ThinkTimings {
    pub fn home_ms(self) -> f32 {
        self.home.as_secs_f32() * 1000.0
    }
    pub fn away_ms(self) -> f32 {
        self.away.as_secs_f32() * 1000.0
    }
    pub fn wall_ms(self) -> f32 {
        self.wall.as_secs_f32() * 1000.0
    }
    pub fn slowest_label(self) -> &'static str {
        if self.home >= self.away {
            "Home(A)"
        } else {
            "Away(B)"
        }
    }
}

/// Brain lifecycle operations needed by the viewer's persistent workers.
pub trait ResettableBrain: TeamBrain {
    fn reset_state(&mut self);
}

enum WorkerCommand<B> {
    Think(TeamApi),
    Reset(Sender<()>),
    Replace(B, Sender<()>),
    Shutdown,
}

struct WorkerResult {
    output: BrainOutput,
    elapsed: Duration,
}

struct BrainWorker<B> {
    commands: Sender<WorkerCommand<B>>,
    results: Mutex<Receiver<WorkerResult>>,
    handle: Option<JoinHandle<()>>,
}

impl<B: ResettableBrain + Send + 'static> BrainWorker<B> {
    fn new(mut brain: B) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            while let Ok(command) = command_rx.recv() {
                match command {
                    WorkerCommand::Think(api) => {
                        let started = Instant::now();
                        let output = brain.think(&api);
                        result_tx
                            .send(WorkerResult {
                                output,
                                elapsed: started.elapsed(),
                            })
                            .expect("brain result receiver");
                    }
                    WorkerCommand::Reset(done) => {
                        brain.reset_state();
                        let _ = done.send(());
                    }
                    WorkerCommand::Replace(next, done) => {
                        brain = next;
                        let _ = done.send(());
                    }
                    WorkerCommand::Shutdown => break,
                }
            }
        });
        Self {
            commands: command_tx,
            results: Mutex::new(result_rx),
            handle: Some(handle),
        }
    }

    fn think(&self, api: TeamApi) {
        self.commands
            .send(WorkerCommand::Think(api))
            .expect("brain worker");
    }

    fn result(&self) -> WorkerResult {
        self.results
            .lock()
            .expect("brain result mutex")
            .recv()
            .expect("brain result")
    }

    fn replace(&self, brain: B) {
        let (done_tx, done_rx) = mpsc::channel();
        self.commands
            .send(WorkerCommand::Replace(brain, done_tx))
            .expect("brain worker");
        done_rx.recv().expect("brain replacement");
    }
}

impl<B> Drop for BrainWorker<B> {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Two persistent brain workers with a strict per-tick barrier.
///
/// Worker threads are created once, then each tick only sends two API
/// snapshots and waits for two results. This preserves the original
/// parallel-brain semantics without paying OS thread creation/scheduling cost
/// on every simulation tick.
pub struct PersistentThinkBarrier<H, A> {
    home: BrainWorker<H>,
    away: BrainWorker<A>,
}

impl<H: ResettableBrain + Send + 'static, A: ResettableBrain + Send + 'static>
    PersistentThinkBarrier<H, A>
{
    pub fn new(home: H, away: A) -> Self {
        Self {
            home: BrainWorker::new(home),
            away: BrainWorker::new(away),
        }
    }

    pub fn think(
        &self,
        home_api: TeamApi,
        away_api: TeamApi,
    ) -> (BrainOutput, BrainOutput, ThinkTimings) {
        let wall0 = Instant::now();
        self.home.think(home_api);
        self.away.think(away_api);
        let home_result = self.home.result();
        let away_result = self.away.result();
        (
            home_result.output,
            away_result.output,
            ThinkTimings {
                home: home_result.elapsed,
                away: away_result.elapsed,
                wall: wall0.elapsed(),
            },
        )
    }

    pub fn reset_state(&self) {
        // Replacing with a reset copy is not possible generically; the worker
        // receives Reset so the brain-specific state hook runs in its owner
        // thread.
        self.reset_worker(&self.home);
        self.reset_worker(&self.away);
    }

    pub fn replace_home(&self, brain: H) {
        self.home.replace(brain);
    }

    pub fn replace_away(&self, brain: A) {
        self.away.replace(brain);
    }

    fn reset_worker<B: ResettableBrain + Send + 'static>(&self, worker: &BrainWorker<B>) {
        let (done_tx, done_rx) = mpsc::channel();
        worker
            .commands
            .send(WorkerCommand::Reset(done_tx))
            .expect("brain worker");
        done_rx.recv().expect("brain reset");
    }
}
