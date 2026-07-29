// Diagnostic: check kickoff positions with graph-declared formations
use aicomp_soccer_sim::*;
use aicomp_soccer_sim::brain::TeamId;
use aicomp_soccer_sim::match_state::{place_kickoff, KickoffFormations};
use aicomp_soccer_sim::params::SimParams;
use aicomp_soccer_sim::ball::Ball;
use aicomp_soccer_sim::player::Player;
use bevy::prelude::Vec2;

fn main() {
    let params = SimParams::default();
    let circle_r = params.kickoff_circle_r;

    // Test 1: Default formations (no graph declarations)
    println!("=== TEST 1: Default formations ===");
    for opening in [TeamId::Home, TeamId::Away] {
        let world = MatchWorld::new_kickoff_opening(params.clone(), opening);
        println!("\nOpening: {:?} kicks off", opening);
        for p in &world.players {
            let dist = p.pos.length();
            let is_kicker = p.team == opening;
            let role = if is_kicker { "KICKER" } else { "RECV" };
            let status = if dist < circle_r { "INSIDE" } else { "out" };
            println!("  {:?} P{} ({:7.2},{:7.2}) d={:.2} {} {}", p.team, p.id.0, p.pos.x, p.pos.y, dist, role, status);
        }
    }

    // Test 2: Graph declares P1 at (0,0) for both teams
    println!("\n=== TEST 2: Graph declares P1 at (0,0) both teams ===");
    let mut formations = KickoffFormations::default();
    formations.home = [Some(Vec2::new(0.0, 0.0)), None, None, None];
    formations.away = [Some(Vec2::new(0.0, 0.0)), None, None, None];

    for opening in [TeamId::Home, TeamId::Away] {
        println!("\nOpening: {:?} kicks off", opening);
        let mut ball = Ball::default();
        let mut players: Vec<Player> = Vec::new();
        for team in [TeamId::Home, TeamId::Away] {
            for id in aicomp_soccer_sim::player::PlayerId::ALL {
                players.push(Player {
                    team, id, pos: Vec2::ZERO, vel: Vec2::ZERO, facing: Vec2::X,
                    stamina: 1.0, stamina_regen_lock_left: 0.0, shot_charge: 0.0,
                    charge_warmup_left: 0.0, interact_bits: 0, grab_hold_active: false,
                });
            }
        }
        place_kickoff(&mut ball, &mut players, opening, &params, &formations);
        for p in &players {
            let dist = p.pos.length();
            let is_kicker = p.team == opening;
            let role = if is_kicker { "KICKER" } else { "RECV" };
            let status = if dist < circle_r { "INSIDE" } else { "out" };
            println!("  {:?} P{} ({:7.2},{:7.2}) d={:.2} {} {}", p.team, p.id.0, p.pos.x, p.pos.y, dist, role, status);
        }
    }

    // Test 3: Graph declares P1 at (0,0) for ONLY the kicking team
    println!("\n=== TEST 3: Graph declares P1 at (0,0) for kicking team only ===");
    for opening in [TeamId::Home, TeamId::Away] {
        let mut form = KickoffFormations::default();
        if opening == TeamId::Home {
            form.home = [Some(Vec2::new(0.0, 0.0)), None, None, None];
        } else {
            form.away = [Some(Vec2::new(0.0, 0.0)), None, None, None];
        }

        println!("\nOpening: {:?} kicks off (declared P1 at 0,0)", opening);
        let mut ball = Ball::default();
        let mut players: Vec<Player> = Vec::new();
        for team in [TeamId::Home, TeamId::Away] {
            for id in aicomp_soccer_sim::player::PlayerId::ALL {
                players.push(Player {
                    team, id, pos: Vec2::ZERO, vel: Vec2::ZERO, facing: Vec2::X,
                    stamina: 1.0, stamina_regen_lock_left: 0.0, shot_charge: 0.0,
                    charge_warmup_left: 0.0, interact_bits: 0, grab_hold_active: false,
                });
            }
        }
        place_kickoff(&mut ball, &mut players, opening, &params, &form);
        for p in &players {
            let dist = p.pos.length();
            let is_kicker = p.team == opening;
            let role = if is_kicker { "KICKER" } else { "RECV" };
            let status = if dist < circle_r { "INSIDE" } else { "out" };
            println!("  {:?} P{} ({:7.2},{:7.2}) d={:.2} {} {}", p.team, p.id.0, p.pos.x, p.pos.y, dist, role, status);
        }
    }
}
