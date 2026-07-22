//! RelayRemoval — rewrite copy chains to their ultimate source.

use std::collections::{HashMap, HashSet};

use crate::graph_vm::ir::{LoweredIR, Reg};
use crate::graph_vm::opcode::OpCode;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct RelayRemoval;

impl Pass for RelayRemoval {
    fn name(&self) -> &'static str {
        "RelayRemoval"
    }

    fn run(&self, ir: &mut LoweredIR) {
        let mut direct_aliases = HashMap::new();
        for inst in &ir.instructions {
            if inst.op == OpCode::Move {
                if let (Some(dest), [source]) = (inst.dest, inst.args.as_slice()) {
                    direct_aliases.insert(dest, *source);
                }
            }
        }

        // Resolve once up front so each Move destination maps to its ultimate
        // source. `resolve_alias` also terminates if malformed IR contains a
        // cycle.
        let mut aliases = HashMap::with_capacity(direct_aliases.len());
        for &dest in direct_aliases.keys() {
            aliases.insert(dest, resolve_alias(dest, &direct_aliases));
        }

        // Only remap known Move destinations. Other Reg-shaped operands, such
        // as StoreVar/LoadVar variable IDs, are not registers in this IR.
        for inst in &mut ir.instructions {
            let op = inst.op;
            for (index, arg) in inst.args.iter_mut().enumerate() {
                if !is_variable_id_arg(op, index) && aliases.contains_key(arg) {
                    *arg = resolve_alias(*arg, &aliases);
                }
            }
        }

        let mut use_count = HashMap::<Reg, usize>::new();
        for inst in &ir.instructions {
            for (index, &arg) in inst.args.iter().enumerate() {
                if !is_variable_id_arg(inst.op, index) {
                    *use_count.entry(arg).or_default() += 1;
                }
            }
        }

        ir.instructions.retain(|inst| {
            if inst.op != OpCode::Move {
                return true;
            }
            let Some(dest) = inst.dest else {
                return true;
            };
            use_count.get(&dest).copied().unwrap_or(0) != 0
        });
    }
}

fn resolve_alias(reg: Reg, aliases: &HashMap<Reg, Reg>) -> Reg {
    let mut current = reg;
    let mut visited = HashSet::new();

    while let Some(&next) = aliases.get(&current) {
        if !visited.insert(current) {
            break;
        }
        current = next;
    }

    current
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
            immediates: vec![],
            source_sid: sid.into(),
            source_port: "Float1".into(),
        }
    }

    #[test]
    fn removes_dead_move_and_rewrites_consumer() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(0)), OpCode::ConstFloat, &[], "const"),
                inst(Some(Reg(1)), OpCode::Move, &[0], "relay"),
                inst(Some(Reg(2)), OpCode::Add, &[1, 0], "consumer"),
            ],
            ..LoweredIR::new()
        };

        RelayRemoval.run(&mut ir);

        assert_eq!(ir.instructions.len(), 2);
        assert_eq!(ir.instructions[1].op, OpCode::Add);
        assert_eq!(ir.instructions[1].args, vec![Reg(0), Reg(0)]);
        assert_eq!(ir.instructions[1].source_sid, "consumer");
    }

    #[test]
    fn folds_move_chain_to_ultimate_source() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(0)), OpCode::ConstFloat, &[], "const"),
                inst(Some(Reg(1)), OpCode::Move, &[0], "relay1"),
                inst(Some(Reg(2)), OpCode::Move, &[1], "relay2"),
                inst(Some(Reg(3)), OpCode::Add, &[2, 0], "consumer"),
            ],
            ..LoweredIR::new()
        };

        RelayRemoval.run(&mut ir);

        assert_eq!(ir.instructions.len(), 2);
        assert!(ir.instructions.iter().all(|i| i.op != OpCode::Move));
        assert_eq!(ir.instructions[1].args, vec![Reg(0), Reg(0)]);
    }

    #[test]
    fn leaves_variable_id_operands_unmapped_unless_they_are_move_destinations() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(0)), OpCode::ConstFloat, &[], "const"),
                inst(Some(Reg(7)), OpCode::Move, &[0], "relay"),
                inst(None, OpCode::StoreVar, &[7, 7], "store"),
            ],
            ..LoweredIR::new()
        };

        RelayRemoval.run(&mut ir);

        assert_eq!(ir.instructions.len(), 2);
        assert_eq!(ir.instructions[1].op, OpCode::StoreVar);
        assert_eq!(ir.instructions[1].args, vec![Reg(7), Reg(0)]);
    }

    #[test]
    fn cycle_in_move_aliases_does_not_loop() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(1)), OpCode::Move, &[2], "relay1"),
                inst(Some(Reg(2)), OpCode::Move, &[1], "relay2"),
                inst(Some(Reg(3)), OpCode::Add, &[1, 2], "consumer"),
            ],
            ..LoweredIR::new()
        };

        RelayRemoval.run(&mut ir);

        assert_eq!(ir.instructions.len(), 3);
        assert_eq!(ir.instructions[2].args, vec![Reg(1), Reg(2)]);
    }

    /// Optional AIA bench: instruction mix before/after RelayRemoval.
    #[test]
    fn aia_relay_removal_instruction_mix() {
        use crate::graph_vm::lower::Lowerer;
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        let graph = crate::graph::load_team_graph(&path).expect("load AIA");
        let mut compiled = Lowerer::compile(graph);
        let count = |ir: &LoweredIR| {
            (
                ir.instructions.len(),
                ir.instructions
                    .iter()
                    .filter(|inst| inst.op == OpCode::Move)
                    .count(),
            )
        };
        let (before_total, before_moves) = count(&compiled.settle);
        let controller_counts = count(&compiled.controllers);
        let before_total = before_total + controller_counts.0;
        let before_moves = before_moves + controller_counts.1;

        RelayRemoval.run(&mut compiled.settle);
        RelayRemoval.run(&mut compiled.controllers);

        let (after_total, after_moves) = count(&compiled.settle);
        let controller_counts = count(&compiled.controllers);
        let after_total = after_total + controller_counts.0;
        let after_moves = after_moves + controller_counts.1;

        eprintln!(
            "AIA RelayRemoval: total {before_total}->{after_total}; \
             Move {before_moves}->{after_moves}"
        );
        assert!(after_total <= before_total);
        assert!(after_moves <= before_moves);
    }
}
