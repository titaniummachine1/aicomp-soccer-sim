//! Run Unity `OperationProbe.txt` through sim Operation eval (same cases as game TimePlot).

#[cfg(test)]
mod operation_probe_sim {
    use std::path::PathBuf;

    use crate::graph::dropdowns::{
        eval_operation, operation_kind, OP_ABS, OP_ACOS, OP_ASIN, OP_ATAN, OP_CEIL, OP_COS,
        OP_EXP, OP_FLOOR, OP_LN, OP_LOG10, OP_POW10, OP_ROUND, OP_SIGN, OP_SIN, OP_SQRT, OP_TAN,
    };
    use crate::graph::load_team_graph;
    use crate::graph_vm::builder::ProgramBuilder;
    use crate::graph_vm::lower::Lowerer;

    fn probe_paths() -> Vec<PathBuf> {
        let mut v = Vec::new();
        v.push(PathBuf::from("data/reference/OperationProbe.txt"));
        if let Ok(home) = std::env::var("USERPROFILE") {
            v.push(
                PathBuf::from(home)
                    .join("AppData/LocalLow/Unicorn One/AIComp/Saves/Soccer/OperationProbe.txt"),
            );
        }
        v
    }

    fn find_probe() -> PathBuf {
        probe_paths()
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| {
                panic!(
                    "OperationProbe.txt not found. Run: python worldcupteams/probes/build_operation_probe.py"
                )
            })
    }

    fn python_expect(kind: u32, x: f32) -> f32 {
        match kind {
            OP_ABS => x.abs(),
            OP_ROUND => x.round(),
            OP_FLOOR => x.floor(),
            OP_CEIL => x.ceil(),
            OP_SIN => x.sin(),
            OP_COS => x.cos(),
            OP_TAN => x.tan(),
            OP_ASIN => x.asin(),
            OP_ACOS => x.acos(),
            OP_ATAN => x.atan(),
            OP_SQRT => x.max(0.0).sqrt(),
            OP_SIGN => {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            OP_LN => x.ln(),
            OP_LOG10 => x.log10(),
            OP_EXP => x.exp(),
            OP_POW10 => 10f32.powf(x),
            _ => panic!("bad kind {kind}"),
        }
    }

    #[test]
    fn operation_probe_txt_all_ops_match_python() {
        let path = find_probe();
        let g = load_team_graph(&path).expect("load OperationProbe");

        let mut rows = Vec::new();
        for node in g.nodes.values().filter(|n| n.id == "Operation") {
            let kind = operation_kind(&node.modifier);
            let in_port = node
                .ports
                .iter()
                .find(|p| p.id == "Float1" && p.polarity == 0)
                .expect("Operation Float1 in");
            let src_sid = g
                .input_source
                .get(&in_port.sid)
                .expect("Operation input wired");
            let src_ref = g.ports.get(src_sid).expect("src port");
            let src_node = g.nodes.get(&src_ref.node_sid).expect("src node");
            assert_eq!(src_node.id, "Float", "Operation input must be Float constant");
            let xin: f32 = src_node
                .modifier
                .trim()
                .replace(',', ".")
                .parse()
                .unwrap_or_else(|_| panic!("bad Float {}", src_node.modifier));

            let out = eval_operation(xin, kind);
            let expect = python_expect(kind, xin);
            let err = out - expect;
            let label = crate::graph::dropdowns::OPERATION
                .get(kind as usize)
                .copied()
                .unwrap_or("?");
            rows.push((label.to_string(), kind, xin, out, expect, err));
            assert!(
                err.abs() <= 1e-3 || err.abs() <= 1e-3 * expect.abs().max(1.0),
                "FAIL {label} kind={kind}: In={xin} Out={out} Expect={expect} Err={err}"
            );
        }

        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.partial_cmp(&b.2).unwrap()));
        eprintln!(
            "OperationProbe sim ({})",
            path.display()
        );
        eprintln!(
            "{:<8} {:>4} {:>12} {:>12} {:>12} {:>12}",
            "op", "kind", "In", "Out", "Expect", "Err"
        );
        for (label, kind, xin, out, expect, err) in &rows {
            eprintln!(
                "{:<8} {:>4} {:>12.6} {:>12.6} {:>12.6} {:>12.6e}",
                label, kind, xin, out, expect, err
            );
        }
        assert_eq!(rows.len(), 17, "probe has 17 Operation nodes");

        // Explicit sqrt regressions (Unity TimePlot confirmed these).
        let sqrt2 = rows
            .iter()
            .find(|r| r.1 == OP_SQRT && (r.2 - 2.0).abs() < 1e-6)
            .expect("sqrt(2) case");
        assert!(
            (sqrt2.3 - 2.0_f32.sqrt()).abs() < 1e-5,
            "sqrt(2) must not be identity; got {}",
            sqrt2.3
        );
        let sqrt025 = rows
            .iter()
            .find(|r| r.1 == OP_SQRT && (r.2 - 0.25).abs() < 1e-6)
            .expect("sqrt(0.25) case");
        assert!((sqrt025.3 - 0.5).abs() < 1e-5);
    }

    #[test]
    fn operation_probe_compiles_without_panic() {
        let path = find_probe();
        let g = load_team_graph(&path).expect("load");
        let compiled = Lowerer::compile(g);
        // TimePlot-only sinks may DCE math; compile must still accept all Operation kinds.
        let _ = ProgramBuilder.pack(&compiled);
    }
}
