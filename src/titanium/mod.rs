//! Drill/scenario scaffolding for the Titanium training harnesses.
//!
//! This module is deliberately **strategy-free**. It only sets up and tears
//! down the scripted training scenarios (park the inactive roster off-pitch,
//! freeze them each tick, place the kickoff formation). The team AI that
//! plays those scenarios is the graph loaded at runtime — see the `--gk` /
//! `--opponent` flags on the `titanium_drill*` binaries and `--home` /
//! `--away graph:<path>` on `soccer_headless`. The old native Rust GK
//! (`TitaniumBrain`) has been retired; the real, current GK logic lives in
//! the private engine repo's `scripts/build_titanium.py` and is loaded here
//! only as a compiled graph.

mod harness;

pub use harness::{
    apply_1v1_freeze, apply_2v1_freeze, apply_4v1_freeze, freeze_except, freeze_except_many,
    repark_1v1_inactive, repark_2v1_inactive, repark_4v1_inactive, setup_1v1_harness,
    setup_2v1_harness, setup_4v1_harness, PassDrillAttackerBrain, ShotExtreme,
};
