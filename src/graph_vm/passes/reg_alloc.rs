//! RegAlloc — compact sparse register numbers to dense 0..N-1.

use std::collections::HashMap;

use crate::graph_vm::ir::{LoweredIR, Reg};
use crate::graph_vm::opcode::OpCode;
use crate::graph_vm::passes::Pass;

#[derive(Debug, Default)]
pub struct RegAlloc;

impl Pass for RegAlloc {
    fn name(&self) -> &'static str {
        "RegAlloc"
    }

    fn run(&self, ir: &mut LoweredIR) {
        let mut used = Vec::<Reg>::new();
        for inst in &ir.instructions {
            if let Some(dest) = inst.dest {
                used.push(dest);
            }
            for (index, &arg) in inst.args.iter().enumerate() {
                if !is_variable_id_arg(inst.op, index) {
                    used.push(arg);
                }
            }
        }

        used.sort_by_key(|reg| reg.0);
        used.dedup();

        let remap: HashMap<Reg, Reg> = used
            .into_iter()
            .enumerate()
            .map(|(index, reg)| (reg, Reg(index as u32)))
            .collect();

        for inst in &mut ir.instructions {
            if let Some(dest) = inst.dest {
                if let Some(&mapped) = remap.get(&dest) {
                    inst.dest = Some(mapped);
                }
            }
            let op = inst.op;
            for (index, arg) in inst.args.iter_mut().enumerate() {
                if !is_variable_id_arg(op, index) {
                    if let Some(&mapped) = remap.get(arg) {
                        *arg = mapped;
                    }
                }
            }
        }
    }
}

fn is_variable_id_arg(op: OpCode, index: usize) -> bool {
    index == 0 && matches!(op, OpCode::LoadVar | OpCode::StoreVar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_vm::builder::ProgramBuilder;
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

    fn used_regs(ir: &LoweredIR) -> Vec<u32> {
        let mut regs = Vec::new();
        for inst in &ir.instructions {
            if let Some(dest) = inst.dest {
                regs.push(dest.0);
            }
            for (index, arg) in inst.args.iter().enumerate() {
                if !is_variable_id_arg(inst.op, index) {
                    regs.push(arg.0);
                }
            }
        }
        regs.sort_unstable();
        regs.dedup();
        regs
    }

    #[test]
    fn compacts_sparse_registers() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(2)), OpCode::Add, &[5, 9], "a"),
                inst(Some(Reg(9)), OpCode::Add, &[2, 5], "b"),
            ],
            ..LoweredIR::new()
        };

        RegAlloc.run(&mut ir);

        assert_eq!(used_regs(&ir), vec![0, 1, 2]);
        let packed = ProgramBuilder.pack_lowered(&ir, 0);
        assert_eq!(packed.register_count, 3);
    }

    #[test]
    fn preserves_load_store_variable_ids() {
        let mut ir = LoweredIR {
            instructions: vec![
                inst(Some(Reg(9)), OpCode::LoadVar, &[5], "load"),
                inst(None, OpCode::StoreVar, &[5, 9], "store"),
            ],
            ..LoweredIR::new()
        };

        RegAlloc.run(&mut ir);

        assert_eq!(ir.instructions[0].args, vec![Reg(5)]);
        assert_eq!(ir.instructions[0].dest, Some(Reg(0)));
        assert_eq!(ir.instructions[1].args, vec![Reg(5), Reg(0)]);
        assert_eq!(ir.instructions[1].source_sid, "store");
    }

    /// Optional AIA bench: report register_count before and after RegAlloc.
    #[test]
    fn aia_reg_alloc_register_count() {
        use crate::graph_vm::lower::Lowerer;
        use crate::graph_vm::passes::{ConstFold, Cse, Fusion, RelayRemoval};
        use std::path::PathBuf;

        let path = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r"AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\AIA.txt");
        if !path.exists() {
            eprintln!("skip: AIA.txt not found at {path:?}");
            return;
        }

        let raw = Lowerer::compile(crate::graph::load_team_graph(&path).expect("load AIA"));

        let mut before = raw.clone();
        ConstFold.run(&mut before.settle);
        ConstFold.run(&mut before.controllers);
        RelayRemoval.run(&mut before.settle);
        RelayRemoval.run(&mut before.controllers);
        Cse.run(&mut before.settle);
        Cse.run(&mut before.controllers);
        Fusion.run(&mut before.settle);
        Fusion.run(&mut before.controllers);

        let before_packed = ProgramBuilder.pack(&before);
        let before_count = before_packed.register_count;

        let mut after = before;
        RegAlloc.run(&mut after.settle);
        RegAlloc.run(&mut after.controllers);
        let after_packed = ProgramBuilder.pack(&after);
        let after_count = after_packed.register_count;

        eprintln!(
            "AIA O1 register_count: before RegAlloc {before_count} -> after RegAlloc {after_count}"
        );
        assert!(after_count <= before_count);
    }
}
