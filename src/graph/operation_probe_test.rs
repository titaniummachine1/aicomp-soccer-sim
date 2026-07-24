//! Run Unity `OperationProbe.txt` through sim Operation eval (same cases as game TimePlot).

#[cfg(test)]
mod operation_probe_sim {
    use std::path::PathBuf;

    use crate::graph::dropdowns::OperationKind;
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

    /// Golden values from Unity TimePlot `timeplot_2026-07-24_12-29-26.json`
    /// (cross-checked vs Python). Keyed by (kind, input).
    fn unity_golden(kind: OperationKind, xin: f32) -> Option<f32> {
        let pairs: &[(OperationKind, f32, f32)] = &[
            (OperationKind::Abs, -3.5, 3.5),
            (OperationKind::Round, 2.6, 3.0),
            (OperationKind::Floor, 2.6, 2.0),
            (OperationKind::Ceil, 2.3, 3.0),
            (OperationKind::Sin, 0.0, 0.0),
            (OperationKind::Cos, 0.0, 1.0),
            (OperationKind::Tan, 0.0, 0.0),
            (OperationKind::Asin, 0.5, 0.5235988),
            (OperationKind::Acos, 0.5, 1.0471976),
            (OperationKind::Atan, 1.0, 0.7853982),
            (OperationKind::Sqrt, 2.0, 1.4142135),
            (OperationKind::Sqrt, 0.25, 0.5),
            (OperationKind::Sign, -5.0, -1.0),
            (OperationKind::Ln, std::f32::consts::E, 1.0),
            (OperationKind::Log10, 100.0, 2.0),
            (OperationKind::Exp, 1.0, std::f32::consts::E),
            (OperationKind::Pow10, 2.0, 100.0),
        ];
        pairs
            .iter()
            .find(|(k, x, _)| *k == kind && (*x - xin).abs() < 1e-5)
            .map(|(_, _, y)| *y)
    }

    #[test]
    fn operation_probe_txt_matches_unity_timeplot_goldens() {
        let path = find_probe();
        let g = load_team_graph(&path).expect("load OperationProbe");

        let mut rows = Vec::new();
        for node in g.nodes.values().filter(|n| n.id == "Operation") {
            let kind = OperationKind::from_modifier(&node.modifier);
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
            assert_eq!(src_node.id, "Float");
            let xin: f32 = src_node
                .modifier
                .trim()
                .replace(',', ".")
                .parse()
                .unwrap_or_else(|_| panic!("bad Float {}", src_node.modifier));

            let out = kind.eval(xin);
            let expect = unity_golden(kind, xin).unwrap_or_else(|| {
                panic!("no Unity golden for {:?} In={}", kind, xin)
            });
            let err = out - expect;
            rows.push((kind, xin, out, expect, err));
            assert!(
                err.abs() <= 1e-4 || err.abs() <= 1e-4 * expect.abs().max(1.0),
                "FAIL {kind:?}: In={xin} Out={out} UnityExpect={expect} Err={err}"
            );
        }

        rows.sort_by(|a, b| {
            a.0.label()
                .cmp(b.0.label())
                .then(a.1.partial_cmp(&b.1).unwrap())
        });
        eprintln!("OperationProbe sim ({})", path.display());
        eprintln!(
            "{:<8} {:>4} {:>12} {:>12} {:>12} {:>12}",
            "op", "kind", "In", "Out", "Unity", "Err"
        );
        for (kind, xin, out, expect, err) in &rows {
            eprintln!(
                "{:<8} {:>4} {:>12.6} {:>12.6} {:>12.6} {:>12.6e}",
                kind.label(),
                kind.as_u32(),
                xin,
                out,
                expect,
                err
            );
        }
        assert_eq!(rows.len(), 17);

        let sqrt2 = rows
            .iter()
            .find(|r| r.0 == OperationKind::Sqrt && (r.1 - 2.0).abs() < 1e-6)
            .expect("sqrt(2)");
        assert!((sqrt2.2 - 2.0_f32.sqrt()).abs() < 1e-5);
        assert!((sqrt2.2 - 2.0).abs() > 0.1, "must not be identity");
    }

    #[test]
    fn operation_probe_compiles_without_panic() {
        let path = find_probe();
        let g = load_team_graph(&path).expect("load");
        let compiled = Lowerer::compile(g);
        let _ = ProgramBuilder.pack(&compiled);
    }
}
