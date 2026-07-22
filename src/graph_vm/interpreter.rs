//! Intentionally dumb O0 interpreter — correctness / TRACE identity first.

use crate::graph_vm::context::ExecutionContext;
use crate::graph_vm::opcode::OpCode;
use crate::graph_vm::program::{Backend, RuntimeProgram};
use crate::graph_vm::value::VmValue;

#[derive(Debug, Default)]
pub struct Interpreter;

impl Backend for Interpreter {
    fn execute(&mut self, program: &RuntimeProgram, ctx: &mut ExecutionContext) {
        ctx.frame.ensure_regs(program.register_count as usize);
        for inst in program.ops.iter() {
            let ops = inst.operands.as_slice();
            match inst.opcode {
                OpCode::ConstNull => {
                    if let Some(&dst) = ops.first() {
                        ctx.frame.registers[dst as usize] = VmValue::Null;
                    }
                }
                OpCode::ConstFloat => {
                    // operands: [dst, imm_bits]
                    if ops.len() >= 2 {
                        let f = f32::from_bits(ops[1]);
                        ctx.frame.registers[ops[0] as usize] = VmValue::Float(f);
                    }
                }
                OpCode::ConstBool => {
                    if ops.len() >= 2 {
                        ctx.frame.registers[ops[0] as usize] = VmValue::Bool(ops[1] != 0);
                    }
                }
                OpCode::Add => bin_f(ctx, ops, |a, b| a + b),
                OpCode::Sub => bin_f(ctx, ops, |a, b| a - b),
                OpCode::Mul => bin_f(ctx, ops, |a, b| a * b),
                OpCode::Div => bin_f(ctx, ops, |a, b| if b.abs() < 1e-12 { 0.0 } else { a / b }),
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
                            ctx.state.vars[vid] = ctx.frame.registers[src];
                        }
                    }
                }
                // Remaining ops: no-op until 1:1 lower emits them (still preserves SID for verify).
                _ => {}
            }
        }
    }
}

fn bin_f(ctx: &mut ExecutionContext, ops: &[u32], f: impl Fn(f32, f32) -> f32) {
    // [dst, a, b]
    if ops.len() < 3 {
        return;
    }
    let a = match ctx.frame.registers[ops[1] as usize] {
        VmValue::Float(x) => x,
        _ => 0.0,
    };
    let b = match ctx.frame.registers[ops[2] as usize] {
        VmValue::Float(x) => x,
        _ => 0.0,
    };
    ctx.frame.registers[ops[0] as usize] = VmValue::Float(f(a, b));
}
