//! Per-tick graph evaluation → SoccerController outputs.
//!
//! Supports constants, math, SoccerGet*, RelativePosition, conditionals,
//! SetVariable/GetVariable, and CreateFunction/Function call frames.

use std::collections::HashMap;

use bevy::prelude::Vec2;

use crate::api::TeamApi;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};
use crate::graph_vm::trace::{ObservableTrace, VarCommit};
use crate::graph_vm::value::VmValue;

use super::load::TeamGraph;
use super::value::GraphValue;

/// Team brain that evaluates a loaded AIComp `.txt` graph each tick.
#[derive(Debug, Clone)]
pub struct GraphBrain {
    pub graph: TeamGraph,
    /// Last committed variable values (also used mid-tick during multi-pass).
    pub vars: HashMap<String, GraphValue>,
    /// Optional TRACE for GraphBrain ≡ RuntimeBrain acceptance.
    trace: Option<ObservableTrace>,
}

impl GraphBrain {
    pub fn new(graph: TeamGraph) -> Self {
        Self {
            graph,
            vars: HashMap::new(),
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

impl TeamBrain for GraphBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let set_vars = self.graph.set_variables.clone();
        let controllers = self.graph.controllers.clone();
        let tracing = self.trace.is_some();

        let mut ctx = EvalCtx {
            graph: &self.graph,
            api,
            vars: &mut self.vars,
            cache: HashMap::new(),
            call_stack: Vec::new(),
            trace: if tracing {
                Some(ObservableTrace::empty())
            } else {
                None
            },
            current_pass: None,
        };

        // Multi-pass so SetVariables that depend on other GetVariables settle.
        for pass in 0..8 {
            ctx.current_pass = Some(pass);
            for sid in &set_vars {
                ctx.commit_set_variable(sid);
            }
            // Clear port cache between passes so GetVariable sees fresh vars.
            ctx.cache.clear();
        }
        // Controller eval may still hit SetVariable sinks; do not TRACE those.
        ctx.current_pass = None;

        let mut out = BrainOutput::default();
        for (i, ctrl_sid) in controllers.iter().enumerate() {
            let Some(sid) = ctrl_sid else {
                continue;
            };
            out.commands[i] = ctx.eval_controller(sid);
        }

        if let Some(mut trace) = ctx.trace.take() {
            trace.controllers = out.clone();
            self.trace = Some(trace);
        }

        out
    }
}

struct CallFrame {
    create_sid: String,
    args: [GraphValue; 4],
    cache: HashMap<String, GraphValue>,
}

struct EvalCtx<'a> {
    graph: &'a TeamGraph,
    api: &'a TeamApi,
    vars: &'a mut HashMap<String, GraphValue>,
    cache: HashMap<String, GraphValue>,
    call_stack: Vec<CallFrame>,
    trace: Option<ObservableTrace>,
    /// `Some(0..7)` during settle passes; `None` while evaluating controllers.
    current_pass: Option<usize>,
}

impl<'a> EvalCtx<'a> {
    fn record_var_commit(&mut self, name: &str, value: &GraphValue) {
        let Some(pass) = self.current_pass else {
            return;
        };
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        if pass < 8 {
            trace.passes[pass].push(VarCommit {
                name: name.to_string(),
                value: VmValue::from_graph(value),
            });
        }
    }

