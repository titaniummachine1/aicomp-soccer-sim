//! Intentionally dumb O0 interpreter — correctness / TRACE identity first.

use bevy::prelude::{Vec2, Vec3};

use crate::graph::pitch_vec::pitch_plane;

use crate::brain::BrainCommand;
use crate::graph_vm::context::ExecutionContext;
use crate::graph_vm::opcode::{Instruction, OpCode};
use crate::graph_vm::program::{Backend, RuntimeProgram};
use crate::graph_vm::value::VmValue;

#[derive(Debug, Default)]
pub struct Interpreter;

impl Backend for Interpreter {
    fn execute_settle(&mut self, program: &RuntimeProgram, ctx: &mut ExecutionContext) {
        ctx.frame.ensure_regs(program.register_count as usize);
        for inst in program.settle_ops() {
            run_inst(inst, ctx, &program.var_names);
        }
    }

    fn execute_controllers(&mut self, program: &RuntimeProgram, ctx: &mut ExecutionContext) {
        ctx.frame.ensure_regs(program.register_count as usize);
        for inst in program.controller_ops() {
            run_inst(inst, ctx, &program.var_names);
        }
    }
}

fn run_inst(inst: &Instruction, ctx: &mut ExecutionContext, names: &[String]) {
    let ops = inst.operands.as_slice();
    match inst.opcode {
        OpCode::ConstNull => {
            if let Some(&dst) = ops.first() {
                ctx.frame.registers[dst as usize] = VmValue::Null;
            }
        }
        OpCode::ConstFloat => {
            if let Some(&dst) = ops.first() {
                let f = ops.get(1).copied().map(f32::from_bits).unwrap_or(0.0);
                ctx.frame.registers[dst as usize] = VmValue::Float(f);
            }
        }
        OpCode::ConstBool => {
            if let Some(&dst) = ops.first() {
                ctx.frame.registers[dst as usize] =
                    VmValue::Bool(ops.get(1).copied().unwrap_or(0) != 0);
            }
        }
        OpCode::ConstVec => {
            if let Some(&dst) = ops.first() {
                let x = ops.get(1).copied().map(f32::from_bits).unwrap_or(0.0);
                let y = ops.get(2).copied().map(f32::from_bits).unwrap_or(0.0);
                let z = ops.get(3).copied().map(f32::from_bits).unwrap_or(0.0);
                ctx.frame.registers[dst as usize] = VmValue::Vector(Vec3::new(x, y, z));
            }
        }
        OpCode::LoadVar => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let vid = ops[1] as usize;
                ctx.frame.registers[dst] =
                    ctx.state.vars.get(vid).copied().unwrap_or(VmValue::Null);
            }
        }
        OpCode::StoreVar => {
            if ops.len() >= 2 {
                let vid = ops[0] as usize;
                let src = ops[1] as usize;
                if vid < ctx.state.vars.len() {
                    let val = ctx.frame.registers[src];
                    ctx.state.vars[vid] = val;
                    ctx.record_var_commit(vid as u16, val, &inst.source_sid);
                }
            }
        }
        OpCode::LoadApi => {
            if let Some(&dst) = ops.first() {
                let slot = ops.get(1).copied().unwrap_or(0) as u16;
                ctx.frame.registers[dst as usize] = ctx.api.load_slot(slot);
            }
        }
        OpCode::Keypress => {
            if let Some(&dst) = ops.first() {
                let kid = ops.get(1).copied().unwrap_or(0);
                ctx.frame.registers[dst as usize] =
                    VmValue::Bool(crate::keypress::is_pressed_id(kid));
            }
        }
        OpCode::Add => bin_f(ctx, ops, |a, b| a + b),
        OpCode::Sub => bin_f(ctx, ops, |a, b| a - b),
        OpCode::Mul => bin_f(ctx, ops, |a, b| a * b),
        OpCode::Div => bin_f(ctx, ops, |a, b| if b.abs() < 1e-12 { 0.0 } else { a / b }),
        OpCode::Mod => bin_f(ctx, ops, |a, b| if b.abs() < 1e-12 { 0.0 } else { a % b }),
        OpCode::Pow => bin_f(ctx, ops, |a, b| a.powf(b)),
        OpCode::Lerp => {
            if ops.len() >= 4 {
                let dst = ops[0] as usize;
                let a = reg_f(ctx, ops[1] as usize);
                let b = reg_f(ctx, ops[2] as usize);
                let t = reg_f(ctx, ops[3] as usize);
                ctx.frame.registers[dst] = VmValue::Float(a + (b - a) * t);
            }
        }
        OpCode::Abs => unary_f(ctx, ops, |a, _| a.abs()),
        OpCode::Operation => unary_f(ctx, ops, crate::graph::dropdowns::eval_operation),
        OpCode::Move => {
            if ops.len() >= 2 {
                let value = ctx
                    .frame
                    .registers
                    .get(ops[1] as usize)
                    .copied()
                    .unwrap_or(VmValue::Null);
                ctx.frame.registers[ops[0] as usize] = value;
            }
        }
        OpCode::IsNull => {
            if ops.len() >= 2 {
                let value = ctx
                    .frame
                    .registers
                    .get(ops[1] as usize)
                    .copied()
                    .unwrap_or(VmValue::Null);
                ctx.frame.registers[ops[0] as usize] = VmValue::Bool(value == VmValue::Null);
            }
        }
        OpCode::Not => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let b = reg_b(ctx, ops[1] as usize);
                ctx.frame.registers[dst] = VmValue::Bool(!b);
            }
        }
        OpCode::And | OpCode::Or | OpCode::Nor | OpCode::Nand => {
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let a = reg_b(ctx, ops[1] as usize);
                let b = reg_b(ctx, ops[2] as usize);
                let value = match inst.opcode {
                    OpCode::And => a && b,
                    OpCode::Or => a || b,
                    OpCode::Nor => !(a || b),
                    OpCode::Nand => !(a && b),
                    _ => unreachable!(),
                };
                ctx.frame.registers[dst] = VmValue::Bool(value);
            }
        }
        OpCode::Lt | OpCode::Gt | OpCode::Le | OpCode::Ge | OpCode::Eq | OpCode::Ne => {
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let a = reg_f(ctx, ops[1] as usize);
                let b = reg_f(ctx, ops[2] as usize);
                let imm = ops.get(3).copied().unwrap_or(0);
                let r = match inst.opcode {
                    OpCode::Lt => a < b,
                    OpCode::Gt => a > b,
                    OpCode::Le => a <= b,
                    OpCode::Ge => a >= b,
                    OpCode::Eq if imm == 1 => (a - b).abs() < 1e-5,
                    OpCode::Eq => reg_eq(ctx, ops[1] as usize, ops[2] as usize),
                    OpCode::Ne => !reg_eq(ctx, ops[1] as usize, ops[2] as usize),
                    _ => false,
                };
                ctx.frame.registers[dst] = VmValue::Bool(r);
            }
        }
        OpCode::ConstructVec => {
            if ops.len() >= 4 {
                let dst = ops[0] as usize;
                let x = reg_f(ctx, ops[1] as usize);
                let y = reg_f(ctx, ops[2] as usize);
                let z = reg_f(ctx, ops[3] as usize);
                ctx.frame.registers[dst] = VmValue::Vector(Vec3::new(x, y, z));
            }
        }
        OpCode::SplitVec => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let axis = ops.get(2).copied().unwrap_or(0);
                let f = match axis {
                    0 => v.x,
                    1 => v.y,
                    _ => v.z,
                };
                ctx.frame.registers[dst] = VmValue::Float(f);
            }
        }
        OpCode::AddVec => bin_v(ctx, ops, |a, b| a + b),
        OpCode::SubVec => bin_v(ctx, ops, |a, b| a - b),
        OpCode::ScaleVec => {
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let s = reg_f(ctx, ops[2] as usize);
                ctx.frame.registers[dst] = VmValue::Vector(v * s);
            }
        }
        OpCode::ScaleAddVec => {
            if ops.len() >= 4 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let s = reg_f(ctx, ops[2] as usize);
                let addend = reg_v(ctx, ops[3] as usize);
                let scaled = v * s;
                ctx.frame.registers[dst] = VmValue::Vector(scaled + addend);
            }
        }
        OpCode::Normalize => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let len = v.length();
                ctx.frame.registers[dst] =
                    VmValue::Vector(if len > 1e-8 { v / len } else { Vec3::ZERO });
            }
        }
        OpCode::Magnitude => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                ctx.frame.registers[dst] = VmValue::Float(v.length());
            }
        }
        OpCode::Distance => {
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let a = reg_v(ctx, ops[1] as usize);
                let b = reg_v(ctx, ops[2] as usize);
                ctx.frame.registers[dst] = VmValue::Float(a.distance(b));
            }
        }
        OpCode::Dot => {
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let a = reg_v(ctx, ops[1] as usize);
                let b = reg_v(ctx, ops[2] as usize);
                ctx.frame.registers[dst] = VmValue::Float(a.dot(b));
            }
        }
        OpCode::Select => {
            if ops.len() >= 4 {
                let dst = ops[0] as usize;
                let cond = reg_b(ctx, ops[1] as usize);
                let t = ctx.frame.registers[ops[2] as usize];
                let f = ctx.frame.registers[ops[3] as usize];
                let chosen = if cond { t } else { f };
                // immediates[0] / ops[4]: 0=Float, 1=Bool, 2=Vector, else passthrough
                // Matches GraphBrain ConditionalSet* as_float/as_bool/as_vec coercion.
                let coerced = match ops.get(4).copied().unwrap_or(3) {
                    0 => VmValue::Float(match chosen {
                        VmValue::Float(x) => x,
                        VmValue::Bool(b) => {
                            if b {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        VmValue::Vector(v) => v.length(),
                        VmValue::Null => 0.0,
                    }),
                    1 => VmValue::Bool(match chosen {
                        VmValue::Bool(b) => b,
                        VmValue::Float(f) => f != 0.0,
                        VmValue::Vector(_) | VmValue::Null => false,
                    }),
                    2 => VmValue::Vector(match chosen {
                        VmValue::Vector(v) => v,
                        _ => Vec3::ZERO,
                    }),
                    _ => chosen,
                };
                ctx.frame.registers[dst] = coerced;
            }
        }
        OpCode::EmitController => {
            if ops.len() >= 4 {
                let slot = ops.get(3).copied().unwrap_or(0) as usize;
                if slot < 4 {
                    let move_to = pitch_plane(reg_v(ctx, ops[0] as usize));
                    let sprint = reg_b(ctx, ops[1] as usize);
                    let interact = reg_b(ctx, ops[2] as usize);
                    ctx.output.commands[slot] = BrainCommand {
                        move_to,
                        sprint,
                        interact,
                    };
                }
            }
        }
        OpCode::DebugDrawLine => {
            if ops.len() >= 7 {
                let a = pitch_plane(reg_v(ctx, ops[0] as usize));
                let b = pitch_plane(reg_v(ctx, ops[1] as usize));
                let w = reg_f(ctx, ops[2] as usize);
                let rgba = [
                    f32::from_bits(ops[3]),
                    f32::from_bits(ops[4]),
                    f32::from_bits(ops[5]),
                    f32::from_bits(ops[6]),
                ];
                crate::debug_draw::line_rgba(a, b, w, rgba);
            }
        }
        OpCode::TimePlot => {
            // Record the channel so offline tooling can read a brain's own
            // internal values. Previously TimePlot was dropped at load time,
            // which left the sim unable to observe any graph-internal number.
            // operands = [value_reg, name_id] -- immediates follow args.
            if ops.len() >= 2 {
                let v = reg_f(ctx, ops[0] as usize);
                let name = names.get(ops[1] as usize).map(String::as_str).unwrap_or("?");
                crate::debug_draw::plot(name, v);
            }
        }
        OpCode::DebugDrawDisc => {
            if ops.len() >= 7 {
                let c = pitch_plane(reg_v(ctx, ops[0] as usize));
                let r = reg_f(ctx, ops[1] as usize);
                let w = reg_f(ctx, ops[2] as usize);
                let rgba = [
                    f32::from_bits(ops[3]),
                    f32::from_bits(ops[4]),
                    f32::from_bits(ops[5]),
                    f32::from_bits(ops[6]),
                ];
                crate::debug_draw::disc_rgba(c, r, w, rgba);
            }
        }
        _ => {}
    }
}

