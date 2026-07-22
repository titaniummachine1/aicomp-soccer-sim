//! Intentionally dumb O0 interpreter — correctness / TRACE identity first.

use bevy::prelude::Vec2;

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
            run_inst(inst, ctx);
        }
    }

    fn execute_controllers(&mut self, program: &RuntimeProgram, ctx: &mut ExecutionContext) {
        ctx.frame.ensure_regs(program.register_count as usize);
        for inst in program.controller_ops() {
            run_inst(inst, ctx);
        }
    }
}

fn run_inst(inst: &Instruction, ctx: &mut ExecutionContext) {
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
                ctx.frame.registers[dst as usize] = VmValue::Bool(ops.get(1).copied().unwrap_or(0) != 0);
            }
        }
        OpCode::ConstVec => {
            if let Some(&dst) = ops.first() {
                let x = ops.get(1).copied().map(f32::from_bits).unwrap_or(0.0);
                let y = ops.get(2).copied().map(f32::from_bits).unwrap_or(0.0);
                ctx.frame.registers[dst as usize] = VmValue::Vector(Vec2::new(x, y));
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
        OpCode::Abs => unary_f(ctx, ops, |a, kind| eval_operation(a, kind)),
        OpCode::Not => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let b = reg_b(ctx, ops[1] as usize);
                ctx.frame.registers[dst] = VmValue::Bool(!b);
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
            if ops.len() >= 3 {
                let dst = ops[0] as usize;
                let x = reg_f(ctx, ops[1] as usize);
                let z = reg_f(ctx, ops[2] as usize);
                ctx.frame.registers[dst] = VmValue::Vector(Vec2::new(x, z));
            }
        }
        OpCode::SplitVec => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let axis = ops.get(2).copied().unwrap_or(0);
                let f = match axis {
                    0 => v.x,
                    1 => 0.0,
                    _ => v.y,
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
        OpCode::Normalize => {
            if ops.len() >= 2 {
                let dst = ops[0] as usize;
                let v = reg_v(ctx, ops[1] as usize);
                let len = v.length();
                ctx.frame.registers[dst] =
                    VmValue::Vector(if len > 1e-8 { v / len } else { Vec2::ZERO });
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
                let mode = ops.get(3).copied().unwrap_or(0);
                if mode >= 10 {
                    let a = reg_b(ctx, ops[1] as usize);
                    let b = reg_b(ctx, ops[2] as usize);
                    let op = mode - 10;
                    let r = match op {
                        1 => a || b,
                        2 => a == b,
                        3 => a ^ b,
                        4 => !(a || b),
                        5 => !(a && b),
                        6 => a == b,
                        _ => a && b,
                    };
                    ctx.frame.registers[dst] = VmValue::Bool(r);
                } else {
                    let cond = reg_b(ctx, ops[1] as usize);
                    let t = ctx.frame.registers[ops[2] as usize];
                    let f = ctx.frame.registers[ops[3] as usize];
                    ctx.frame.registers[dst] = if cond { t } else { f };
                }
            }
        }
        OpCode::EmitController => {
            if ops.len() >= 4 {
                let slot = ops.get(3).copied().unwrap_or(0) as usize;
                if slot < 4 {
                    let move_to = reg_v(ctx, ops[0] as usize);
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

fn bin_v(ctx: &mut ExecutionContext, ops: &[u32], f: impl Fn(Vec2, Vec2) -> Vec2) {
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
        _ => 0.0,
    }
}

fn reg_b(ctx: &ExecutionContext, i: usize) -> bool {
    match ctx.frame.registers.get(i).copied().unwrap_or(VmValue::Null) {
        VmValue::Bool(x) => x,
        _ => false,
    }
}

fn reg_v(ctx: &ExecutionContext, i: usize) -> Vec2 {
    match ctx.frame.registers.get(i).copied().unwrap_or(VmValue::Null) {
        VmValue::Vector(v) => v,
        _ => Vec2::ZERO,
    }
}

fn reg_eq(ctx: &ExecutionContext, a: usize, b: usize) -> bool {
    ctx.frame.registers.get(a).copied().unwrap_or(VmValue::Null)
        == ctx.frame.registers.get(b).copied().unwrap_or(VmValue::Null)
}

fn eval_operation(a: f32, kind: u32) -> f32 {
    match kind {
        0 => a.abs(),
        1 => a.round(),
        2 => a.floor(),
        3 => a.ceil(),
        4 => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        5 => a.max(0.0).sqrt(),
        6 => a.sin(),
        7 => a.cos(),
        8 => a.tan(),
        9 => a.ln(),
        10 => a.log10(),
        11 => a.exp(),
        12 => 10f32.powf(a),
        _ => a,
    }
}
