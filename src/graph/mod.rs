//! AIComp team-graph loader + tick evaluator (MVP).
//!
//! Implements constants, float math (incl. Power), SoccerGet*, RelativePosition,
//! and SoccerController1–4. Variables / functions / sensors come later.

pub(crate) mod dropdowns;
mod eval;
pub(crate) mod load;
mod value;

#[cfg(test)]
mod operation_probe_test;

pub use eval::GraphBrain;
pub use load::{
    index_graph, load_team_graph, RawConnection, RawGraph, RawNode, RawPort, TeamGraph,
};
pub use value::GraphValue;

#[cfg(test)]
mod aia_smoke {
    use super::*;
    use crate::api::TeamApi;
    use crate::brain::{TeamBrain, TeamId};
    use crate::params::SimParams;
    use crate::world::MatchWorld;
    use std::path::PathBuf;

    #[test]
    fn aia_graph_produces_move_targets() {
        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }
        let g = load_team_graph(&path).expect("load AIA");
        assert!(
            g.set_variables.len() > 10,
            "expected many SetVariables, got {}",
            g.set_variables.len()
        );
        assert!(
            !g.create_functions.is_empty(),
            "expected CreateFunction defs"
        );
        let mut brain = GraphBrain::new(g);
        let world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);
        let (api, _) = world.build_apis();
        let out = brain.think(&api);
        eprintln!(
            "AIA moveTo: {:?}",
            out.commands.iter().map(|c| c.move_to).collect::<Vec<_>>()
        );
        let any = out.commands.iter().any(|c| c.move_to.length() > 0.05);
        assert!(any, "AIA should drive at least one non-zero moveTo");
    }

    #[allow(dead_code)]
    fn _keep_api_import(_: &TeamApi) {}
}
