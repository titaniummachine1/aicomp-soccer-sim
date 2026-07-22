//! Fusion - conservative O1 compound operations.

use std::collections::{HashMap, HashSet};

use crate::graph_vm::ir::{LoweredIR, Reg};
use crate::graph_vm::opcode::OpCode;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct Fusion;

impl Pass for Fusion {
    fn name(&self) -> &'static str {
        "Fusion"
    }

    fn run(&self, ir: &mut LoweredIR) {
        let by_dest: HashMap<Reg, usize> = ir
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(index, inst)| inst.dest.map(|dest| (dest, index)))
            .collect();
        let mut use_count = HashMap::<Reg, usize>::new();
        for inst in &ir.instructions {
            for (index, &arg) in inst.args.iter().enumerate() {
                if !is_variable_id_arg(inst.op, index) {
                    *use_count.entry(arg).or_default() += 1;
                }
            }
        }

        let mut dead = HashSet::<Reg>::new();
        for index in 0..ir.instructions.len() {
            let (op, args) = {
                let inst = &ir.instructions[index];
                (inst.op, inst.args.clone())
            };

            match op {
                OpCode::AddVec if ir.instructions[index].dest.is_some() && args.len() == 2 => {
                    let fused =
                        scale_add_args(args[0], args[1], &ir.instructions, &by_dest, &use_count)
                            .or_else(|| {
                                scale_add_args(
                                    args[1],
                                    args[0],
                                    &ir.instructions,
                                    &by_dest,
                                    &use_count,
                                )
                            });
                    if let Some((scale_dest, fused_args)) = fused {
                        let inst = &mut ir.instructions[index];
                        inst.op = OpCode::ScaleAddVec;
                        inst.args = fused_args;
                        inst.immediates.clear();
                        dead.insert(scale_dest);
                    }
                }
                OpCode::Not if ir.instructions[index].dest.is_some() && args.len() == 1 => {
                    let Some(&producer_index) = by_dest.get(&args[0]) else {
                        continue;
                    };
                    if use_count.get(&args[0]).copied().unwrap_or(0) != 1 {
                        continue;
                    }
                    let producer = &ir.instructions[producer_index];
                    let fused_op = match (producer.op, producer.args.as_slice()) {
                        (OpCode::Or, [a, b]) => Some((OpCode::Nor, *a, *b)),
                        (OpCode::And, [a, b]) => Some((OpCode::Nand, *a, *b)),
                        _ => None,
                    };
                    if let Some((fused_op, a, b)) = fused_op {
                        let inst = &mut ir.instructions[index];
                        inst.op = fused_op;
                        inst.args = vec![a, b];
                        inst.immediates.clear();
                        dead.insert(args[0]);
                    }
                }
                _ => {}
            }
        }

        ir.instructions
            .retain(|inst| inst.dest.map(|dest| !dead.contains(&dest)).unwrap_or(true));
    }
}

fn scale_add_args(
    scaled_reg: Reg,
    addend: Reg,
    instructions: &[crate::graph_vm::ir::IrInst],
    by_dest: &HashMap<Reg, usize>,
    use_count: &HashMap<Reg, usize>,
) -> Option<(Reg, Vec<Reg>)> {
    if use_count.get(&scaled_reg).copied().unwrap_or(0) != 1 {
        return None;
    }
    let &scale_index = by_dest.get(&scaled_reg)?;
    let scale = &instructions[scale_index];
    let [vec, scalar] = scale.args.as_slice() else {
        return None;
    };
    (scale.op == OpCode::ScaleVec).then_some((scaled_reg, vec![*vec, *scalar, addend]))
}