    fn commit_set_variable(&mut self, node_sid: &str) {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return;
        };
        if node.id != "SetVariable" {
            return;
        }
        let value = self
            .input_named(node_sid, "Any1")
            .unwrap_or(GraphValue::Null);
        self.record_var_commit(&node.modifier, &value);
        self.vars.insert(node.modifier, value);
    }

    fn eval_controller(&mut self, node_sid: &str) -> BrainCommand {
        let move_to = self
            .input_named(node_sid, "Vector31")
            .map(|v| v.as_vec())
            .unwrap_or(Vec2::ZERO);
        let sprint = self
            .input_named(node_sid, "Bool1")
            .map(|v| v.as_bool())
            .unwrap_or(false);
        let interact = self
            .input_named(node_sid, "Bool2")
            .map(|v| v.as_bool())
            .unwrap_or(false);
        BrainCommand {
            move_to,
            sprint,
            interact,
        }
    }

    fn input_named(&mut self, node_sid: &str, port_name: &str) -> Option<GraphValue> {
        let in_sid = self.graph.input_port_sid(node_sid, port_name)?;
        let src_out = self.graph.input_source.get(&in_sid)?.clone();
        Some(self.eval_port(&src_out))
    }

    fn cache_get(&self, port_sid: &str) -> Option<GraphValue> {
        if let Some(frame) = self.call_stack.last() {
            frame.cache.get(port_sid).cloned()
        } else {
            self.cache.get(port_sid).cloned()
        }
    }

    fn cache_set(&mut self, port_sid: String, value: GraphValue) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.cache.insert(port_sid, value);
        } else {
            self.cache.insert(port_sid, value);
        }
    }

    fn eval_port(&mut self, port_sid: &str) -> GraphValue {
        if let Some(v) = self.cache_get(port_sid) {
            return v;
        }
        let Some(pref) = self.graph.ports.get(port_sid).cloned() else {
            return GraphValue::Null;
        };
        let value = self.eval_node_output(&pref.node_sid, &pref.port_name);
        self.cache_set(port_sid.to_string(), value.clone());
        value
    }

    fn eval_node_output(&mut self, node_sid: &str, port_name: &str) -> GraphValue {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return GraphValue::Null;
        };

        // CreateFunction param outputs while inside a matching call frame.
        if node.id == "CreateFunction" {
            if let Some(frame) = self.call_stack.last() {
                if frame.create_sid == node.sid {
                    let idx = match port_name {
                        "Any1" => 0,
                        "Any2" => 1,
                        "Any3" => 2,
                        "Any4" => 3,
                        _ => return GraphValue::Null,
                    };
                    // Only polarity-out params (CreateFunction Any1-4 out).
                    return frame.args[idx].clone();
                }
            }
            return GraphValue::Null;
        }

        match node.id.as_str() {
            "Float" => GraphValue::Float(parse_float(&node.modifier)),
            "Bool" => GraphValue::Bool(node.modifier.trim() != "1"),
            "String" => GraphValue::String(node.modifier.clone()),

            "GetVariable" => self
                .vars
                .get(&node.modifier)
                .cloned()
                .unwrap_or(GraphValue::Null),
            "SetVariable" => {
                // Side-effect sink; if something reads it, return stored value.
                // Nested commits during settle must TRACE like VM StoreVar deps.
                let value = self
                    .input_named(node_sid, "Any1")
                    .unwrap_or(GraphValue::Null);
                self.record_var_commit(&node.modifier, &value);
                self.vars.insert(node.modifier.clone(), value.clone());
                value
            }

            "Function" => self.eval_function_call(&node),

            "SoccerGetBool" => GraphValue::Bool(self.api.get_bool(&node.modifier).unwrap_or(false)),
            "SoccerGetFloat" => {
                GraphValue::Float(self.api.get_float(&node.modifier).unwrap_or(0.0))
            }
            "SoccerGetTransform" => {
                GraphValue::Transform(self.api.get_transform(&node.modifier).unwrap_or(Vec2::ZERO))
            }
            "SoccerGetVector3" => match self.api.get_vector3(&node.modifier) {
                Some(Some(v)) => GraphValue::Vec(v),
                _ => GraphValue::Null,
            },

            "RelativePosition" => {
                let pos = self
                    .input_named(node_sid, "Transform1")
                    .map(|v| v.as_transform_pos())
                    .unwrap_or(Vec2::ZERO);
                GraphValue::Vec(pos)
            }

            "ConstructVector3" => {
                let x = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let _y = self
                    .input_named(node_sid, "Float2")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let z = self
                    .input_named(node_sid, "Float3")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                GraphValue::Vec(Vec2::new(x, z))
            }

            "Vector3Split" => {
                let v = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                match port_name {
                    "Float1" => GraphValue::Float(v.x),
                    "Float2" => GraphValue::Float(0.0), // Unity Y unused in 2D
                    "Float3" => GraphValue::Float(v.y),
                    _ => GraphValue::Null,
                }
            }

            "AddFloats" => bin_f(self, node_sid, |a, b| a + b),
            "SubtractFloats" => bin_f(self, node_sid, |a, b| a - b),
            "MultiplyFloats" => bin_f(self, node_sid, |a, b| a * b),
            "DivideFloats" => bin_f(
                self,
                node_sid,
                |a, b| {
                    if b.abs() < 1e-12 {
                        0.0
                    } else {
                        a / b
                    }
                },
            ),
            "Power" => bin_f(self, node_sid, |a, b| a.powf(b)),
            "Modulo" => bin_f(
                self,
                node_sid,
                |a, b| {
                    if b.abs() < 1e-12 {
                        0.0
                    } else {
                        a % b
                    }
                },
            ),
            "Lerp" => {
                let a = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let b = self
                    .input_named(node_sid, "Float2")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let t = self
                    .input_named(node_sid, "Float3")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                GraphValue::Float(a + (b - a) * t)
            }

            "Relay" => self
                .input_named(node_sid, "Any1")
                .unwrap_or(GraphValue::Null),

            "Operation" => {
                let a = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let op = node.modifier.to_ascii_lowercase();
                let r = match op.as_str() {
                    "abs" => a.abs(),
                    "round" => a.round(),
                    "floor" => a.floor(),
                    "ceil" => a.ceil(),
                    "sign" | "signum" => {
                        if a > 0.0 {
                            1.0
                        } else if a < 0.0 {
                            -1.0
                        } else {
                            0.0
                        }
                    }
                    "sqrt" => a.max(0.0).sqrt(),
                    "sin" => a.sin(),
                    "cos" => a.cos(),
                    "tan" => a.tan(),
                    "ln" => a.ln(),
                    "log10" => a.log10(),
                    "e^" => a.exp(),
                    "10^" => 10f32.powf(a),
                    _ => a,
                };
                GraphValue::Float(r)
            }

            "AbsFloat" | "Absolute" => {
                let a = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                GraphValue::Float(a.abs())
            }

            "AddVector3" => bin_v(self, node_sid, |a, b| a + b),
            "SubtractVector3" => bin_v(self, node_sid, |a, b| a - b),
            "ScaleVector3" => {
                let v = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                let s = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(1.0);
                GraphValue::Vec(v * s)
            }
            "Normalize" => {
                let v = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                let len = v.length();
                GraphValue::Vec(if len > 1e-8 { v / len } else { Vec2::ZERO })
            }
            "Magnitude" => {
                let v = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                GraphValue::Float(v.length())
            }
            "Distance" => {
                let a = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                let b = self
                    .input_named(node_sid, "Vector32")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                GraphValue::Float(a.distance(b))
            }
            "DotProduct" => {
                let a = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                let b = self
                    .input_named(node_sid, "Vector32")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                GraphValue::Float(a.dot(b))
            }

            "Not" => {
                let b = self
                    .input_named(node_sid, "Bool1")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                GraphValue::Bool(!b)
            }
            "CompareFloats" => {
                let a = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let b = self
                    .input_named(node_sid, "Float2")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let op = node.modifier.parse::<i32>().unwrap_or(0);
                let r = match op {
                    1 => a < b,
                    2 => a > b,
                    3 => a <= b,
                    4 => a >= b,
                    _ => (a - b).abs() < 1e-5,
                };
                GraphValue::Bool(r)
            }
            "CompareBool" => {
                let a = self
                    .input_named(node_sid, "Bool1")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let b = self
                    .input_named(node_sid, "Bool2")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let op = node.modifier.parse::<i32>().unwrap_or(0);
                let r = match op {
                    1 => a || b,
                    2 => a == b,
                    3 => a ^ b,
                    4 => !(a || b),
                    5 => !(a && b),
                    6 => a == b,
                    _ => a && b,
                };
                GraphValue::Bool(r)
            }
            "ConditionalSetBool" => {
                let cond = self
                    .input_named(node_sid, "Bool1")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let t = self
                    .input_named(node_sid, "Bool2")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let f = self
                    .input_named(node_sid, "Bool3")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                GraphValue::Bool(if cond { t } else { f })
            }
            "ConditionalSetFloatV2" | "ConditionalSetFloat" => {
                let cond = self
                    .input_named(node_sid, "Bool1")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let t = self
                    .input_named(node_sid, "Float1")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                let f = self
                    .input_named(node_sid, "Float2")
                    .map(|v| v.as_float())
                    .unwrap_or(0.0);
                GraphValue::Float(if cond { t } else { f })
            }
            "ConditionalSetVector3" => {
                let cond = self
                    .input_named(node_sid, "Bool1")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let t = self
                    .input_named(node_sid, "Vector31")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                let f = self
                    .input_named(node_sid, "Vector32")
                    .map(|v| v.as_vec())
                    .unwrap_or(Vec2::ZERO);
                GraphValue::Vec(if cond { t } else { f })
            }

            "IsNull" => {
                let v = self
                    .input_named(node_sid, "Any1")
                    .or_else(|| self.input_named(node_sid, "Vector31"));
                GraphValue::Bool(matches!(v, None | Some(GraphValue::Null)))
            }

            "Debug"
            | "DebugDrawDisc"
            | "DebugDrawLine"
            | "TimePlot"
            | "Region"
            | "ConstructSoccerProperties"
            | "Spherecast"
            | "Color"
            | "Country"
            | "Stat" => GraphValue::Null,

            other if other.starts_with("SoccerPlayerSensors") => GraphValue::Null,
            other if other.starts_with("SoccerController") => GraphValue::Null,

            other => {
                let _ = (other, port_name);
                GraphValue::Null
            }
        }
    }

    fn eval_function_call(&mut self, fn_node: &super::load::GraphNode) -> GraphValue {
        let Some(def) = self.graph.create_functions.get(&fn_node.modifier).cloned() else {
            return GraphValue::Null;
        };

        let args = [
            self.input_named(&fn_node.sid, "Any1")
                .unwrap_or(GraphValue::Null),
            self.input_named(&fn_node.sid, "Any2")
                .unwrap_or(GraphValue::Null),
            self.input_named(&fn_node.sid, "Any3")
                .unwrap_or(GraphValue::Null),
            self.input_named(&fn_node.sid, "Any4")
                .unwrap_or(GraphValue::Null),
        ];

        self.call_stack.push(CallFrame {
            create_sid: def.sid.clone(),
            args,
            cache: HashMap::new(),
        });

        // Return is CreateFunction Any1 polarity 0.
        let ret = self
            .input_named(&def.sid, "Any1")
            .unwrap_or(GraphValue::Null);

        self.call_stack.pop();
        ret
    }
}

