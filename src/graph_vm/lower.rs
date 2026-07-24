//! Demand-driven 1:1 lowerer — TeamGraph semantics match `graph::eval`.

use std::collections::HashMap;

use bevy::prelude::Vec2;

use crate::graph::load::{GraphNode, TeamGraph};
use crate::graph_vm::context::VariableId;
use crate::graph_vm::ir::{IrInst, LoweredIR, Reg, LOWERED_IR_VERSION};
use crate::graph_vm::opcode::{ApiSlot, OpCode};
use crate::graph_vm::value::RegisterKind;

#[derive(Debug, Clone)]
pub struct VariableTable {
    pub names: Vec<String>,
    pub name_to_id: HashMap<String, VariableId>,
}

impl VariableTable {
    pub fn intern(&mut self, name: &str) -> VariableId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = VariableId(self.names.len() as u16);
        self.names.push(name.to_string());
        self.name_to_id.insert(name.to_string(), id);
        id
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiKind {
    Bool,
    Float,
    Transform,
    Vector3,
}

#[derive(Debug, Clone)]
pub struct ApiSlotTable {
    pub labels: Vec<String>,
    pub kinds: Vec<ApiKind>,
    /// SoccerGet catalog index (`UNKNOWN_ID` if label not in catalog).
    pub dense_ids: Vec<u16>,
    label_to_slot: HashMap<(ApiKind, String), ApiSlot>,
}

impl ApiSlotTable {
    pub fn intern(&mut self, label: &str, kind: ApiKind) -> ApiSlot {
        let key = (kind, label.to_string());
        if let Some(&slot) = self.label_to_slot.get(&key) {
            return slot;
        }
        let dense = match kind {
            ApiKind::Bool => crate::api::bool_index(label).unwrap_or(crate::api::UNKNOWN_ID),
            ApiKind::Float => crate::api::float_index(label).unwrap_or(crate::api::UNKNOWN_ID),
            ApiKind::Transform => {
                crate::api::transform_index(label).unwrap_or(crate::api::UNKNOWN_ID)
            }
            ApiKind::Vector3 => crate::api::vector_index(label).unwrap_or(crate::api::UNKNOWN_ID),
        };
        let idx = self.labels.len();
        self.labels.push(label.to_string());
        self.kinds.push(kind);
        self.dense_ids.push(dense);
        let slot = ApiSlot::new((idx + 1) as u16).expect("api slot");
        self.label_to_slot.insert(key, slot);
        slot
    }

    pub fn label(&self, slot: ApiSlot) -> &str {
        &self.labels[slot.get() as usize - 1]
    }

    pub fn kind(&self, slot: ApiSlot) -> ApiKind {
        self.kinds[slot.get() as usize - 1]
    }

    pub fn dense_id(&self, slot: ApiSlot) -> u16 {
        self.dense_ids
            .get(slot.get() as usize - 1)
            .copied()
            .unwrap_or(crate::api::UNKNOWN_ID)
    }
}

#[derive(Debug, Clone)]
pub struct CompileResult {
    pub settle: LoweredIR,
    pub controllers: LoweredIR,
    pub vars: VariableTable,
    pub apis: ApiSlotTable,
    pub set_variable_sids: Vec<String>,
}

#[derive(Debug, Clone)]
struct CallFrame {
    create_sid: String,
    /// Function call node sid (for resolving Color args through params).
    call_sid: String,
    args: [Reg; 4],
    port_regs: HashMap<String, Reg>,
}

#[derive(Debug)]
pub struct Lowerer {
    graph: TeamGraph,
    next_reg: u32,
    port_regs: HashMap<String, Reg>,
    call_stack: Vec<CallFrame>,
    ir: Vec<IrInst>,
    vars: VariableTable,
    apis: ApiSlotTable,
}

impl Lowerer {
    pub fn compile(graph: TeamGraph) -> CompileResult {
        let set_variable_sids = graph.set_variables.clone();
        let mut lowerer = Self {
            graph,
            next_reg: 0,
            port_regs: HashMap::new(),
            call_stack: Vec::new(),
            ir: Vec::new(),
            vars: VariableTable {
                names: Vec::new(),
                name_to_id: HashMap::new(),
            },
            apis: ApiSlotTable {
                labels: Vec::new(),
                kinds: Vec::new(),
                dense_ids: Vec::new(),
                label_to_slot: HashMap::new(),
            },
        };

        for sid in &set_variable_sids {
            if let Some(node) = lowerer.graph.nodes.get(sid) {
                lowerer.vars.intern(&node.modifier);
            }
        }
        let mut variable_nodes: Vec<_> = lowerer
            .graph
            .nodes
            .values()
            .filter(|node| node.id == "SetVariable" || node.id == "GetVariable")
            .map(|node| (node.sid.clone(), node.modifier.clone()))
            .collect();
        variable_nodes.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, name) in variable_nodes {
            lowerer.vars.intern(&name);
        }

        for sid in &set_variable_sids {
            lowerer.lower_set_variable(sid);
        }
        let settle = lowerer.take_ir();

