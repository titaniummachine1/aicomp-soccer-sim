//! RuntimeBrain — compiled TeamGraph via the Graph VM.

use std::sync::Arc;

use crate::api::TeamApi;
use crate::brain::{BrainOutput, TeamBrain};
use crate::graph::TeamGraph;
use crate::graph_vm::builder::ProgramBuilder;
use crate::graph_vm::context::ExecutionContext;
use crate::graph_vm::interpreter::Interpreter;
use crate::graph_vm::lower::{ApiSlotTable, Lowerer, VariableTable};
use crate::graph_vm::passes::PassManager;
use crate::graph_vm::program::{Backend, RuntimeProgram};
use crate::graph_vm::trace::ObservableTrace;
use crate::graph_vm::value::VmValue;

/// Shared compiled program + tables — clone cheaply to spawn many RuntimeBrain instances.
#[derive(Debug, Clone)]
pub struct CachedProgram {
    pub program: Arc<RuntimeProgram>,
    pub vars: VariableTable,
    pub apis: ApiSlotTable,
}

#[derive(Debug)]
pub struct RuntimeBrain {
    program: Arc<RuntimeProgram>,
    vars: VariableTable,
    apis: ApiSlotTable,
    persistent_vars: Vec<VmValue>,
    backend: Interpreter,
    trace: Option<ObservableTrace>,
}

impl RuntimeBrain {
    pub fn compile(graph: TeamGraph) -> Self {
        Self::from_cached(Self::compile_cached(graph))
    }

    /// Lower + O1 optimize once; reuse via [`Self::from_cached`] / [`CachedProgram`].
    pub fn compile_cached(graph: TeamGraph) -> CachedProgram {
        let mut compiled = Lowerer::compile(graph);
        // O1: ConstFold → RelayRemoval → CSE → Fusion → RegAlloc.
        let pm = PassManager::o1();
        pm.run_all(&mut compiled.settle);
        pm.run_all(&mut compiled.controllers);
        CachedProgram {
            program: Arc::new(ProgramBuilder.pack(&compiled)),
            vars: compiled.vars,
            apis: compiled.apis,
        }
    }

    /// Spawn a fresh brain from a cached program — does **not** re-lower/re-opt.
    pub fn from_cached(cached: CachedProgram) -> Self {
        Self::from_program(cached.program, cached.vars, cached.apis)
    }

    /// Spawn a fresh brain from an already-packed program — own persistent_vars + Interpreter.
    pub fn from_program(
        program: Arc<RuntimeProgram>,
        vars: VariableTable,
        apis: ApiSlotTable,
    ) -> Self {
        let persistent_vars = vec![VmValue::Null; program.variable_count as usize];
        Self {
            program,
            vars,
            apis,
            persistent_vars,
            backend: Interpreter::default(),
            trace: None,
        }
    }

    pub fn with_trace(mut self) -> Self {
        self.trace = Some(ObservableTrace::empty());
        self
    }

    pub fn take_trace(&mut self) -> Option<ObservableTrace> {
        self.trace.take()
    }
}

