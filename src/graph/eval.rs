//! Per-tick graph evaluation → SoccerController outputs.

use std::collections::HashMap;

use bevy::prelude::Vec2;

use crate::api::TeamApi;
use crate::brain::{BrainCommand, BrainOutput, TeamBrain};

use super::load::TeamGraph;
use super::value::GraphValue;

/// Team brain that evaluates a loaded AIComp `.txt` graph each tick.
#[derive(Debug, Clone)]
pub struct GraphBrain {
    pub graph: TeamGraph,
}

impl GraphBrain {
    pub fn new(graph: TeamGraph) -> Self {
        Self { graph }
    }
}

impl TeamBrain for GraphBrain {
    fn think(&mut self, api: &TeamApi) -> BrainOutput {
        let mut ctx = EvalCtx {
            graph: &self.graph,
            api,
            cache: HashMap::new(),
        };
        let mut out = BrainOutput::default();
        for (i, ctrl_sid) in self.graph.controllers.iter().enumerate() {
            let Some(sid) = ctrl_sid else {
                continue;
            };
            out.commands[i] = ctx.eval_controller(sid);
        }
        out
    }
}

struct EvalCtx<'a> {
    graph: &'a TeamGraph,
    api: &'a TeamApi,
    /// Memo: output port_sid → value
    cache: HashMap<String, GraphValue>,
}

impl<'a> EvalCtx<'a> {
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

    fn eval_port(&mut self, port_sid: &str) -> GraphValue {
        if let Some(v) = self.cache.get(port_sid) {
            return v.clone();
        }
        let Some(pref) = self.graph.ports.get(port_sid).cloned() else {
            return GraphValue::Null;
        };
        let value = self.eval_node_output(&pref.node_sid, &pref.port_name);
        self.cache.insert(port_sid.to_string(), value.clone());
        value
    }

    fn eval_node_output(&mut self, node_sid: &str, port_name: &str) -> GraphValue {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return GraphValue::Null;
        };