        lowerer.port_regs.clear();
        // DebugDraw sinks once per think (controllers phase — settle runs 8×).
        let debug_draw_sids = lowerer.graph.debug_draws.clone();
        for sid in &debug_draw_sids {
            lowerer.lower_debug_draw(sid);
        }
        // Root Function side-effects (e.g. AIA Draw Player Debugs) — return unused.
        let root_fns = lowerer.graph.root_functions.clone();
        for sid in &root_fns {
            if let Some(node) = lowerer.graph.nodes.get(sid).cloned() {
                let _ = lowerer.lower_function_call(&node);
            }
        }
        let controllers_by_slot = lowerer.graph.controllers.clone();
        for (i, ctrl_sid) in controllers_by_slot.iter().enumerate() {
            if let Some(sid) = ctrl_sid {
                lowerer.lower_controller(i, sid);
            }
        }
        let controllers = lowerer.take_ir();

        CompileResult {
            settle,
            controllers,
            vars: lowerer.vars,
            apis: lowerer.apis,
            set_variable_sids,
        }
    }

    fn take_ir(&mut self) -> LoweredIR {
        LoweredIR {
            ir_version: LOWERED_IR_VERSION,
            instructions: std::mem::take(&mut self.ir),
            block_succs: Vec::new(),
        }
    }

    fn fresh_reg(&mut self, kind: RegisterKind) -> Reg {
        let r = Reg(self.next_reg);
        self.next_reg += 1;
        let _ = kind;
        r
    }

    #[allow(dead_code)] // O0 helper; settle/controllers push IrInst directly.
    fn emit(&mut self, inst: IrInst) -> Reg {
        let dest = inst.dest;
        self.ir.push(inst);
        dest.expect("emit with dest")
    }

