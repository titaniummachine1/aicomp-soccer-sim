//! How far can the ball travel in ONE tick, with no kick at all?
//!
//! Two free displacements, both a consequence of confirmed rules:
//!
//!   1. ROTATION. A held ball sits at `carrier + facing * hold_offset`, and
//!      facing is set instantly. Turning 180 degrees moves the ball
//!      `2 * hold_offset` = 3.34 m in a single tick, at no cost.
//!   2. TEAMMATE TACKLE. Taking the ball off a teammate moves it from their
//!      hold point to yours: up to `interact_radius + hold_offset` = 3.42 m.
//!
//! And they compose within one tick, because interaction resolves the CARRIER
//! first and then everyone else nearest-ball-first. So the carrier turns, the
//! nearest teammate takes it, the next one takes it off them, and so on — the
//! whole chain lands before the tick ends.
//!
//! Players are laid out along -X at exactly the spacing that keeps each one in
//! reach of the ball at the moment it becomes their turn.
//!
//!     cargo run --release --bin ball_teleport

use bevy::prelude::Vec2;

use aicomp_soccer_sim::brain::{BrainCommand, BrainOutput, TeamId};
use aicomp_soccer_sim::match_state::MatchPhase;
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::world::{MatchWorld, FIXED_DT};

fn main() {
    let params = SimParams::default();
    let hold = params.hold_offset;
    let reach = params.interact_radius;
    // 1 cm inside maximum reach: at exactly `reach` the f32 gap lands on
    // 1.7500001 and `dist <= interact_radius` refuses.
    let gap = reach - 0.01;

    let mut w = MatchWorld::new_kickoff_opening(params.clone(), TeamId::Home);
    w.match_state.phase = MatchPhase::Play;
    w.possession.first_kick_done = true;
    w.possession.kickoff_touch_done = true;

    // P1 on the centre spot holding the ball out along +X.
    // After it flips to -X the ball is at -hold. Each teammate then sits `gap`
    // beyond the previous hold point, facing -X so its own hold point is a
    // further `hold` along.
    let mut chain_x = -hold; // ball position after the flip
    let mut spots = vec![0.0f32];
    for _ in 0..3 {
        let p = chain_x - gap;
        spots.push(p);
        chain_x = p - hold;
    }

    for p in w.players.iter_mut() {
        p.vel = Vec2::ZERO;
        p.stamina = 1.0;
        p.shot_charge = 0.0;
        p.charge_warmup_left = 0.0;
        p.interact_bits = 0;
        p.pos = if p.team == TeamId::Away {
            Vec2::new(0.0, 60.0)
        } else {
            Vec2::new(spots[p.id.0 as usize - 1], 0.0)
        };
        // Everyone faces -X, the direction the ball is being dragged.
        p.facing = -Vec2::X;
    }
    w.players[0].facing = Vec2::X; // P1 starts looking RIGHT
    w.possession.carrier = Some((TeamId::Home, 1));
    w.ball.held = true;
    w.ball.pos = w.players[0].pos + Vec2::X * hold;
    w.ball.vel = Vec2::ZERO;

    println!("hold offset {hold:.2}   interact radius {reach:.2}   dt {FIXED_DT}");
    for p in w.players.iter().filter(|p| p.team == TeamId::Home) {
        println!("  P{} at x = {:7.2}", p.id.0, p.pos.x);
    }
    let start = w.ball.pos;
    println!("\nball starts at x = {:.2}  (P1 looking RIGHT)\n", start.x);

    // ONE tick. P1 turns to look LEFT; every teammate presses Interact.
    let mut home = BrainOutput::default();
    home.commands[0] = BrainCommand {
        // Aim far to the -X side: facing becomes -X instantly, dragging the
        // held ball across the carrier from +hold to -hold.
        move_to: Vec2::new(-100.0, 0.0),
        sprint: false,
        interact: false,
    };
    for slot in 1..4 {
        home.commands[slot] = BrainCommand {
            move_to: w.players[slot].pos + Vec2::new(-100.0, 0.0),
            sprint: false,
            interact: true,
        };
    }
    w.step_with_commands(&home, &BrainOutput::default(), FIXED_DT);

    let end = w.ball.pos;
    let moved = (end - start).length();
    println!("after ONE tick:");
    println!("  carrier      {:?}", w.possession.carrier);
    println!("  ball x       {:.2}  (from {:.2})", end.x, start.x);
    println!("  displacement {moved:.2} m in {FIXED_DT} s");
    println!("  effective    {:.1} m/s", moved / FIXED_DT);
    print!("  stamina      ");
    for p in w.players.iter().filter(|p| p.team == TeamId::Home) {
        print!("P{} {:.2}  ", p.id.0, p.stamina);
    }
    println!("\n");

    println!(
        "  for comparison, the hardest possible KICK leaves the foot at {:.2} m/s",
        params.kick_max_speed
    );
    println!(
        "  budget: 2*hold {:.2} from the flip, then {} tackles x (reach + hold) {:.2}",
        2.0 * hold,
        3,
        reach + hold
    );
}