fn bin_f(ctx: &mut ExecutionContext, ops: &[u32], f: impl Fn(f32, f32) -> f32) {
    if ops.len() < 3 {
        return;
    }
    let a = reg_f(ctx, ops[1] as usize);
    let b = reg_f(ctx, ops[2] as usize);
    ctx.frame.registers[ops[0] as usize] = VmValue::Float(f(a, b));
}

fn bin_v(ctx: &mut ExecutionContext, ops: &[u32], f: impl Fn(Vec3, Vec3) -> Vec3) {
    if ops.len() < 3 {
        return;
    }
    let a = reg_v(ctx, ops[1] as usize);
    let b = reg_v(ctx, ops[2] as usize);
    ctx.frame.registers[ops[0] as usize] = VmValue::Vector(f(a, b));
}

fn unary_f(ctx: &mut ExecutionContext, ops: &[u32], f: impl Fn(f32, u32) -> f32) {
    if ops.len() < 2 {
        return;
    }
    let kind = ops.get(2).copied().unwrap_or(0);
    let a = reg_f(ctx, ops[1] as usize);
    ctx.frame.registers[ops[0] as usize] = VmValue::Float(f(a, kind));
}

fn reg_f(ctx: &ExecutionContext, i: usize) -> f32 {
    match ctx.frame.registers.get(i).copied().unwrap_or(VmValue::Null) {
        VmValue::Float(x) => x,
        VmValue::Bool(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        VmValue::Vector(value) => value.length(),
        VmValue::Null => 0.0,
    }
}

fn reg_b(ctx: &ExecutionContext, i: usize) -> bool {
    match ctx.frame.registers.get(i).copied().unwrap_or(VmValue::Null) {
        VmValue::Bool(x) => x,
        VmValue::Float(value) => value != 0.0,
        VmValue::Vector(_) | VmValue::Null => false,
    }
}

fn reg_v(ctx: &ExecutionContext, i: usize) -> Vec3 {
    match ctx.frame.registers.get(i).copied().unwrap_or(VmValue::Null) {
        VmValue::Vector(v) => v,
        _ => Vec3::ZERO,
    }
}

fn reg_eq(ctx: &ExecutionContext, a: usize, b: usize) -> bool {
    ctx.frame.registers.get(a).copied().unwrap_or(VmValue::Null)
        == ctx.frame.registers.get(b).copied().unwrap_or(VmValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::TeamId;
    use crate::graph_vm::builder::ProgramBuilder;
    use crate::graph_vm::ir::{IrInst, LoweredIR, Reg};
    use crate::graph_vm::value::RegisterKind;

    #[test]
    fn hand_built_lowered_ir_add_executes() {
        let mut ir = LoweredIR::new();
        ir.instructions = vec![
            IrInst {
                dest: Some(Reg(0)),
                kind: RegisterKind::Float,
                op: OpCode::ConstFloat,
                args: vec![],
                immediates: vec![2.5_f32.to_bits()],
                source_sid: "left".into(),
                source_port: "Float1".into(),
            },
            IrInst {
                dest: Some(Reg(1)),
                kind: RegisterKind::Float,
                op: OpCode::ConstFloat,
                args: vec![],
                immediates: vec![4.0_f32.to_bits()],
                source_sid: "right".into(),
                source_port: "Float1".into(),
            },
            IrInst {
                dest: Some(Reg(2)),
                kind: RegisterKind::Float,
                op: OpCode::Add,
                args: vec![Reg(0), Reg(1)],
                immediates: vec![],
                source_sid: "add".into(),
                source_port: "Float1".into(),
            },
        ];
        let program = ProgramBuilder.pack_lowered(&ir, 0);
        let api = crate::api::TeamApi::empty(TeamId::Home);
        let mut ctx = ExecutionContext::new(api, 0, program.register_count as usize);

        Interpreter.execute_settle(&program, &mut ctx);

        assert_eq!(ctx.frame.registers[2], VmValue::Float(6.5));
    }
}
