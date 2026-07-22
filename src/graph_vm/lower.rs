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
    label_to_slot: HashMap<(ApiKind, String), ApiSlot>,
}

impl ApiSlotTable {
    pub fn intern(&mut self, label: &str, kind: ApiKind) -> ApiSlot {
        let key = (kind, label.to_string());
        if let Some(&slot) = self.label_to_slot.get(&key) {
            return slot;
        }
        let idx = self.labels.len();
        self.labels.push(label.to_string());
        self.kinds.push(kind);
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
                let pos = self
                    .lower_input(node_sid, "Transform1")
                    .unwrap_or_else(|| self.emit_const_vec2(node_sid, "Transform1", Vec2::ZERO));
                self.emit_move(node_sid, port_name, pos, RegisterKind::Vector)
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
                    (OpCode::Operation, operation_kind(&node.modifier))
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
            args,
            port_regs: HashMap::new(),
        });
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

fn operation_kind(modifier: &str) -> u32 {
    match modifier.to_ascii_lowercase().as_str() {
        "abs" => 0,
        "round" => 1,
        "floor" => 2,
        "ceil" => 3,
        "sign" | "signum" => 4,
        "sqrt" => 5,
        "sin" => 6,
        "cos" => 7,
        "tan" => 8,
        "ln" => 9,
        "log10" => 10,
        "e^" => 11,
        "10^" => 12,
        _ => 13,
    }
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
            crate::api::TeamApi {
                team: crate::brain::TeamId::Home,
                bools: Default::default(),
                floats: Default::default(),
                transforms: Default::default(),
                vectors: Default::default(),
            },
            compiled.vars.len(),
            program.register_count as usize,
        );
        ctx.init_api_slots(&compiled.apis);
        let mut interp = Interpreter::default();
        interp.execute_controllers(&program, &mut ctx);
        assert!((ctx.output.commands[0].move_to.x - 7.0).abs() < 1e-4);
    }
}
