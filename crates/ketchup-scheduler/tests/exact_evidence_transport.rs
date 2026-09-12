use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_scheduler::ExactWorkerSupervisor;

#[test]
fn fresh_worker_preserves_complete_64_gon_topology_and_fingerprint() {
    let definition = DefinitionId(1);
    let profile = FeatureId(1);
    let extrusion = FeatureId(2);
    let sides = 64;
    let radius = 10.0;
    let height = 5.0;
    let points_mm = (0..sides)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / f64::from(sides);
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Transport regression".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Polygon".into(),
                kind: FeatureKind::Profile { points_mm },
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: definition,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::new("5", height).unwrap(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let digest = snapshot.canonical_digest();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, extrusion).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts, [128, 192, 66, 1, 1]);
    assert_eq!(package.edge_evidence.len(), 192);
    let ordinals: std::collections::BTreeSet<_> = package
        .edge_evidence
        .iter()
        .map(|edge| edge.edge_ordinal)
        .collect();
    assert_eq!(ordinals.len(), 192);
    assert!(
        package
            .edge_evidence
            .iter()
            .all(|edge| edge.adjacent_face_ordinals.len() == 2)
    );
    let expected_volume = f64::from(sides)
        * radius
        * radius
        * (std::f64::consts::TAU / f64::from(sides)).sin()
        * height
        / 2.0;
    assert!((package.volume_mm3 - expected_volume).abs() < 1.0e-7);
    assert!(!package.vertices.is_empty());
    assert_eq!(document.current().canonical_digest(), digest);
    // A fresh process must reproduce the same complete, request-bound package.
    drop(worker);
    let mut fresh =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    assert_eq!(fresh.evaluate_exact_brep_graph(&graph).unwrap(), package);
}
