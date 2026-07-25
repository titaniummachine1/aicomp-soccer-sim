//! Value watchpoints: "who produced this number?"
//!
//! When a drawn line starts at (0,0) or a target is 250 m off the pitch, the
//! hard part is not seeing the wrong value — it is finding which node made it.
//! Reading the graph backwards by hand is hours of work.
//!
//! This watches every instruction result and reports the graph node whose sID
//! produced a matching value, so the answer is a grep in the .txt away.
//!
//! Enable with an env var — no CLI plumbing, works with every existing tool:
//!
//! ```text
//! TITANIUM_WATCH=0.0 cargo run --release --bin probe_dump -- --home graph:...
//! TITANIUM_WATCH=0.0 TITANIUM_WATCH_EPS=1e-6 TITANIUM_WATCH_MAX=40 ...
//! ```
//!
//! Deliberately dumb and un-optimised: it reports raw per-instruction results
//! rather than anything folded or reordered, because a value that has been
//! optimised away cannot be traced back to the node that caused it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

struct Config {
    value: f32,
    eps: f32,
    max: usize,
}

static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
static HITS: AtomicUsize = AtomicUsize::new(0);

fn config() -> Option<&'static Config> {
    CONFIG
        .get_or_init(|| {
            let raw = std::env::var("TITANIUM_WATCH").ok()?;
            let value: f32 = raw.trim().parse().ok()?;
            let eps = std::env::var("TITANIUM_WATCH_EPS")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1e-6);
            let max = std::env::var("TITANIUM_WATCH_MAX")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(25);
            eprintln!(
                "[watch] armed on {value} (eps {eps}, max {max} reports) — \
                 grep a reported sID in the graph .txt to find the node"
            );
            Some(Config { value, eps, max })
        })
        .as_ref()
}

pub fn is_armed() -> bool {
    config().is_some()
}

/// Report if `produced` matches the watched value. `sid`/`port` come straight
/// from the instruction, so they point at the exact graph node.
pub fn check(produced: f32, sid: &str, port: &str, op: &str) {
    let Some(cfg) = config() else { return };
    if (produced - cfg.value).abs() > cfg.eps {
        return;
    }
    let n = HITS.fetch_add(1, Ordering::Relaxed);
    if n >= cfg.max {
        return;
    }
    eprintln!("[watch] {produced} produced by {op}  node sID={sid}  port={port}");
    if n + 1 == cfg.max {
        eprintln!("[watch] reached TITANIUM_WATCH_MAX — further hits suppressed");
    }
}

pub fn hits() -> usize {
    HITS.load(Ordering::Relaxed)
}