    fn lower_set_variable(&mut self, node_sid: &str) {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return;
        };
        if node.id != "SetVariable" {
            return;
        }
        let value = self
            .lower_input(node_sid, "Any1")
            .unwrap_or_else(|| self.emit_const_null(node_sid, "Any1"));
        let vid = self.vars.intern(&node.modifier);
        self.ir.push(IrInst {
            dest: None,
            kind: RegisterKind::Null,
            op: OpCode::StoreVar,
            args: vec![Reg(vid.0 as u32), value],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: "Any1".to_string(),
        });
    }

    fn lower_controller(&mut self, slot: usize, node_sid: &str) {
        let move_to = self
            .lower_input(node_sid, "Vector31")
            .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
        let sprint = self
            .lower_input(node_sid, "Bool1")
            .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool1", false));
        let interact = self
            .lower_input(node_sid, "Bool2")
            .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool2", false));
        self.ir.push(IrInst {
            dest: None,
            kind: RegisterKind::Null,
            op: OpCode::EmitController,
            args: vec![move_to, sprint, interact],
            immediates: vec![slot as u32],
            source_sid: node_sid.to_string(),
            source_port: "controller".to_string(),
        });
    }

    fn lower_debug_draw(&mut self, node_sid: &str) {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return;
        };
        let rgba = self.resolve_color_rgba(node_sid, "Color1");
        let imm = vec![
            rgba[0].to_bits(),
            rgba[1].to_bits(),
            rgba[2].to_bits(),
            rgba[3].to_bits(),
        ];
        match node.id.as_str() {
            "DebugDrawLine" => {
                let a = self
                    .lower_input(node_sid, "Vector31")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
                let b = self
                    .lower_input(node_sid, "Vector32")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector32", Vec2::ZERO));
                let w = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 1.0));
                self.ir.push(IrInst {
                    dest: None,
                    kind: RegisterKind::Null,
                    op: OpCode::DebugDrawLine,
                    args: vec![a, b, w],
                    immediates: imm,
                    source_sid: node_sid.to_string(),
                    source_port: "DebugDrawLine".to_string(),
                });
            }
            "DebugDrawDisc" => {
                let center = self
                    .lower_input(node_sid, "Vector31")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
                let radius = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 1.0));
                let width = self
                    .lower_input(node_sid, "Float2")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 1.0));
                self.ir.push(IrInst {
                    dest: None,
                    kind: RegisterKind::Null,
                    op: OpCode::DebugDrawDisc,
                    args: vec![center, radius, width],
                    immediates: imm,
                    source_sid: node_sid.to_string(),
                    source_port: "DebugDrawDisc".to_string(),
                });
            }
            _ => {}
        }
    }

    fn resolve_color_rgba(&self, node_sid: &str, port_name: &str) -> [f32; 4] {
        let Some(in_sid) = self.graph.input_port_sid(node_sid, port_name) else {
            return crate::debug_draw::named_rgba("White");
        };
        self.resolve_color_from_port(&in_sid, 0)
    }

    /// Walk Relay / CreateFunction params back to a Color constant.
    fn resolve_color_from_port(&self, port_sid: &str, depth: u32) -> [f32; 4] {
        if depth > 12 {
            return crate::debug_draw::named_rgba("White");
        }
        // If this port is an input/relay, follow its source first.
        if let Some(src_out) = self.graph.input_source.get(port_sid) {
            return self.resolve_color_from_port(src_out, depth + 1);
        }
        let Some(pref) = self.graph.ports.get(port_sid) else {
            return crate::debug_draw::named_rgba("White");
        };
        let Some(node) = self.graph.nodes.get(&pref.node_sid) else {
            return crate::debug_draw::named_rgba("White");
        };
        match node.id.as_str() {
            "Color" => crate::debug_draw::named_rgba(&node.modifier),
            "Relay" => {
                if let Some(in_sid) = self.graph.input_port_sid(&node.sid, "Any1") {
                    self.resolve_color_from_port(&in_sid, depth + 1)
                } else {
                    crate::debug_draw::named_rgba("White")
                }
            }
            "CreateFunction" => {
                let idx = match pref.port_name.as_str() {
                    "Any1" => 0,
                    "Any2" => 1,
                    "Any3" => 2,
                    "Any4" => 3,
                    _ => return crate::debug_draw::named_rgba("White"),
                };
                let Some(frame) = self.call_stack.last() else {
                    return crate::debug_draw::named_rgba("White");
                };
                if frame.create_sid != node.sid {
                    return crate::debug_draw::named_rgba("White");
                }
                let arg_port = ["Any1", "Any2", "Any3", "Any4"][idx];
                if let Some(in_sid) = self.graph.input_port_sid(&frame.call_sid, arg_port) {
                    self.resolve_color_from_port(&in_sid, depth + 1)
                } else {
                    crate::debug_draw::named_rgba("White")
                }
            }
            _ => crate::debug_draw::named_rgba("White"),
        }
    }

    fn lower_input(&mut self, node_sid: &str, port_name: &str) -> Option<Reg> {
        let in_sid = self.graph.input_port_sid(node_sid, port_name)?;
        let src_out = self.graph.input_source.get(&in_sid)?.clone();
        Some(self.lower_port(&src_out))
    }

    fn cache_get(&self, port_sid: &str) -> Option<Reg> {
        if let Some(frame) = self.call_stack.last() {
            frame.port_regs.get(port_sid).copied()
        } else {
            self.port_regs.get(port_sid).copied()
        }
    }

    fn cache_set(&mut self, port_sid: String, reg: Reg) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.port_regs.insert(port_sid, reg);
        } else {
            self.port_regs.insert(port_sid, reg);
        }
    }

    fn lower_port(&mut self, port_sid: &str) -> Reg {
        if let Some(r) = self.cache_get(port_sid) {
            return r;
        }
        let Some(pref) = self.graph.ports.get(port_sid).cloned() else {
            return self.emit_const_null(port_sid, "missing");
        };
        let reg = self.lower_node_output(&pref.node_sid, &pref.port_name);
        self.cache_set(port_sid.to_string(), reg);
        reg
    }

    fn lower_node_output(&mut self, node_sid: &str, port_name: &str) -> Reg {
        let Some(node) = self.graph.nodes.get(node_sid).cloned() else {
            return self.emit_const_null(node_sid, port_name);
        };

        if node.id == "CreateFunction" {
            if let Some(frame) = self.call_stack.last() {
                if frame.create_sid == node.sid {
                    let idx = match port_name {
                        "Any1" => 0,
                        "Any2" => 1,
                        "Any3" => 2,
                        "Any4" => 3,
                        _ => return self.emit_const_null(node_sid, port_name),
                    };
                    let arg = frame.args[idx];
                    return self.emit_move(node_sid, port_name, arg, RegisterKind::Null);
                }
            }
            return self.emit_const_null(node_sid, port_name);
        }

        match node.id.as_str() {
            "Float" => self.emit_const_float(node_sid, port_name, parse_float(&node.modifier)),
            "Bool" => self.emit_const_bool(node_sid, port_name, node.modifier.trim() != "1"),
            "GetVariable" => {
                let vid = self.vars.intern(&node.modifier);
                let dst = self.fresh_reg(RegisterKind::Null);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Null,
                    op: OpCode::LoadVar,
                    args: vec![Reg(vid.0 as u32)],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "SetVariable" => {
                let value = self
                    .lower_input(node_sid, "Any1")
                    .unwrap_or_else(|| self.emit_const_null(node_sid, "Any1"));
                let vid = self.vars.intern(&node.modifier);
                self.ir.push(IrInst {
                    dest: None,
                    kind: RegisterKind::Null,
                    op: OpCode::StoreVar,
                    args: vec![Reg(vid.0 as u32), value],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: "Any1".to_string(),
                });
                self.emit_move(node_sid, port_name, value, RegisterKind::Null)
            }
            "Function" => {
                let value = self.lower_function_call(&node);
                self.emit_move(node_sid, port_name, value, RegisterKind::Null)
            }
            "SoccerGetBool" => {
                let slot = self.apis.intern(&node.modifier, ApiKind::Bool);
                self.emit_load_api(node_sid, port_name, slot, RegisterKind::Bool)
            }
            "SoccerGetFloat" => {
                let slot = self.apis.intern(&node.modifier, ApiKind::Float);
                self.emit_load_api(node_sid, port_name, slot, RegisterKind::Float)
            }
            "SoccerGetTransform" => {
                let slot = self.apis.intern(&node.modifier, ApiKind::Transform);
                self.emit_load_api(node_sid, port_name, slot, RegisterKind::Vector)
            }
            "SoccerGetVector3" => {
                let slot = self.apis.intern(&node.modifier, ApiKind::Vector3);
                self.emit_load_api(node_sid, port_name, slot, RegisterKind::Vector)
            }
            "RelativePosition" => {
                use crate::graph::dropdowns::{relative_position_mode, RelativePosMode};
                match relative_position_mode(&node.modifier) {
                    RelativePosMode::WorldPos => {
                        let pos = self.lower_input(node_sid, "Transform1").unwrap_or_else(|| {
                            self.emit_const_vec2(node_sid, "Transform1", Vec2::ZERO)
                        });
                        self.emit_move(node_sid, port_name, pos, RegisterKind::Vector)
                    }
                    RelativePosMode::DirOnly(d) => self.emit_const_vec2(node_sid, port_name, d),
                    RelativePosMode::PosPlus(d) => {
                        let pos = self.lower_input(node_sid, "Transform1").unwrap_or_else(|| {
                            self.emit_const_vec2(node_sid, "Transform1", Vec2::ZERO)
                        });
                        let offset = self.emit_const_vec2(node_sid, "RelOffset", d);
                        let dst = self.fresh_reg(RegisterKind::Vector);
                        self.ir.push(IrInst {
                            dest: Some(dst),
                            kind: RegisterKind::Vector,
                            op: OpCode::AddVec,
                            args: vec![pos, offset],
                            immediates: vec![],
                            source_sid: node_sid.to_string(),
                            source_port: port_name.to_string(),
                        });
                        dst
                    }
                }
            }
            "ConstructVector3" => {
                let x = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0));
                let _y = self
                    .lower_input(node_sid, "Float2")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 0.0));
                let z = self
                    .lower_input(node_sid, "Float3")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float3", 0.0));
                let dst = self.fresh_reg(RegisterKind::Vector);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Vector,
                    op: OpCode::ConstructVec,
                    args: vec![x, z],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "Vector3Split" => {
                let v = self
                    .lower_input(node_sid, "Vector31")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
                let axis = match port_name {
                    "Float1" => 0,
                    "Float2" => 1,
                    "Float3" => 2,
                    _ => return self.emit_const_null(node_sid, port_name),
                };
                let dst = self.fresh_reg(RegisterKind::Float);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Float,
                    op: OpCode::SplitVec,
                    args: vec![v],
                    immediates: vec![axis],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "AddFloats" => self.bin_f(node_sid, port_name, OpCode::Add),
            "SubtractFloats" => self.bin_f(node_sid, port_name, OpCode::Sub),
            "MultiplyFloats" => self.bin_f(node_sid, port_name, OpCode::Mul),
            "DivideFloats" => self.bin_f(node_sid, port_name, OpCode::Div),
            "Power" => self.bin_f(node_sid, port_name, OpCode::Pow),
            "Modulo" => self.bin_f(node_sid, port_name, OpCode::Mod),
            "Lerp" => {
                let a = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0));
                let b = self
                    .lower_input(node_sid, "Float2")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 0.0));
                let t = self
                    .lower_input(node_sid, "Float3")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float3", 0.0));
                let dst = self.fresh_reg(RegisterKind::Float);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Float,
                    op: OpCode::Lerp,
                    args: vec![a, b, t],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "Relay" => {
                let value = self
                    .lower_input(node_sid, "Any1")
                    .unwrap_or_else(|| self.emit_const_null(node_sid, "Any1"));
                self.emit_move(node_sid, port_name, value, RegisterKind::Null)
            }
            "Operation" | "AbsFloat" | "Absolute" => {
                let a = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0));
                let (opcode, op) = if node.id == "Operation" {
                    let kind =
                        crate::graph::dropdowns::OperationKind::from_modifier(&node.modifier);
                    (OpCode::Operation, kind.as_u32())
                } else {
                    (OpCode::Abs, 0)
                };
                let dst = self.fresh_reg(RegisterKind::Float);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Float,
                    op: opcode,
                    args: vec![a],
                    immediates: if opcode == OpCode::Operation {
                        vec![op]
                    } else {
                        vec![]
                    },
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "AddVector3" => self.bin_v(node_sid, port_name, OpCode::AddVec),
            "SubtractVector3" => self.bin_v(node_sid, port_name, OpCode::SubVec),
            "ScaleVector3" => {
                let v = self
                    .lower_input(node_sid, "Vector31")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
                let s = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 1.0));
                let dst = self.fresh_reg(RegisterKind::Vector);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Vector,
                    op: OpCode::ScaleVec,
                    args: vec![v, s],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "Normalize" => self.unary_v(node_sid, port_name, OpCode::Normalize),
            "Magnitude" => self.unary_v(node_sid, port_name, OpCode::Magnitude),
            "Distance" => self.bin_v(node_sid, port_name, OpCode::Distance),
            "DotProduct" => self.bin_v(node_sid, port_name, OpCode::Dot),
            "Not" => {
                let b = self
                    .lower_input(node_sid, "Bool1")
                    .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool1", false));
                let dst = self.fresh_reg(RegisterKind::Bool);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Bool,
                    op: OpCode::Not,
                    args: vec![b],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "CompareFloats" => {
                let a = self
                    .lower_input(node_sid, "Float1")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0));
                let b = self
                    .lower_input(node_sid, "Float2")
                    .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 0.0));
                let op = node.modifier.parse::<i32>().unwrap_or(0);
                let (opcode, imm) = compare_float_op(op);
                let dst = self.fresh_reg(RegisterKind::Bool);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Bool,
                    op: opcode,
                    args: vec![a, b],
                    immediates: vec![imm],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "CompareBool" => {
                let a = self
                    .lower_input(node_sid, "Bool1")
                    .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool1", false));
                let b = self
                    .lower_input(node_sid, "Bool2")
                    .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool2", false));
                let op = node.modifier.parse::<i32>().unwrap_or(0);
                let opcode = match op {
                    1 => OpCode::Or,
                    2 | 6 => OpCode::Eq,
                    3 => OpCode::Ne,
                    4 => OpCode::Nor,
                    5 => OpCode::Nand,
                    _ => OpCode::And,
                };
                let dst = self.fresh_reg(RegisterKind::Bool);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Bool,
                    op: opcode,
                    args: vec![a, b],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            "ConditionalSetBool" => self.select(node_sid, port_name, RegisterKind::Bool),
            "ConditionalSetFloatV2" | "ConditionalSetFloat" => {
                self.select(node_sid, port_name, RegisterKind::Float)
            }
            "ConditionalSetVector3" => self.select(node_sid, port_name, RegisterKind::Vector),
            "IsNull" => {
                let v = self
                    .lower_input(node_sid, "Any1")
                    .or_else(|| self.lower_input(node_sid, "Vector31"))
                    .unwrap_or_else(|| self.emit_const_null(node_sid, "Any1"));
                let dst = self.fresh_reg(RegisterKind::Bool);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Bool,
                    op: OpCode::IsNull,
                    args: vec![v],
                    immediates: vec![],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            // Runtime Keypress — viewer fills [`crate::keypress`]; headless stays false.
            "Keypress" => {
                let kid = crate::keypress::key_id(&node.modifier);
                let dst = self.fresh_reg(RegisterKind::Bool);
                self.ir.push(IrInst {
                    dest: Some(dst),
                    kind: RegisterKind::Bool,
                    op: OpCode::Keypress,
                    args: vec![],
                    immediates: vec![kid],
                    source_sid: node_sid.to_string(),
                    source_port: port_name.to_string(),
                });
                dst
            }
            // Color is a named constant (White/Cyan/…); keep as non-null so
            // consumers (TimePlot/DebugDraw) can lower inputs. Stored as float 0
            // — color name only matters for Unity viz.
            "Color" => self.emit_const_float(node_sid, port_name, 0.0),
            // Side-effect / viz / faceoff stubs: evaluate as null outputs.
            "Debug"
            | "DebugDrawDisc"
            | "DebugDrawLine"
            | "TimePlot"
            | "Region"
            | "ConstructSoccerProperties"
            | "Spherecast"
            | "Country"
            | "Stat" => self.emit_const_null(node_sid, port_name),
            _ => self.emit_const_null(node_sid, port_name),
        }
    }

    fn select(&mut self, node_sid: &str, port_name: &str, kind: RegisterKind) -> Reg {
        let cond = self
            .lower_input(node_sid, "Bool1")
            .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool1", false));
        let t = match kind {
            RegisterKind::Bool => self
                .lower_input(node_sid, "Bool2")
                .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool2", false)),
            RegisterKind::Float => self
                .lower_input(node_sid, "Float1")
                .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0)),
            RegisterKind::Vector => self
                .lower_input(node_sid, "Vector31")
                .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO)),
            RegisterKind::Null => self.emit_const_null(node_sid, port_name),
        };
        let f = match kind {
            RegisterKind::Bool => self
                .lower_input(node_sid, "Bool3")
                .unwrap_or_else(|| self.emit_const_bool(node_sid, "Bool3", false)),
            RegisterKind::Float => self
                .lower_input(node_sid, "Float2")
                .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 0.0)),
            RegisterKind::Vector => self
                .lower_input(node_sid, "Vector32")
                .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector32", Vec2::ZERO)),
            RegisterKind::Null => self.emit_const_null(node_sid, port_name),
        };
        let dst = self.fresh_reg(kind);
        // GraphBrain ConditionalSet* always coerces via as_bool/as_float/as_vec.
        let kind_imm = match kind {
            RegisterKind::Float => 0,
            RegisterKind::Bool => 1,
            RegisterKind::Vector => 2,
            RegisterKind::Null => 3,
        };
        self.ir.push(IrInst {
            dest: Some(dst),
            kind,
            op: OpCode::Select,
            args: vec![cond, t, f],
            immediates: vec![kind_imm],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn lower_function_call(&mut self, fn_node: &GraphNode) -> Reg {
        // Unity AIComp: custom functions cannot nest (Discord AIA bot). Nested
        // Function calls lower to Null — pass values as parameters instead.
        if !self.call_stack.is_empty() {
            return self.emit_const_null(&fn_node.sid, "Any1");
        }
        let Some(def) = self.graph.create_functions.get(&fn_node.modifier).cloned() else {
            return self.emit_const_null(&fn_node.sid, "Any1");
        };
        let args = [
            self.lower_input(&fn_node.sid, "Any1")
                .unwrap_or_else(|| self.emit_const_null(&fn_node.sid, "Any1")),
            self.lower_input(&fn_node.sid, "Any2")
                .unwrap_or_else(|| self.emit_const_null(&fn_node.sid, "Any2")),
            self.lower_input(&fn_node.sid, "Any3")
                .unwrap_or_else(|| self.emit_const_null(&fn_node.sid, "Any3")),
            self.lower_input(&fn_node.sid, "Any4")
                .unwrap_or_else(|| self.emit_const_null(&fn_node.sid, "Any4")),
        ];
        self.call_stack.push(CallFrame {
            create_sid: def.sid.clone(),
            call_sid: fn_node.sid.clone(),
            args,
            port_regs: HashMap::new(),
        });
        // Unity runs DebugDraw sinks inside the function body each call.
        let body_draws = def.debug_draws.clone();
        for sid in &body_draws {
            self.lower_debug_draw(sid);
        }
        let ret = self
            .lower_input(&def.sid, "Any1")
            .unwrap_or_else(|| self.emit_const_null(&def.sid, "Any1"));
        self.call_stack.pop();
        ret
    }

    fn bin_f(&mut self, node_sid: &str, port_name: &str, op: OpCode) -> Reg {
        let a = self
            .lower_input(node_sid, "Float1")
            .unwrap_or_else(|| self.emit_const_float(node_sid, "Float1", 0.0));
        let b = self
            .lower_input(node_sid, "Float2")
            .unwrap_or_else(|| self.emit_const_float(node_sid, "Float2", 0.0));
        let dst = self.fresh_reg(RegisterKind::Float);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind: RegisterKind::Float,
            op,
            args: vec![a, b],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn bin_v(&mut self, node_sid: &str, port_name: &str, op: OpCode) -> Reg {
        let a = self
            .lower_input(node_sid, "Vector31")
            .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
        let b = self
            .lower_input(node_sid, "Vector32")
            .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector32", Vec2::ZERO));
        let kind = match op {
            OpCode::Dot | OpCode::Distance => RegisterKind::Float,
            _ => RegisterKind::Vector,
        };
        let dst = self.fresh_reg(kind);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind,
            op,
            args: vec![a, b],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn unary_v(&mut self, node_sid: &str, port_name: &str, op: OpCode) -> Reg {
        let v = self
            .lower_input(node_sid, "Vector31")
            .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Vector31", Vec2::ZERO));
        let kind = match op {
            OpCode::Magnitude => RegisterKind::Float,
            _ => RegisterKind::Vector,
        };
        let dst = self.fresh_reg(kind);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind,
            op,
            args: vec![v],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_move(
        &mut self,
        node_sid: &str,
        port_name: &str,
        value: Reg,
        kind: RegisterKind,
    ) -> Reg {
        let dst = self.fresh_reg(kind);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind,
            op: OpCode::Move,
            args: vec![value],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_load_api(
        &mut self,
        node_sid: &str,
        port_name: &str,
        slot: ApiSlot,
        kind: RegisterKind,
    ) -> Reg {
        let dst = self.fresh_reg(kind);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind,
            op: OpCode::LoadApi,
            args: vec![],
            immediates: vec![slot.get() as u32],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_const_float(&mut self, node_sid: &str, port_name: &str, f: f32) -> Reg {
        let dst = self.fresh_reg(RegisterKind::Float);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind: RegisterKind::Float,
            op: OpCode::ConstFloat,
            args: vec![],
            immediates: vec![f.to_bits()],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_const_bool(&mut self, node_sid: &str, port_name: &str, b: bool) -> Reg {
        let dst = self.fresh_reg(RegisterKind::Bool);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind: RegisterKind::Bool,
            op: OpCode::ConstBool,
            args: vec![],
            immediates: vec![if b { 1 } else { 0 }],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_const_vec2(&mut self, node_sid: &str, port_name: &str, v: Vec2) -> Reg {
        let dst = self.fresh_reg(RegisterKind::Vector);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind: RegisterKind::Vector,
            op: OpCode::ConstVec,
            args: vec![],
            immediates: vec![v.x.to_bits(), v.y.to_bits()],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }

    fn emit_const_null(&mut self, node_sid: &str, port_name: &str) -> Reg {
        let dst = self.fresh_reg(RegisterKind::Null);
        self.ir.push(IrInst {
            dest: Some(dst),
            kind: RegisterKind::Null,
            op: OpCode::ConstNull,
            args: vec![],
            immediates: vec![],
            source_sid: node_sid.to_string(),
            source_port: port_name.to_string(),
        });
        dst
    }
}

fn parse_float(s: &str) -> f32 {
    let t = s.trim().replace(',', ".");
    t.parse().unwrap_or(0.0)
}

fn compare_float_op(op: i32) -> (OpCode, u32) {
    match op {
        1 => (OpCode::Lt, 0),
        2 => (OpCode::Gt, 0),
        3 => (OpCode::Le, 0),
        4 => (OpCode::Ge, 0),
        _ => (OpCode::Eq, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::load::{index_graph, RawConnection, RawGraph, RawNode, RawPort};
    use crate::graph_vm::builder::ProgramBuilder;
    use crate::graph_vm::context::ExecutionContext;
    use crate::graph_vm::interpreter::Interpreter;
    use crate::graph_vm::program::Backend;

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
    fn hand_built_add_floats_lower_and_run() {
        let raw = RawGraph {
            nodes: vec![
                node("Float", "fa", "3", vec![port("Float1", "fao", 1, "fa")]),
                node("Float", "fb", "4", vec![port("Float1", "fbo", 1, "fb")]),
                node(
                    "AddFloats",
                    "add",
                    "",
                    vec![
                        port("Float1", "add_a", 0, "add"),
                        port("Float2", "add_b", 0, "add"),
                        port("Float1", "add_o", 1, "add"),
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
                    port0: "fao".into(),
                    port1: "add_a".into(),
                },
                RawConnection {
                    port0: "fbo".into(),
                    port1: "add_b".into(),
                },
                RawConnection {
                    port0: "add_o".into(),
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
        let graph = index_graph(raw, "add_test".into());
        let compiled = Lowerer::compile(graph);
        assert!(!compiled.controllers.instructions.is_empty());
        let program = ProgramBuilder.pack(&compiled);
        let mut ctx = ExecutionContext::new(
            crate::api::TeamApi::empty(crate::brain::TeamId::Home),
            compiled.vars.len(),
            program.register_count as usize,
        );
        ctx.init_api_slots(&compiled.apis);
        let mut interp = Interpreter::default();
        interp.execute_controllers(&program, &mut ctx);
        assert!((ctx.output.commands[0].move_to.x - 7.0).abs() < 1e-4);
    }

    #[test]
    fn unity_sqrt_dropdown_index_10_via_runtime() {
        // Unity AIA saves Operation(sqrt) as modifier "10", not the label "sqrt".
        let raw = RawGraph {
            nodes: vec![
                node("Float", "f2", "2", vec![port("Float1", "f2o", 1, "f2")]),
                node(
                    "Operation",
                    "op",
                    "10",
                    vec![
                        port("Float1", "opi", 0, "op"),
                        port("Float1", "opo", 1, "op"),
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
                    port1: "opi".into(),
                },
                RawConnection {
                    port0: "opo".into(),
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
        let graph = index_graph(raw, "sqrt_test".into());
        let compiled = Lowerer::compile(graph);
        let program = ProgramBuilder.pack(&compiled);
        let mut ctx = ExecutionContext::new(
            crate::api::TeamApi::empty(crate::brain::TeamId::Home),
            compiled.vars.len(),
            program.register_count as usize,
        );
        ctx.init_api_slots(&compiled.apis);
        let mut interp = Interpreter::default();
        interp.execute_controllers(&program, &mut ctx);
        let expect = 2.0_f32.sqrt();
        assert!(
            (ctx.output.commands[0].move_to.x - expect).abs() < 1e-4,
            "got {} want sqrt(2)={}",
            ctx.output.commands[0].move_to.x,
            expect
        );
    }

    fn node_owned(
        id: &str,
        sid: &str,
        modifier: &str,
        owner: &str,
        ports: Vec<RawPort>,
    ) -> RawNode {
        RawNode {
            id: id.into(),
            sid: sid.into(),
            modifier: serde_json::json!(modifier),
            owner_function_sid: owner.into(),
            ports,
        }
    }

    #[test]
    fn root_function_debug_draw_lowers_and_runs() {
        let raw = RawGraph {
            nodes: vec![
                node(
                    "CreateFunction",
                    "cf",
                    "Draw",
                    vec![
                        port("Any1", "cf_a1o", 1, "cf"),
                        port("Any2", "cf_a2o", 1, "cf"),
                        port("Any4", "cf_a4o", 1, "cf"),
                        port("Any1", "cf_ret", 0, "cf"),
                    ],
                ),
                node_owned("Relay", "ra", "", "cf", vec![port("Any1", "rao", 2, "ra")]),
                node_owned("Relay", "rb", "", "cf", vec![port("Any1", "rbo", 2, "rb")]),
                node_owned("Relay", "rc", "", "cf", vec![port("Any1", "rco", 2, "rc")]),
                node_owned(
                    "Float",
                    "fw",
                    "1",
                    "cf",
                    vec![port("Float1", "fwo", 1, "fw")],
                ),
                node_owned(
                    "DebugDrawLine",
                    "ddl",
                    "",
                    "cf",
                    vec![
                        port("Vector31", "ddla", 0, "ddl"),
                        port("Vector32", "ddlb", 0, "ddl"),
                        port("Float1", "ddlw", 0, "ddl"),
                        port("Color1", "ddlc", 0, "ddl"),
                    ],
                ),
                node("Float", "ax", "1", vec![port("Float1", "axo", 1, "ax")]),
                node("Float", "az", "2", vec![port("Float1", "azo", 1, "az")]),
                node("Float", "ay", "0", vec![port("Float1", "ayo", 1, "ay")]),
                node(
                    "ConstructVector3",
                    "pa",
                    "",
                    vec![
                        port("Vector31", "pao", 1, "pa"),
                        port("Float1", "pax", 0, "pa"),
                        port("Float2", "pay", 0, "pa"),
                        port("Float3", "paz", 0, "pa"),
                    ],
                ),
                node("Float", "bx", "5", vec![port("Float1", "bxo", 1, "bx")]),
                node("Float", "bz", "6", vec![port("Float1", "bzo", 1, "bz")]),
                node("Float", "by", "0", vec![port("Float1", "byo", 1, "by")]),
                node(
                    "ConstructVector3",
                    "pb",
                    "",
                    vec![
                        port("Vector31", "pbo", 1, "pb"),
                        port("Float1", "pbx", 0, "pb"),
                        port("Float2", "pby", 0, "pb"),
                        port("Float3", "pbz", 0, "pb"),
                    ],
                ),
                node(
                    "Color",
                    "col",
                    "Orange",
                    vec![port("Color1", "colo", 1, "col")],
                ),
                node(
                    "Function",
                    "fn",
                    "Draw",
                    vec![
                        port("Any1", "fna1", 0, "fn"),
                        port("Any2", "fna2", 0, "fn"),
                        port("Any4", "fna4", 0, "fn"),
                        port("Any1", "fno", 1, "fn"),
                    ],
                ),
            ],
            connections: vec![
                RawConnection {
                    port0: "cf_a1o".into(),
                    port1: "rao".into(),
                },
                RawConnection {
                    port0: "cf_a2o".into(),
                    port1: "rbo".into(),
                },
                RawConnection {
                    port0: "cf_a4o".into(),
                    port1: "rco".into(),
                },
                RawConnection {
                    port0: "rao".into(),
                    port1: "ddla".into(),
                },
                RawConnection {
                    port0: "rbo".into(),
                    port1: "ddlb".into(),
                },
                RawConnection {
                    port0: "rco".into(),
                    port1: "ddlc".into(),
                },
                RawConnection {
                    port0: "fwo".into(),
                    port1: "ddlw".into(),
                },
                RawConnection {
                    port0: "axo".into(),
                    port1: "pax".into(),
                },
                RawConnection {
                    port0: "ayo".into(),
                    port1: "pay".into(),
                },
                RawConnection {
                    port0: "azo".into(),
                    port1: "paz".into(),
                },
                RawConnection {
                    port0: "bxo".into(),
                    port1: "pbx".into(),
                },
                RawConnection {
                    port0: "byo".into(),
                    port1: "pby".into(),
                },
                RawConnection {
                    port0: "bzo".into(),
                    port1: "pbz".into(),
                },
                RawConnection {
                    port0: "pao".into(),
                    port1: "fna1".into(),
                },
                RawConnection {
                    port0: "pbo".into(),
                    port1: "fna2".into(),
                },
                RawConnection {
                    port0: "colo".into(),
                    port1: "fna4".into(),
                },
            ],
        };

        let graph = index_graph(raw, "vm_draw_test".into());
        let compiled = Lowerer::compile(graph);
        let has_draw = compiled
            .controllers
            .instructions
            .iter()
            .any(|i| i.op == OpCode::DebugDrawLine);
        assert!(
            has_draw,
            "controllers IR must include DebugDrawLine from Function body"
        );

        let program = ProgramBuilder.pack(&compiled);
        let mut ctx = ExecutionContext::new(
            crate::api::TeamApi::empty(crate::brain::TeamId::Home),
            compiled.vars.len(),
            program.register_count as usize,
        );
        ctx.init_api_slots(&compiled.apis);
        crate::debug_draw::begin_frame();
        let mut interp = Interpreter::default();
        interp.execute_settle(&program, &mut ctx);
        interp.execute_controllers(&program, &mut ctx);
        let snap = crate::debug_draw::snapshot();
        assert_eq!(snap.lines.len(), 1);
        let line = &snap.lines[0];
        assert!((line.a.x - 1.0).abs() < 1e-4 && (line.a.y - 2.0).abs() < 1e-4);
        assert!((line.b.x - 5.0).abs() < 1e-4 && (line.b.y - 6.0).abs() < 1e-4);
        assert_eq!(line.rgba, crate::debug_draw::named_rgba("Orange"));
    }
}
