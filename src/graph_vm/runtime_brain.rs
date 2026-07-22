//! RuntimeBrain — compiled TeamGraph via O0 Graph VM.

use std::sync::Arc;

use crate::api::TeamApi;
use crate::brain::{BrainOutput, TeamBrain};
use crate::graph::TeamGraph;
use crate::graph_vm::builder::ProgramBuilder;
use crate::graph_vm::context::ExecutionContext;
use crate::graph_vm::interpreter::Interpreter;
use crate::graph_vm::lower::{ApiSlotTable, Lowerer, VariableTable};
use crate::graph_vm::program::{Backend, RuntimeProgram};
use crate::graph_vm::trace::ObservableTrace;
use crate::graph_vm::value::VmValue;

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
        let compiled = Lowerer::compile(graph);
        let vars = compiled.vars.clone();
        let apis = compiled.apis.clone();
        let program = Arc::new(ProgramBuilder.pack(&compiled));
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
}
