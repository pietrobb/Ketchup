use super::*;
use ketchup_model::import::{StepImportEvidence, plan_iges_xde_import, plan_step_xde_import};

/// Builds part `part_index` of an imported multi-part source into a graph on
/// its own and with a through cut, and evaluates both from the source bytes.
fn assert_part_evaluates_in_graph(
    supervisor: &mut ExactWorkerSupervisor,
    batch: &CommandBatch,
    source: &[u8],
    part_index: u32,
    part: &StepImportEvidence,
) {
    let mut document = DocumentStore::new();
    document.apply_batch(batch).unwrap();
    let imported_snapshot = document.current();
    let imported = imported_snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) if spec.source_part_index == Some(part_index) => {
                Some(feature.id())
            }
            _ => None,
        })
        .unwrap();
    let definition = imported_snapshot
        .definitions()
        .find(|definition| definition.feature_ids().contains(&imported))
        .unwrap()
        .id();
    let [min, max] = part.bounds_mm;
    let x0 = min[0] + (max[0] - min[0]) * 0.25;
    let x1 = min[0] + (max[0] - min[0]) * 0.75;
    let y0 = min[1] + (max[1] - min[1]) * 0.25;
    let y1 = min[1] + (max[1] - min[1]) * 0.75;
    let profile = FeatureId(imported.0 + 100);
    let cut = FeatureId(imported.0 + 101);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Cut through the imported part".into(),
                kind: FeatureKind::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
            },
            CanonicalCommand::CreateFeature {
                id: cut,
                definition_id: definition,
                name: "Imported part through all".into(),
                kind: FeatureKind::through_cut(imported, profile),
            },
        ]))
        .unwrap();
    let snapshot = document.current();

    let part_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, imported).unwrap();
    let part_package = supervisor
        .evaluate_exact_brep_graph_with_imported_source(&part_graph, source)
        .unwrap();
    assert_eq!(part_package.topology_counts, part.topology_counts);
    assert!((part_package.volume_mm3 - part.volume_mm3).abs() <= part.volume_mm3 * 1.0e-9);
    assert_bounds_close(
        part_package.bounds_mm,
        [min[0], min[1], min[2], max[0], max[1], max[2]],
    );

    let cut_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, cut).unwrap();
    let cut_package = supervisor
        .evaluate_exact_brep_graph_with_imported_source(&cut_graph, source)
        .unwrap();
    assert!(cut_package.volume_mm3 > 0.0);
    assert!(cut_package.volume_mm3 < part.volume_mm3);
}

#[test]
fn worker_evaluates_a_step_and_an_iges_xde_part_inside_a_graph() {
    let _turn = crate::integration_support::file_turn();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let cancelled = AtomicBool::new(false);
    let step_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let step = std::fs::read(&step_path).unwrap();
    let step_evidence = supervisor
        .inspect_step_xde_import_with_cancellation(&step_path, &sha256_hex(&step), &cancelled)
        .unwrap();
    // The second part, not the whole file nor the first part read as one body.
    let step_part = step_evidence.parts.last().unwrap();
    assert!(step_part.index > 0);
    let step_batch = plan_step_xde_import(
        &DocumentStore::new().current(),
        &step,
        "assembly.step",
        &step_evidence,
    )
    .unwrap();
    assert_part_evaluates_in_graph(
        &mut supervisor,
        &step_batch,
        &step,
        step_part.index,
        &step_part.exact,
    );

    let directory = tempfile::tempdir().unwrap();
    let iges_path = directory.path().join("assembly.iges");
    supervisor
        .convert_step_to_iges_with_cancellation(&step_path, &iges_path, 1, &cancelled)
        .unwrap();
    let iges = std::fs::read(&iges_path).unwrap();
    let iges_evidence = supervisor
        .inspect_iges_xde_import_with_cancellation(&iges_path, &sha256_hex(&iges), &cancelled)
        .unwrap();
    let iges_part = iges_evidence.parts.last().unwrap();
    assert!(iges_part.index > 0);
    let iges_batch = plan_iges_xde_import(
        &DocumentStore::new().current(),
        &iges,
        "assembly.iges",
        &iges_evidence,
    )
    .unwrap();
    assert_part_evaluates_in_graph(
        &mut supervisor,
        &iges_batch,
        &iges,
        iges_part.index,
        &iges_part.exact,
    );
}
