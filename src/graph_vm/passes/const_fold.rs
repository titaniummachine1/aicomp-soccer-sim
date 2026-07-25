//! ConstFold — Pure ops with Const* inputs → Const*.
//!
//! Mirrors interpreter evaluation (including Select/Operation immediates and
//! reg_f/reg_b/reg_v coercion). Does not DCE dead Const* producers.

use bevy::prelude::Vec3;

use crate::graph_vm::ir::{IrInst, LoweredIR, Reg};
use crate::graph_vm::opcode::{OpCode, OpEffect};
use crate::graph_vm::passes::Pass;
use crate::graph_vm::value::{RegisterKind, VmValue};

#[derive(Debug, Default)]
pub struct ConstFold;

impl Pass for ConstFold {
    fn name(&self) -> &'static str {
        "ConstFold"
    }

    fn run(&self, ir: &mut LoweredIR) {
        // Sparse map: index = reg id. None = unknown / not yet written as const.
        let mut known: Vec<Option<VmValue>> = Vec::new();

        for inst in &mut ir.instructions {
            if let Some(v) = const_value_of(inst) {
                if let Some(Reg(d)) = inst.dest {
                    set_known(&mut known, d, v);
                }
                continue;
            }

            if inst.op.effect() != OpEffect::Pure {
                continue;
            }

            let Some(folded) = try_fold(inst, &known) else {
                continue;
            };

            let dest = inst.dest;
            let source_sid = std::mem::take(&mut inst.source_sid);
            let source_port = std::mem::take(&mut inst.source_port);
            *inst = make_const(dest, folded, source_sid, source_port);
            if let Some(Reg(d)) = dest {
                set_known(&mut known, d, folded);
            }
        }
    }
}

fn set_known(known: &mut Vec<Option<VmValue>>, reg: u32, v: VmValue) {
    let i = reg as usize;
    if known.len() <= i {
        known.resize(i + 1, None);
    }
    known[i] = Some(v);
}

fn get_known(known: &[Option<VmValue>], reg: Reg) -> Option<VmValue> {
    known.get(reg.0 as usize).copied().flatten()
}

fn const_value_of(inst: &IrInst) -> Option<VmValue> {
    match inst.op {
        OpCode::ConstNull => Some(VmValue::Null),
        OpCode::ConstFloat => {
            let bits = *inst.immediates.first()?;
            Some(VmValue::Float(f32::from_bits(bits)))
        }
        OpCode::ConstBool => {
            let b = inst.immediates.first().copied().unwrap_or(0) != 0;
            Some(VmValue::Bool(b))
        }
        OpCode::ConstVec => {
            let x = f32::from_bits(*inst.immediates.first()?);
            let y = f32::from_bits(*inst.immediates.get(1)?);
            let z = f32::from_bits(*inst.immediates.get(2)?);
            Some(VmValue::Vector(Vec3::new(x, y, z)))
        }
        _ => None,
    }
}

fn make_const(
    dest: Option<Reg>,
    value: VmValue,
    source_sid: String,
    source_port: String,
) -> IrInst {
    match value {
        VmValue::Float(f) => IrInst {
            dest,
            kind: RegisterKind::Float,
            op: OpCode::ConstFloat,
            args: vec![],
            immediates: vec![f.to_bits()],
            source_sid,
            source_port,
        },
        VmValue::Bool(b) => IrInst {
            dest,
            kind: RegisterKind::Bool,
            op: OpCode::ConstBool,
            args: vec![],
            immediates: vec![if b { 1 } else { 0 }],
            source_sid,
            source_port,
        },
        VmValue::Vector(v) => IrInst {
            dest,
            kind: RegisterKind::Vector,
            op: OpCode::ConstVec,
            args: vec![],
            immediates: vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()],
            source_sid,
            source_port,
        },
        VmValue::Null => IrInst {
            dest,
            kind: RegisterKind::Null,
            op: OpCode::ConstNull,
            args: vec![],
            immediates: vec![],
            source_sid,
            source_port,
        },
    }
}