        match node.id.as_str() {
            "Float" => GraphValue::Float(parse_float(&node.modifier)),
            "Bool" => {
                // AIComp: modifier "0" = True, "1" = False
                GraphValue::Bool(node.modifier.trim() != "1")
            }
            "String" => GraphValue::String(node.modifier.clone()),

            "SoccerGetBool" => GraphValue::Bool(
                self.api
                    .get_bool(&node.modifier)
                    .unwrap_or(false),
            ),
            "SoccerGetFloat" => GraphValue::Float(
                self.api
                    .get_float(&node.modifier)
                    .unwrap_or(0.0),
            ),
            "SoccerGetTransform" => GraphValue::Transform(
                self.api
                    .get_transform(&node.modifier)
                    .unwrap_or(Vec2::ZERO),
            ),
            "SoccerGetVector3" => match self.api.get_vector3(&node.modifier) {
                Some(Some(v)) => GraphValue::Vec(v),
                _ => GraphValue::Null,
            },

            "RelativePosition" => {
                // MVP: Self / empty → world position of input transform.
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

            "AddFloats" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(a + b)
            }
            "SubtractFloats" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(a - b)
            }
            "MultiplyFloats" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(a * b)
            }
            "DivideFloats" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(if b.abs() < 1e-12 { 0.0 } else { a / b })
            }
            "Power" => {
                // Float1 = base, Float2 = exponent (library Power ports)
                let base = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let exp = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(base.powf(exp))
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
            "Modulo" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(if b.abs() < 1e-12 { 0.0 } else { a % b })
            }
            "Lerp" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                let t = self.input_named(node_sid, "Float3").map(|v| v.as_float()).unwrap_or(0.0);
                GraphValue::Float(a + (b - a) * t)
            }

            "AddVector3" => {
                let a = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let b = self.input_named(node_sid, "Vector32").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                GraphValue::Vec(a + b)
            }
            "SubtractVector3" => {
                let a = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let b = self.input_named(node_sid, "Vector32").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                GraphValue::Vec(a - b)
            }
            "ScaleVector3" => {
                let v = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let s = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(1.0);
                GraphValue::Vec(v * s)
            }
            "Normalize" => {
                let v = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let len = v.length();
                GraphValue::Vec(if len > 1e-8 { v / len } else { Vec2::ZERO })
            }
            "Magnitude" => {
                let v = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                GraphValue::Float(v.length())
            }
            "Distance" => {
                let a = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let b = self.input_named(node_sid, "Vector32").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                GraphValue::Float(a.distance(b))
            }

            "Not" => {
                let b = self.input_named(node_sid, "Bool1").map(|v| v.as_bool()).unwrap_or(false);
                GraphValue::Bool(!b)
            }
            "CompareFloats" => {
                let a = self.input_named(node_sid, "Float1").map(|v| v.as_float()).unwrap_or(0.0);
                let b = self.input_named(node_sid, "Float2").map(|v| v.as_float()).unwrap_or(0.0);
                // modifier index: == < > <= >=
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
                let a = self.input_named(node_sid, "Bool1").map(|v| v.as_bool()).unwrap_or(false);
                let b = self.input_named(node_sid, "Bool2").map(|v| v.as_bool()).unwrap_or(false);
                // 0=and 1=or 2=equal 3=xor 4=nor 5=nand 6=xnor
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
                let cond = self.input_named(node_sid, "Bool1").map(|v| v.as_bool()).unwrap_or(false);
                let t = self.input_named(node_sid, "Bool2").map(|v| v.as_bool()).unwrap_or(false);
                let f = self.input_named(node_sid, "Bool3").map(|v| v.as_bool()).unwrap_or(false);
                GraphValue::Bool(if cond { t } else { f })
            }
            "ConditionalSetFloatV2" | "ConditionalSetFloat" => {
                // Ports: Bool1 cond, Float1 then, Float2 else, Float1 out
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
                let cond = self.input_named(node_sid, "Bool1").map(|v| v.as_bool()).unwrap_or(false);
                // true branch Vector31 in, false Vector32 in (output also Vector31)
                let t = self.input_named(node_sid, "Vector31").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                let f = self.input_named(node_sid, "Vector32").map(|v| v.as_vec()).unwrap_or(Vec2::ZERO);
                GraphValue::Vec(if cond { t } else { f })
            }

            "IsNull" => {
                let v = self.input_named(node_sid, "Any1").or_else(|| {
                    self.input_named(node_sid, "Vector31")
                });
                GraphValue::Bool(matches!(v, None | Some(GraphValue::Null)))
            }

            // Debug / plot / region / sensors — no-ops that don't feed controllers in MVP
            "Debug" | "DebugDrawDisc" | "DebugDrawLine" | "TimePlot" | "Region"
            | "ConstructSoccerProperties" | "Spherecast" => GraphValue::Null,

            other if other.starts_with("SoccerPlayerSensors") => GraphValue::Null,
            other if other.starts_with("SoccerController") => GraphValue::Null,

            other => {
                // Unknown node: try to forward first connected input if any
                let _ = (other, port_name);
                GraphValue::Null
            }
        }
    }
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

    #[test]
    fn float_power_to_controller_move_x() {
        // Float(2) Power Float(3) → ConstructVector3(x=8,y=0,z=0) → Controller1.moveTo
        let raw = RawGraph {
            nodes: vec![
                RawNode {
                    id: "Float".into(),
                    sid: "f2".into(),
                    modifier: serde_json::json!("2"),
                    ports: vec![port("Float1", "f2o", 1, "f2")],
                },
                RawNode {
                    id: "Float".into(),
                    sid: "f3".into(),
                    modifier: serde_json::json!("3"),
                    ports: vec![port("Float1", "f3o", 1, "f3")],
                },
                RawNode {
                    id: "Power".into(),
                    sid: "pow".into(),
                    modifier: serde_json::json!(""),
                    ports: vec![
                        port("Float1", "pow_b", 0, "pow"),
                        port("Float2", "pow_e", 0, "pow"),
                        port("Float1", "pow_o", 1, "pow"),
                    ],
                },
                RawNode {
                    id: "Float".into(),
                    sid: "z0".into(),
                    modifier: serde_json::json!("0"),
                    ports: vec![port("Float1", "z0o", 1, "z0")],
                },
                RawNode {
                    id: "ConstructVector3".into(),
                    sid: "cv".into(),
                    modifier: serde_json::json!(""),
                    ports: vec![
                        port("Vector31", "cvo", 1, "cv"),
                        port("Float1", "cvx", 0, "cv"),
                        port("Float2", "cvy", 0, "cv"),
                        port("Float3", "cvz", 0, "cv"),
                    ],
                },
                RawNode {
                    id: "Bool".into(),
                    sid: "bf".into(),
                    modifier: serde_json::json!("1"), // False
                    ports: vec![port("Bool1", "bfo", 1, "bf")],
                },
                RawNode {
                    id: "SoccerController1".into(),
                    sid: "c1".into(),
                    modifier: serde_json::json!(""),
                    ports: vec![
                        port("Vector31", "c1m", 0, "c1"),
                        port("Bool1", "c1s", 0, "c1"),
                        port("Bool2", "c1i", 0, "c1"),
                    ],
                },
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
        let g = index_graph(raw, "test".into());
        let mut brain = GraphBrain::new(g);
        let api = TeamApi {
            team: crate::brain::TeamId::Home,
            bools: Default::default(),
            floats: Default::default(),
            transforms: Default::default(),
            vectors: Default::default(),
        };
        let out = brain.think(&api);
        assert!((out.commands[0].move_to.x - 8.0).abs() < 1e-4);
        assert!(!out.commands[0].sprint);
    }
}