fn bin_f(ctx: &mut EvalCtx<'_>, node_sid: &str, f: impl Fn(f32, f32) -> f32) -> GraphValue {
    let a = ctx
        .input_named(node_sid, "Float1")
        .map(|v| v.as_float())
        .unwrap_or(0.0);
    let b = ctx
        .input_named(node_sid, "Float2")
        .map(|v| v.as_float())
        .unwrap_or(0.0);
    GraphValue::Float(f(a, b))
}

fn bin_v(ctx: &mut EvalCtx<'_>, node_sid: &str, f: impl Fn(Vec2, Vec2) -> Vec2) -> GraphValue {
    let a = ctx
        .input_named(node_sid, "Vector31")
        .map(|v| v.as_vec())
        .unwrap_or(Vec2::ZERO);
    let b = ctx
        .input_named(node_sid, "Vector32")
        .map(|v| v.as_vec())
        .unwrap_or(Vec2::ZERO);
    GraphValue::Vec(f(a, b))
}

fn parse_float(s: &str) -> f32 {
    let t = s.trim().replace(',', ".");
    t.parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::load::index_graph;
    use crate::graph::load::{RawConnection, RawGraph, RawNode, RawPort};

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

    #[test]
    fn float_power_to_controller_move_x() {
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
        let mut brain = GraphBrain::new(index_graph(raw, "test".into()));
        let api = empty_api();
        let out = brain.think(&api);
        assert!((out.commands[0].move_to.x - 8.0).abs() < 1e-4);
    }

    #[test]
    fn set_get_variable_feeds_controller() {
        // SetVariable("Target") = ConstructVector3(5,0,2) → GetVariable → Controller
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
        let mut brain = GraphBrain::new(index_graph(raw, "test".into()));
        let out = brain.think(&empty_api());
        assert!((out.commands[0].move_to.x - 5.0).abs() < 1e-4);
        assert!((out.commands[0].move_to.y - 2.0).abs() < 1e-4);
    }

    #[test]
    fn function_call_returns_scaled_vec() {
        // CreateFunction "Scale2": return ScaleVector3(Param1, 2)
        // Function("Scale2", vec(3,4)) → controller
        let mut scale_node = node(
            "ScaleVector3",
            "sc",
            "",
            vec![
                port("Float1", "scf", 0, "sc"),
                port("Vector31", "scvo", 1, "sc"),
                port("Vector31", "scvi", 0, "sc"),
            ],
        );
        scale_node.owner_function_sid = "cf".into();
        let mut f2 = node("Float", "two", "2", vec![port("Float1", "twoo", 1, "two")]);
        f2.owner_function_sid = "cf".into();

        let raw = RawGraph {
            nodes: vec![
                node(
                    "CreateFunction",
                    "cf",
                    "Scale2",
                    vec![
                        port("Any1", "cfr", 0, "cf"), // return in
                        port("String1", "cfs1", 0, "cf"),
                        port("Any1", "cfp1", 1, "cf"), // param1 out
                        port("Any2", "cfp2", 1, "cf"),
                        port("Any3", "cfp3", 1, "cf"),
                        port("Any4", "cfp4", 1, "cf"),
                    ],
                ),
                f2,
                scale_node,
                node("Float", "vx", "3", vec![port("Float1", "vxo", 1, "vx")]),
                node("Float", "vz", "4", vec![port("Float1", "vzo", 1, "vz")]),
                node("Float", "vy", "0", vec![port("Float1", "vyo", 1, "vy")]),
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
                    "Function",
                    "fn",
                    "Scale2",
                    vec![
                        port("Any1", "fni1", 0, "fn"),
                        port("Any2", "fni2", 0, "fn"),
                        port("Any3", "fni3", 0, "fn"),
                        port("Any4", "fni4", 0, "fn"),
                        port("Any1", "fno", 1, "fn"),
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
                // body: param1 → scale in, float 2 → scale factor, scale out → return
                RawConnection {
                    port0: "cfp1".into(),
                    port1: "scvi".into(),
                },
                RawConnection {
                    port0: "twoo".into(),
                    port1: "scf".into(),
                },
                RawConnection {
                    port0: "scvo".into(),
                    port1: "cfr".into(),
                },
                // call args
                RawConnection {
                    port0: "vxo".into(),
                    port1: "cvx".into(),
                },
                RawConnection {
                    port0: "vyo".into(),
                    port1: "cvy".into(),
                },
                RawConnection {
                    port0: "vzo".into(),
                    port1: "cvz".into(),
                },
                RawConnection {
                    port0: "cvo".into(),
                    port1: "fni1".into(),
                },
                RawConnection {
                    port0: "fno".into(),
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
        let mut brain = GraphBrain::new(index_graph(raw, "test".into()));
        let out = brain.think(&empty_api());
        assert!((out.commands[0].move_to.x - 6.0).abs() < 1e-4);
        assert!((out.commands[0].move_to.y - 8.0).abs() < 1e-4);
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
}