fn try_fold(inst: &IrInst, known: &[Option<VmValue>]) -> Option<VmValue> {
    let args: Vec<VmValue> = inst
        .args
        .iter()
        .map(|&r| get_known(known, r))
        .collect::<Option<Vec<_>>>()?;

    match inst.op {
        OpCode::Move if args.len() >= 1 => args.first().copied(),
        OpCode::IsNull if args.len() >= 1 => {
            Some(VmValue::Bool(args[0] == VmValue::Null))
        }
        OpCode::Add if args.len() >= 2 => Some(VmValue::Float(as_f(args[0]) + as_f(args[1]))),
        OpCode::Sub if args.len() >= 2 => Some(VmValue::Float(as_f(args[0]) - as_f(args[1]))),
        OpCode::Mul if args.len() >= 2 => Some(VmValue::Float(as_f(args[0]) * as_f(args[1]))),
        OpCode::Div if args.len() >= 2 => {
            let b = as_f(args[1]);
            Some(VmValue::Float(if b.abs() < 1e-12 {
                0.0
            } else {
                as_f(args[0]) / b
            }))
        }
        OpCode::Mod if args.len() >= 2 => {
            let b = as_f(args[1]);
            Some(VmValue::Float(if b.abs() < 1e-12 {
                0.0
            } else {
                as_f(args[0]) % b
            }))
        }
        OpCode::Pow if args.len() >= 2 => {
            Some(VmValue::Float(as_f(args[0]).powf(as_f(args[1]))))
        }
        OpCode::Lerp if args.len() >= 3 => {
            let a = as_f(args[0]);
            let b = as_f(args[1]);
            let t = as_f(args[2]);
            Some(VmValue::Float(a + (b - a) * t))
        }
        OpCode::Abs if args.len() >= 1 => Some(VmValue::Float(as_f(args[0]).abs())),
        OpCode::Operation if args.len() >= 1 => {
            let kind = inst.immediates.first().copied().unwrap_or(0);
            Some(VmValue::Float(crate::graph::dropdowns::eval_operation(as_f(args[0]), kind)))
        }
        OpCode::Not if args.len() >= 1 => Some(VmValue::Bool(!as_b(args[0]))),
        OpCode::And if args.len() >= 2 => Some(VmValue::Bool(as_b(args[0]) && as_b(args[1]))),
        OpCode::Or if args.len() >= 2 => Some(VmValue::Bool(as_b(args[0]) || as_b(args[1]))),
        OpCode::Nor if args.len() >= 2 => {
            Some(VmValue::Bool(!(as_b(args[0]) || as_b(args[1]))))
        }
        OpCode::Nand if args.len() >= 2 => {
            Some(VmValue::Bool(!(as_b(args[0]) && as_b(args[1]))))
        }
        OpCode::Lt if args.len() >= 2 => Some(VmValue::Bool(as_f(args[0]) < as_f(args[1]))),
        OpCode::Gt if args.len() >= 2 => Some(VmValue::Bool(as_f(args[0]) > as_f(args[1]))),
        OpCode::Le if args.len() >= 2 => Some(VmValue::Bool(as_f(args[0]) <= as_f(args[1]))),
        OpCode::Ge if args.len() >= 2 => Some(VmValue::Bool(as_f(args[0]) >= as_f(args[1]))),
        OpCode::Eq if args.len() >= 2 => {
            let imm = inst.immediates.first().copied().unwrap_or(0);
            let r = if imm == 1 {
                (as_f(args[0]) - as_f(args[1])).abs() < 1e-5
            } else {
                args[0] == args[1]
            };
            Some(VmValue::Bool(r))
        }
        OpCode::Ne if args.len() >= 2 => Some(VmValue::Bool(args[0] != args[1])),
        OpCode::ConstructVec if args.len() >= 3 => Some(VmValue::Vector(Vec3::new(
            as_f(args[0]),
            as_f(args[1]),
            as_f(args[2]),
        ))),
        OpCode::SplitVec if args.len() >= 1 => {
            let v = as_v(args[0]);
            let axis = inst.immediates.first().copied().unwrap_or(0);
            let f = match axis {
                0 => v.x,
                1 => v.y,
                _ => v.z,
            };
            Some(VmValue::Float(f))
        }
        OpCode::AddVec if args.len() >= 2 => {
            Some(VmValue::Vector(as_v(args[0]) + as_v(args[1])))
        }
        OpCode::SubVec if args.len() >= 2 => {
            Some(VmValue::Vector(as_v(args[0]) - as_v(args[1])))
        }
        OpCode::ScaleVec if args.len() >= 2 => {
            Some(VmValue::Vector(as_v(args[0]) * as_f(args[1])))
        }
        OpCode::ScaleAddVec if args.len() >= 3 => {
            let scaled = as_v(args[0]) * as_f(args[1]);
            Some(VmValue::Vector(scaled + as_v(args[2])))
        }
        OpCode::Normalize if args.len() >= 1 => {
            let v = as_v(args[0]);
            let len = v.length();
            Some(VmValue::Vector(if len > 1e-8 {
                v / len
            } else {
                Vec3::ZERO
            }))
        }
        OpCode::Magnitude if args.len() >= 1 => Some(VmValue::Float(as_v(args[0]).length())),
        OpCode::Distance if args.len() >= 2 => {
            Some(VmValue::Float(as_v(args[0]).distance(as_v(args[1]))))
        }
        OpCode::Dot if args.len() >= 2 => {
            Some(VmValue::Float(as_v(args[0]).dot(as_v(args[1]))))
        }
        OpCode::Select if args.len() >= 3 => {
            let cond = as_b(args[0]);
            let chosen = if cond { args[1] } else { args[2] };
            let coerce = inst.immediates.first().copied().unwrap_or(3);
            Some(coerce_select(chosen, coerce))
        }
        // Min/Max/Clamp/Neg exist in ABI but are not executed by the O0 interpreter yet.
        _ => None,
    }
}