fn is_variable_id_arg(op: OpCode, index: usize) -> bool {
    index == 0 && matches!(op, OpCode::LoadVar | OpCode::StoreVar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_vm::ir::{IrInst, LoweredIR};
    use crate::graph_vm::value::RegisterKind;

    fn inst(dest: Option<Reg>, op: OpCode, args: &[u32], sid: &str) -> IrInst {
        IrInst {
            dest,
            kind: RegisterKind::Float,
            op,
            args: args.iter().copied().map(Reg).collect(),
            immediates: vec![99],
            source_sid: sid.into(),
            source_port: "Float1".into(),
        }
    }

    #[test]
    fn fuses_scale_then_add_vec() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::ScaleVec, &[0, 1], "scale"),
                inst(Some(Reg(3)), OpCode::AddVec, &[4, 2], "add"),
            ],
            ..LoweredIR::new()
        };

        Fusion.run(&mut ir);

        assert_eq!(ir.instructions.len(), 1);
        assert_eq!(ir.instructions[0].op, OpCode::ScaleAddVec);
        assert_eq!(ir.instructions[0].dest, Some(Reg(3)));
        assert_eq!(ir.instructions[0].args, vec![Reg(0), Reg(1), Reg(4)]);
        assert!(ir.instructions[0].immediates.is_empty());
        assert_eq!(ir.instructions[0].source_sid, "add");
    }

    #[test]
    fn fuses_add_vec_with_scaled_lhs() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::ScaleVec, &[0, 1], "scale"),
                inst(Some(Reg(3)), OpCode::AddVec, &[2, 4], "add"),
            ],
            ..LoweredIR::new()
        };

        Fusion.run(&mut ir);

        assert_eq!(ir.instructions.len(), 1);
        assert_eq!(ir.instructions[0].op, OpCode::ScaleAddVec);
        assert_eq!(ir.instructions[0].args, vec![Reg(0), Reg(1), Reg(4)]);
    }

    #[test]
    fn does_not_fuse_multiuse_scale() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::ScaleVec, &[0, 1], "scale"),
                inst(Some(Reg(3)), OpCode::AddVec, &[4, 2], "add1"),
                inst(Some(Reg(5)), OpCode::AddVec, &[2, 6], "add2"),
            ],
            ..LoweredIR::new()
        };

        Fusion.run(&mut ir);

        assert_eq!(ir.instructions.len(), 3);
        assert!(ir
            .instructions
            .iter()
            .all(|inst| inst.op != OpCode::ScaleAddVec));
    }

    #[test]
    fn fuses_not_or_into_nor() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::Or, &[0, 1], "or"),
                inst(Some(Reg(3)), OpCode::Not, &[2], "not"),
            ],
            ..LoweredIR::new()
        };

        Fusion.run(&mut ir);

        assert_eq!(ir.instructions.len(), 1);
        assert_eq!(ir.instructions[0].op, OpCode::Nor);
        assert_eq!(ir.instructions[0].args, vec![Reg(0), Reg(1)]);
        assert_eq!(ir.instructions[0].source_sid, "not");
    }

    #[test]
    fn fuses_not_and_into_nand() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::And, &[0, 1], "and"),
                inst(Some(Reg(3)), OpCode::Not, &[2], "not"),
            ],
            ..LoweredIR::new()
        };

        Fusion.run(&mut ir);

        assert_eq!(ir.instructions.len(), 1);
        assert_eq!(ir.instructions[0].op, OpCode::Nand);
        assert_eq!(ir.instructions[0].args, vec![Reg(0), Reg(1)]);
    }

    /// Optional AIA bench: report instruction totals through Fusion.
    #[test]
    fn aia_fusion_instruction_mix() {
        use crate::graph_vm::lower::Lowerer;
        use crate::graph_vm::passes::{ConstFold, Cse, RelayRemoval};
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        let raw = Lowerer::compile(crate::graph::load_team_graph(&path).expect("load AIA"));
        let total = |compiled: &crate::graph_vm::lower::CompileResult| {
            compiled.settle.instructions.len() + compiled.controllers.instructions.len()
        };
        let before = total(&raw);

        let mut after_relay = raw.clone();
        ConstFold.run(&mut after_relay.settle);
        ConstFold.run(&mut after_relay.controllers);
        RelayRemoval.run(&mut after_relay.settle);
        RelayRemoval.run(&mut after_relay.controllers);
        let after_relay_count = total(&after_relay);

        let mut after_cse = after_relay.clone();
        Cse.run(&mut after_cse.settle);
        Cse.run(&mut after_cse.controllers);
        let cse_count = total(&after_cse);

        let mut after_fusion = after_cse.clone();
        Fusion.run(&mut after_fusion.settle);
        Fusion.run(&mut after_fusion.controllers);
        let fusion_count = total(&after_fusion);
        let fusion_removed = cse_count - fusion_count;

        eprintln!(
            "AIA O1 instruction totals: raw {before} -> \
             ConstFold+RelayRemoval {after_relay_count} -> \
             +CSE {cse_count} -> +Fusion {fusion_count} \
             (removed {fusion_removed})"
        );
        assert!(after_relay_count <= before);
        assert!(cse_count <= after_relay_count);
        assert!(fusion_count <= cse_count);
    }
}
