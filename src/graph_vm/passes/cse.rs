//! CSE — eliminate equivalent Pure/ReadOnly producers within one program.

use std::collections::{HashMap, HashSet};

use crate::graph_vm::ir::{LoweredIR, Reg};
use crate::graph_vm::opcode::{OpCode, OpEffect};
use crate::graph_vm::passes::Pass;
use crate::graph_vm::value::RegisterKind;

#[derive(Debug, Default)]
pub struct Cse;

impl Pass for Cse {
    fn name(&self) -> &'static str {
        "CSE"
    }

    fn run(&self, ir: &mut LoweredIR) {
        let mut available = HashMap::<ExprKey, Reg>::new();
        let mut aliases = HashMap::<Reg, Reg>::new();
        let mut dead = HashSet::<Reg>::new();

        for inst in &mut ir.instructions {
            let op = inst.op;
            for (index, arg) in inst.args.iter_mut().enumerate() {
                if !is_variable_id_arg(op, index) {
                    *arg = resolve_alias(*arg, &aliases);
                }
            }

            // A StoreVar changes only the named variable. Keep unrelated
            // LoadVar expressions available, but never reuse this variable's
            // value across the write.
            if op == OpCode::StoreVar {
                if let Some(&variable) = inst.args.first() {
                    available.retain(|key, _| {
                        !(key.op == OpCode::LoadVar
                            && key.args.first().copied() == Some(variable))
                    });
                }
            }

            if !matches!(op.effect(), OpEffect::Pure | OpEffect::ReadOnly) {
                continue;
            }
            let Some(dest) = inst.dest else {
                continue;
            };

            let key = ExprKey {
                op,
                args: inst.args.clone(),
                immediates: inst.immediates.clone(),
                kind: inst.kind,
            };
            if let Some(&available_dest) = available.get(&key) {
                aliases.insert(dest, available_dest);
                dead.insert(dest);
            } else {
                available.insert(key, dest);
            }
        }

        ir.instructions.retain(|inst| {
            inst.dest.map(|dest| !dead.contains(&dest)).unwrap_or(true)
        });

        // The scan remaps every later use as it goes. This final rewrite also
        // handles any non-standard IR where a use was ordered before a
        // duplicate producer was encountered.
        for inst in &mut ir.instructions {
            let op = inst.op;
            for (index, arg) in inst.args.iter_mut().enumerate() {
                if !is_variable_id_arg(op, index) {
                    *arg = resolve_alias(*arg, &aliases);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExprKey {
    op: OpCode,
    args: Vec<Reg>,
    immediates: Vec<u32>,
    kind: RegisterKind,
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
    use crate::graph_vm::passes::{ConstFold, RelayRemoval};
    use crate::graph_vm::value::RegisterKind;

    fn inst(dest: Option<Reg>, op: OpCode, args: &[u32], sid: &str) -> IrInst {
        inst_with_immediates(dest, op, args, &[], sid)
    }

    fn inst_with_immediates(
        dest: Option<Reg>,
        op: OpCode,
        args: &[u32],
        immediates: &[u32],
        sid: &str,
    ) -> IrInst {
        IrInst {
            dest,
            kind: RegisterKind::Float,
            op,
            args: args.iter().copied().map(Reg).collect(),
            immediates: immediates.to_vec(),
            source_sid: sid.into(),
            source_port: "Float1".into(),
        }
    }

    #[test]
    fn cse_identical_adds() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::Add, &[0, 1], "add1"),
                inst(Some(Reg(3)), OpCode::Add, &[0, 1], "add2"),
                inst(Some(Reg(4)), OpCode::Add, &[3, 0], "consumer"),
            ],
            ..LoweredIR::new()
        };

        Cse.run(&mut ir);

        assert_eq!(ir.instructions.len(), 2);
        assert_eq!(ir.instructions[0].dest, Some(Reg(2)));
        assert_eq!(ir.instructions[0].source_sid, "add1");
        assert_eq!(ir.instructions[1].op, OpCode::Add);
        assert_eq!(ir.instructions[1].args, vec![Reg(2), Reg(0)]);
        assert_eq!(ir.instructions[1].source_sid, "consumer");
    }

    #[test]
    fn cse_identical_load_api() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst_with_immediates(Some(Reg(0)), OpCode::LoadApi, &[], &[7], "api1"),
                inst_with_immediates(Some(Reg(1)), OpCode::LoadApi, &[], &[7], "api2"),
                inst(Some(Reg(2)), OpCode::Add, &[1, 1], "consumer"),
            ],
            ..LoweredIR::new()
        };

        Cse.run(&mut ir);

        assert_eq!(ir.instructions.len(), 2);
        assert_eq!(ir.instructions[0].dest, Some(Reg(0)));
        assert_eq!(ir.instructions[0].source_sid, "api1");
        assert_eq!(ir.instructions[1].args, vec![Reg(0), Reg(0)]);
    }

    #[test]
    fn does_not_cse_store_var_or_external() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(None, OpCode::StoreVar, &[5, 0], "store1"),
                inst(None, OpCode::StoreVar, &[5, 0], "store2"),
                inst(None, OpCode::EmitController, &[0, 1, 2], "emit1"),
                inst(None, OpCode::EmitController, &[0, 1, 2], "emit2"),
            ],
            ..LoweredIR::new()
        };

        Cse.run(&mut ir);

        assert_eq!(ir.instructions.len(), 4);
        assert_eq!(
            ir.instructions
                .iter()
                .filter(|inst| inst.op == OpCode::StoreVar)
                .count(),
            2
        );
        assert_eq!(
            ir.instructions
                .iter()
                .filter(|inst| inst.op == OpCode::EmitController)
                .count(),
            2
        );
    }

    #[test]
    fn load_var_cse_is_invalidated_by_store() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(0)), OpCode::LoadVar, &[5], "load1"),
                inst(None, OpCode::StoreVar, &[5, 1], "store"),
                inst(Some(Reg(2)), OpCode::LoadVar, &[5], "load2"),
            ],
            ..LoweredIR::new()
        };

        Cse.run(&mut ir);

        assert_eq!(ir.instructions.len(), 3);
        assert_eq!(
            ir.instructions
                .iter()
                .filter(|inst| inst.op == OpCode::LoadVar)
                .count(),
            2
        );
    }

    /// Optional AIA bench: report raw, O1-before-CSE, and O1 instruction totals.
    #[test]
    fn aia_cse_instruction_mix() {
        use crate::graph_vm::lower::Lowerer;
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
        let after_cse_count = total(&after_cse);

        eprintln!(
            "AIA O1 instruction totals: raw {before} -> \
             ConstFold+RelayRemoval {after_relay_count} -> \
             +CSE {after_cse_count}"
        );
        assert!(after_relay_count <= before);
        assert!(after_cse_count <= after_relay_count);
    }
}
