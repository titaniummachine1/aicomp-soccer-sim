//! Default worker layout helpers: timed parallel team brains with a barrier.
//!
//! Physics stays on the caller. Both teams must finish `think` before the tick
//! advances — no partial / skipped brain results.

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

/// Run both brains in parallel; block until **both** complete (tick barrier).
pub fn think_barrier<H: TeamBrain + Send, A: TeamBrain + Send>(
    home: &mut H,
    away: &mut A,
    home_api: TeamApi,
    away_api: TeamApi,
) -> (BrainOutput, BrainOutput, ThinkTimings) {
    let wall0 = Instant::now();
    let (home_out, home_dt, away_out, away_dt) = std::thread::scope(|scope| {
        let home_h = scope.spawn(|| {
            let t0 = Instant::now();
            let out = home.think(&home_api);
            (out, t0.elapsed())
        });
        let away_h = scope.spawn(|| {
            let t0 = Instant::now();
            let out = away.think(&away_api);
            (out, t0.elapsed())
        });
        let (ho, hd) = home_h.join().expect("home think");
        let (ao, ad) = away_h.join().expect("away think");
        (ho, hd, ao, ad)
    });
    (
        home_out,
        away_out,
        ThinkTimings {
            home: home_dt,
            away: away_dt,
            wall: wall0.elapsed(),
        },
    )
}
