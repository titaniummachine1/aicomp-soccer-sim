//! Default worker layout: render (main) + team A thread + team B thread.
//!
//! Physics/possession stay on the Bevy fixed-update (render/main) side.
//! Each team brain evaluates its `TeamApi` snapshot on its own OS thread.

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::api::TeamApi;
use crate::brain::{BrainOutput, ChaseBallBrain, TeamBrain, TeamId};

enum TeamMsg {
    Think(TeamApi),
    Shutdown,
}

/// Persistent home/away brain workers. Main/render thread owns physics + draw.
pub struct TeamWorkers {
    home_tx: mpsc::Sender<TeamMsg>,
    away_tx: mpsc::Sender<TeamMsg>,
    home_rx: mpsc::Receiver<BrainOutput>,
    away_rx: mpsc::Receiver<BrainOutput>,
    home_join: Option<JoinHandle<()>>,
    away_join: Option<JoinHandle<()>>,
}

impl TeamWorkers {
    pub fn spawn() -> Self {
        let (home_tx, home_in) = mpsc::channel::<TeamMsg>();
        let (away_tx, away_in) = mpsc::channel::<TeamMsg>();
        let (home_out_tx, home_rx) = mpsc::channel::<BrainOutput>();
        let (away_out_tx, away_rx) = mpsc::channel::<BrainOutput>();

        let home_join = thread::Builder::new()
            .name("team-a-home".into())
            .spawn(move || team_loop(TeamId::Home, home_in, home_out_tx))
            .expect("spawn team-a");

        let away_join = thread::Builder::new()
            .name("team-b-away".into())
            .spawn(move || team_loop(TeamId::Away, away_in, away_out_tx))
            .expect("spawn team-b");

        Self {
            home_tx,
            away_tx,
            home_rx,
            away_rx,
            home_join: Some(home_join),
            away_join: Some(away_join),
        }
    }

    /// Dispatch both team brains in parallel; block until both finish this tick.
    pub fn think_parallel(&self, home_api: TeamApi, away_api: TeamApi) -> (BrainOutput, BrainOutput) {
        self.home_tx
            .send(TeamMsg::Think(home_api))
            .expect("team-a alive");
        self.away_tx
            .send(TeamMsg::Think(away_api))
            .expect("team-b alive");
        let home = self.home_rx.recv().expect("team-a result");
        let away = self.away_rx.recv().expect("team-b result");
        (home, away)
    }
}

impl Drop for TeamWorkers {
    fn drop(&mut self) {
        let _ = self.home_tx.send(TeamMsg::Shutdown);
        let _ = self.away_tx.send(TeamMsg::Shutdown);
        if let Some(h) = self.home_join.take() {
            let _ = h.join();
        }
        if let Some(h) = self.away_join.take() {
            let _ = h.join();
        }
    }
}

fn team_loop(
    team: TeamId,
    rx: mpsc::Receiver<TeamMsg>,
    tx: mpsc::Sender<BrainOutput>,
) {
    let mut brain = ChaseBallBrain;
    while let Ok(msg) = rx.recv() {
        match msg {
            TeamMsg::Shutdown => break,
            TeamMsg::Think(api) => {
                debug_assert_eq!(api.team, team);
                let out = brain.think(&api);
                if tx.send(out).is_err() {
                    break;
                }
            }
        }
    }
}