impl TeamBrain for RuntimeBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut ctx = ExecutionContext::new(
            api.clone(),
            self.vars.len(),
            self.program.register_count as usize,
        );
        ctx.init_api_slots(&self.apis);
        ctx.var_names = Arc::from(self.program.var_names.clone());
        ctx.set_variable_sids = Arc::from(self.program.set_variable_sids.clone());
        ctx.state.vars.clone_from(&self.persistent_vars);
        if self.trace.is_some() {
            ctx.trace = Some(ObservableTrace::empty());
        }

        for pass in 0..8 {
            ctx.begin_pass(pass);
            self.backend.execute_settle(&self.program, &mut ctx);
        }

        ctx.begin_pass(8);
        self.backend.execute_controllers(&self.program, &mut ctx);
        self.persistent_vars.clone_from(&ctx.state.vars);

        if let Some(mut trace) = ctx.trace.take() {
            trace.controllers = ctx.output.clone();
            self.trace = Some(trace);
        }

        ctx.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        index_graph, GraphBrain, RawConnection, RawGraph, RawNode, RawPort, TeamGraph,
    };

    fn port(id: &str, sid: &str, pol: i32, node: &str) -> RawPort {
        RawPort {
            id: id.into(),
            sid: sid.into(),
            polarity: pol,
            node_sid: node.into(),
        }
    }

    fn node(id: &str, sid: &str, modifier: &str, ports: Vec<RawPort>) -> RawNode {
        RawNode {
            id: id.into(),
            sid: sid.into(),
            modifier: serde_json::json!(modifier),
            owner_function_sid: String::new(),
            ports,
        }
    }

    fn empty_api() -> TeamApi {
        TeamApi {
            team: crate::brain::TeamId::Home,
            bools: Default::default(),
            floats: Default::default(),
            transforms: Default::default(),
            vectors: Default::default(),
        }
    }

    fn power_controller_graph() -> TeamGraph {
        let raw = RawGraph {
            nodes: vec![
                node("Float", "f2", "2", vec![port("Float1", "f2o", 1, "f2")]),
                node("Float", "f3", "3", vec![port("Float1", "f3o", 1, "f3")]),
                node(
                    "Power",
                    "pow",
                    "",
                    vec![
                        port("Float1", "pow_b", 0, "pow"),
                        port("Float2", "pow_e", 0, "pow"),
                        port("Float1", "pow_o", 1, "pow"),
                    ],
                ),
                node("Float", "z0", "0", vec![port("Float1", "z0o", 1, "z0")]),
                node(
                    "ConstructVector3",
                    "cv",
                    "",
                    vec![
                        port("Vector31", "cvo", 1, "cv"),
                        port("Float1", "cvx", 0, "cv"),
                        port("Float2", "cvy", 0, "cv"),
                        port("Float3", "cvz", 0, "cv"),
                    ],
                ),
                node("Bool", "bf", "1", vec![port("Bool1", "bfo", 1, "bf")]),
                node(
                    "SoccerController1",
                    "c1",
                    "",
                    vec![
                        port("Vector31", "c1m", 0, "c1"),
                        port("Bool1", "c1s", 0, "c1"),
                        port("Bool2", "c1i", 0, "c1"),
                    ],
                ),
            ],
            connections: vec![
                RawConnection {
                    port0: "f2o".into(),
                    port1: "pow_b".into(),
                },
                RawConnection {
                    port0: "f3o".into(),
                    port1: "pow_e".into(),
                },
                RawConnection {
                    port0: "pow_o".into(),
                    port1: "cvx".into(),
                },
                RawConnection {
                    port0: "z0o".into(),
                    port1: "cvy".into(),
                },
                RawConnection {
                    port0: "z0o".into(),
                    port1: "cvz".into(),
                },
                RawConnection {
                    port0: "cvo".into(),
                    port1: "c1m".into(),
                },
                RawConnection {
                    port0: "bfo".into(),
                    port1: "c1s".into(),
                },
                RawConnection {
                    port0: "bfo".into(),
                    port1: "c1i".into(),
                },
            ],
        };
        index_graph(raw, "test".into())
    }

    #[test]
    fn runtime_brain_matches_graph_brain_power() {
        let graph = power_controller_graph();
        let api = empty_api();

        let mut graph_brain = GraphBrain::new(graph.clone());
        let graph_out = graph_brain.think(&api);

        let mut vm_brain = RuntimeBrain::compile(graph).with_trace();
        let vm_out = vm_brain.think(&api);

        assert_eq!(graph_out, vm_out);
        assert!((vm_out.commands[0].move_to.x - 8.0).abs() < 1e-4);
    }

    #[test]
    fn runtime_brain_set_variable_settle() {
        let raw = RawGraph {
            nodes: vec![
                node("Float", "fx", "5", vec![port("Float1", "fxo", 1, "fx")]),
                node("Float", "fz", "2", vec![port("Float1", "fzo", 1, "fz")]),
                node("Float", "fy", "0", vec![port("Float1", "fyo", 1, "fy")]),
                node(
                    "ConstructVector3",
                    "cv",
                    "",
                    vec![
                        port("Vector31", "cvo", 1, "cv"),
                        port("Float1", "cvx", 0, "cv"),
                        port("Float2", "cvy", 0, "cv"),
                        port("Float3", "cvz", 0, "cv"),
                    ],
                ),
                node(
                    "SetVariable",
                    "sv",
                    "Target",
                    vec![port("Any1", "svi", 0, "sv")],
                ),
                node(
                    "GetVariable",
                    "gv",
                    "Target",
                    vec![port("Any1", "gvo", 1, "gv")],
                ),
                node("Bool", "bf", "1", vec![port("Bool1", "bfo", 1, "bf")]),
                node(
                    "SoccerController1",
                    "c1",
                    "",
                    vec![
                        port("Vector31", "c1m", 0, "c1"),
                        port("Bool1", "c1s", 0, "c1"),
                        port("Bool2", "c1i", 0, "c1"),
                    ],
                ),
            ],
            connections: vec![
                RawConnection {
                    port0: "fxo".into(),
                    port1: "cvx".into(),
                },
                RawConnection {
                    port0: "fyo".into(),
                    port1: "cvy".into(),
                },
                RawConnection {
                    port0: "fzo".into(),
                    port1: "cvz".into(),
                },
                RawConnection {
                    port0: "cvo".into(),
                    port1: "svi".into(),
                },
                RawConnection {
                    port0: "gvo".into(),
                    port1: "c1m".into(),
                },
                RawConnection {
                    port0: "bfo".into(),
                    port1: "c1s".into(),
                },
                RawConnection {
                    port0: "bfo".into(),
                    port1: "c1i".into(),
                },
            ],
        };
        let graph = index_graph(raw, "test".into());
        let api = empty_api();

        let mut graph_brain = GraphBrain::new(graph.clone());
        let graph_out = graph_brain.think(&api);

        let mut vm_brain = RuntimeBrain::compile(graph);
        let vm_out = vm_brain.think(&api);

        assert_eq!(graph_out, vm_out);
        assert!((vm_out.commands[0].move_to.x - 5.0).abs() < 1e-4);
        assert!((vm_out.commands[0].move_to.y - 2.0).abs() < 1e-4);
    }

    /// O0 acceptance smoke: AIA GraphBrain ≡ RuntimeBrain controllers over many ticks.
    /// Skips when Unity saves `AIA.txt` is absent on this machine.
    #[test]
    fn runtime_aia_matches_graph_brain_controllers() {
        use crate::brain::{IdleBrain, TeamId};
        use crate::params::SimParams;
        use crate::world::{MatchWorld, FIXED_DT};
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        let graph = crate::graph::load_team_graph(&path).expect("load AIA.txt");
        let mut graph_brain = GraphBrain::new(graph.clone());
        let mut vm_brain = RuntimeBrain::compile(graph).with_trace();
        let mut away = IdleBrain;
        let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);

        const TICKS: u64 = 40;
        const EPS: f32 = 1e-4;
        let mut last_vm = BrainOutput::default();

        for tick in 0..TICKS {
            let (home_api, away_api) = world.build_apis();
            let graph_out = graph_brain.think(&home_api);
            let vm_out = vm_brain.think(&home_api);
            assert_controllers_match(tick, &graph_out, &vm_out, EPS);
            last_vm = vm_out.clone();

            let away_out = away.think(&away_api);
            // Advance with GraphBrain commands; both brains saw the same snapshot.
            world.step_with_commands(&graph_out, &away_out, FIXED_DT);
        }

        // RuntimeBrain TRACE capture (Pass 1–8 var commits).
        let trace = vm_brain
            .take_trace()
            .expect("with_trace should leave ObservableTrace after think");
        assert_controllers_match(TICKS - 1, &last_vm, &trace.controllers, EPS);
        let commit_count: usize = trace.passes.iter().map(|p| p.len()).sum();
        assert!(
            commit_count > 0,
            "RuntimeBrain TRACE should record SetVariable commits across settle passes"
        );
    }

    /// Full TRACE identity: GraphBrain ≡ RuntimeBrain Pass 1–8 commits + controllers.
    #[test]
    fn runtime_aia_trace_matches_graph_brain() {
        use crate::brain::{IdleBrain, TeamId};
        use crate::graph_vm::compare_traces;
        use crate::params::SimParams;
        use crate::world::{MatchWorld, FIXED_DT};
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        let graph = crate::graph::load_team_graph(&path).expect("load AIA.txt");
        let mut graph_brain = GraphBrain::new(graph.clone()).with_trace();
        let mut vm_brain = RuntimeBrain::compile(graph).with_trace();
        let mut away = IdleBrain;
        let mut world = MatchWorld::new_kickoff_opening(SimParams::default(), TeamId::Home);

        const TICKS: u64 = 16;

        for tick in 0..TICKS {
            let (home_api, away_api) = world.build_apis();
            let graph_out = graph_brain.think(&home_api);
            let vm_out = vm_brain.think(&home_api);
            assert_controllers_match(tick, &graph_out, &vm_out, 1e-4);

            let g_trace = graph_brain.take_trace().expect("GraphBrain with_trace");
            let r_trace = vm_brain.take_trace().expect("RuntimeBrain with_trace");
            if let Some(mismatch) = compare_traces(tick, &g_trace, &r_trace) {
                panic!("{mismatch}");
            }
            // take_trace clears the Option; re-arm for the next tick.
            graph_brain = graph_brain.with_trace();
            vm_brain = vm_brain.with_trace();

            let away_out = away.think(&away_api);
            world.step_with_commands(&graph_out, &away_out, FIXED_DT);
        }
    }

    fn assert_controllers_match(tick: u64, expected: &BrainOutput, actual: &BrainOutput, eps: f32) {
        for i in 0..4 {
            let e = &expected.commands[i];
            let a = &actual.commands[i];
            let move_ok = (e.move_to.x - a.move_to.x).abs() <= eps
                && (e.move_to.y - a.move_to.y).abs() <= eps;
            if e.sprint != a.sprint || e.interact != a.interact || !move_ok {
                panic!(
                    "AIA controller mismatch at tick {tick}, controller {}\n  \
                     expected: move_to={:?} sprint={} interact={}\n  \
                     actual:   move_to={:?} sprint={} interact={}\n  \
                     deltas:   dx={:.6} dy={:.6} (eps={eps})",
                    i + 1,
                    e.move_to,
                    e.sprint,
                    e.interact,
                    a.move_to,
                    a.sprint,
                    a.interact,
                    (e.move_to.x - a.move_to.x).abs(),
                    (e.move_to.y - a.move_to.y).abs(),
                );
            }
        }
    }
}