fn as_f(v: VmValue) -> f32 {
    match v {
        VmValue::Float(x) => x,
        VmValue::Bool(b) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        VmValue::Vector(value) => value.length(),
        VmValue::Null => 0.0,
    }
}

fn as_b(v: VmValue) -> bool {
    match v {
        VmValue::Bool(x) => x,
        VmValue::Float(value) => value != 0.0,
        VmValue::Vector(_) | VmValue::Null => false,
    }
}

fn as_v(v: VmValue) -> Vec3 {
    match v {
        VmValue::Vector(v) => v,
        _ => Vec3::ZERO,
    }
}

fn coerce_select(chosen: VmValue, mode: u32) -> VmValue {
    match mode {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_vm::ir::Reg;
    use crate::graph_vm::opcode::OpCode;
    use crate::graph_vm::value::RegisterKind;

    #[test]
    fn folds_add_of_two_const_floats() {
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

        ConstFold.run(&mut ir);

        assert_eq!(ir.instructions[2].op, OpCode::ConstFloat);
        assert!(ir.instructions[2].args.is_empty());
        assert_eq!(
            ir.instructions[2].immediates,
            vec![6.5_f32.to_bits()]
        );
        assert_eq!(ir.instructions[2].source_sid, "add");
        // Producers remain (no DCE in this pass).
        assert_eq!(ir.instructions[0].op, OpCode::ConstFloat);
        assert_eq!(ir.instructions[1].op, OpCode::ConstFloat);
    }

    #[test]
    fn folds_select_with_operation_immediates() {
        let mut ir = LoweredIR::new();
        ir.instructions = vec![
            IrInst {
                dest: Some(Reg(0)),
                kind: RegisterKind::Bool,
                op: OpCode::ConstBool,
                args: vec![],
                immediates: vec![1],
                source_sid: "c".into(),
                source_port: "Bool1".into(),
            },
            IrInst {
                dest: Some(Reg(1)),
                kind: RegisterKind::Float,
                op: OpCode::ConstFloat,
                args: vec![],
                immediates: vec![(-3.2_f32).to_bits()],
                source_sid: "t".into(),
                source_port: "Float1".into(),
            },
            IrInst {
                dest: Some(Reg(2)),
                kind: RegisterKind::Float,
                op: OpCode::ConstFloat,
                args: vec![],
                immediates: vec![9.0_f32.to_bits()],
                source_sid: "f".into(),
                source_port: "Float2".into(),
            },
            IrInst {
                dest: Some(Reg(3)),
                kind: RegisterKind::Float,
                op: OpCode::Select,
                args: vec![Reg(0), Reg(1), Reg(2)],
                immediates: vec![0], // float coerce
                source_sid: "sel".into(),
                source_port: "Float1".into(),
            },
            IrInst {
                dest: Some(Reg(4)),
                kind: RegisterKind::Float,
                op: OpCode::Operation,
                args: vec![Reg(3)],
                immediates: vec![0], // abs
                source_sid: "op".into(),
                source_port: "Float1".into(),
            },
        ];

        ConstFold.run(&mut ir);

        assert_eq!(ir.instructions[3].op, OpCode::ConstFloat);
        assert_eq!(
            ir.instructions[3].immediates,
            vec![(-3.2_f32).to_bits()]
        );
        assert_eq!(ir.instructions[4].op, OpCode::ConstFloat);
        assert_eq!(ir.instructions[4].immediates, vec![3.2_f32.to_bits()]);
    }

    /// Optional AIA bench: instruction mix before/after ConstFold (skips if AIA absent).
    #[test]
    fn aia_const_fold_instruction_mix() {
        use crate::graph_vm::lower::Lowerer;
        use crate::graph_vm::opcode::OpCode;
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        fn is_const(op: OpCode) -> bool {
            matches!(
                op,
                OpCode::ConstFloat | OpCode::ConstBool | OpCode::ConstVec | OpCode::ConstNull
            )
        }
        fn is_foldable_compute(op: OpCode) -> bool {
            matches!(
                op,
                OpCode::Add
                    | OpCode::Sub
                    | OpCode::Mul
                    | OpCode::Div
                    | OpCode::Mod
                    | OpCode::Pow
                    | OpCode::Lerp
                    | OpCode::Abs
                    | OpCode::Operation
                    | OpCode::Not
                    | OpCode::And
                    | OpCode::Or
                    | OpCode::Nor
                    | OpCode::Nand
                    | OpCode::Lt
                    | OpCode::Gt
                    | OpCode::Le
                    | OpCode::Ge
                    | OpCode::Eq
                    | OpCode::Ne
                    | OpCode::ConstructVec
                    | OpCode::SplitVec
                    | OpCode::AddVec
                    | OpCode::SubVec
                    | OpCode::ScaleVec
                    | OpCode::ScaleAddVec
                    | OpCode::Normalize
                    | OpCode::Magnitude
                    | OpCode::Distance
                    | OpCode::Dot
                    | OpCode::Select
                    | OpCode::IsNull
                    | OpCode::Move
            )
        }

        let graph = crate::graph::load_team_graph(&path).expect("load AIA");
        let mut compiled = Lowerer::compile(graph);
        let before_total =
            compiled.settle.instructions.len() + compiled.controllers.instructions.len();
        let before_const = compiled
            .settle
            .instructions
            .iter()
            .chain(compiled.controllers.instructions.iter())
            .filter(|i| is_const(i.op))
            .count();
        let before_compute = compiled
            .settle
            .instructions
            .iter()
            .chain(compiled.controllers.instructions.iter())
            .filter(|i| is_foldable_compute(i.op))
            .count();

        ConstFold.run(&mut compiled.settle);
        ConstFold.run(&mut compiled.controllers);

        let after_total =
            compiled.settle.instructions.len() + compiled.controllers.instructions.len();
        let after_const = compiled
            .settle
            .instructions
            .iter()
            .chain(compiled.controllers.instructions.iter())
            .filter(|i| is_const(i.op))
            .count();
        let after_compute = compiled
            .settle
            .instructions
            .iter()
            .chain(compiled.controllers.instructions.iter())
            .filter(|i| is_foldable_compute(i.op))
            .count();

        eprintln!(
            "AIA ConstFold: total {before_total}->{after_total} (in-place); \
             Const* {before_const}->{after_const}; \
             foldable-compute {before_compute}->{after_compute} (Δ {})",
            before_compute as i64 - after_compute as i64
        );
        assert_eq!(before_total, after_total, "ConstFold replaces in place; no DCE");
        assert!(
            after_const >= before_const,
            "folding should only increase Const* count"
        );
        assert!(after_compute <= before_compute);
    }
}
