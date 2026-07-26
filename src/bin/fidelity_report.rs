//! Print what the simulator actually knows it gets right.
//!
//! Two independent axes, both of which have burned us:
//!   * node TYPES — is the operation implemented at all (graph_vm::diagnostics)
//!   * getter VALUES — does an implemented getter return the game's number
//!     (api::fidelity)
//!
//! A getter is UNCERTAIN until someone compares it to a real-game reading, so
//! this list shrinks only by measuring, never by assuming.
fn main() {
    println!("{}", aicomp_soccer_sim::api::fidelity::report());
}
