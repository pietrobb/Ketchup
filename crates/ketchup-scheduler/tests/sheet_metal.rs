use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    FeatureParameterTarget, ParameterPath, ParameterValueType,
};
use ketchup_model::exact_brep_graph::{EXACT_BREP_GRAPH_SCHEMA_V19, ExactBRepGraph};
use ketchup_model::sheet_metal::{SheetMetalBend, SheetMetalSpec};
use ketchup_scheduler::ExactWorkerSupervisor;

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn target(
    feature_id: FeatureId,
    path: &str,
    value_type: ParameterValueType,
) -> FeatureParameterTarget {
    FeatureParameterTarget {
        feature_id,
        path: ParameterPath::new(path).unwrap(),
        value_type,
    }
}

fn bend(parent: Option<usize>, edge: usize, length: f64, angle_degrees: f64) -> SheetMetalBend {
    SheetMetalBend {
        parent,
        edge,
        length: dimension(length),
        angle_degrees,
        inner_radius: dimension(3.0),
    }
}

fn sheet(base_mm: Vec<[f64; 2]>, bends: Vec<SheetMetalBend>) -> SheetMetalSpec {
    SheetMetalSpec {
        base_mm,
        thickness: dimension(2.0),
        k_factor: 0.4,
        bends,
    }
}

fn rectangle() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]]
}

/// The document holding `spec` as feature 1, and whether it was accepted.
fn document(spec: SheetMetalSpec) -> (DocumentStore, bool) {
    let mut document = DocumentStore::new();
    let applied = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Sheet metal part".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(1),
            name: "Sheet".into(),
            kind: FeatureKind::SheetMetal(spec),
        },
    ]));
    (document, applied.is_ok())
}

/// The volume and solid count of the exact body of feature 1.
fn evaluate(worker: &mut ExactWorkerSupervisor, document: &DocumentStore) -> (f64, u64) {
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(1), FeatureId(1)).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V19);
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    (package.volume_mm3, package.topology_counts[4] as u64)
}

/// Volume of a 2 mm sheet over `base_area`, plus one 3 mm inner-radius bend of `degrees`
/// and a flange `length` long per `(span, degrees, length)`.
fn sheet_volume(base_area: f64, bends: &[(f64, f64, f64)]) -> f64 {
    base_area * 2.0
        + bends
            .iter()
            .map(|(span, degrees, length)| {
                degrees.to_radians() * 0.5 * (5.0_f64.powi(2) - 3.0_f64.powi(2)) * span
                    + length * span * 2.0
            })
            .sum::<f64>()
}

#[test]
fn canonical_sheet_metal_bend_is_exact_and_invalid_edits_are_atomic() {
    let _turn = crate::integration_support::file_turn();
    let feature_id = FeatureId(1);
    let spec = sheet(rectangle(), vec![bend(None, 1, 30.0, 90.0)]);
    assert!((spec.bend_allowance_mm(0).unwrap() - 5.969_026_041_820_607).abs() < 1.0e-12);

    let (mut document, accepted) = document(spec);
    assert!(accepted);
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let (volume, solids) = evaluate(&mut worker, &document);
    let expected_volume = sheet_volume(5000.0, &[(50.0, 90.0, 30.0)]);
    assert!(
        (volume - expected_volume).abs() < 1.0e-5,
        "{volume} != {expected_volume}"
    );
    assert_eq!(solids, 1);

    for (path, value, value_type) in [
        ("thickness", 100.0, ParameterValueType::Length),
        ("bends.0.inner_radius", 0.0, ParameterValueType::Length),
        ("bends.0.angle", 180.0, ParameterValueType::Angle),
    ] {
        let before = document.current();
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureParameter {
                        target: target(feature_id, path, value_type),
                        dimension: dimension(value),
                    }
                ]))
                .is_err()
        );
        assert_eq!(document.current().revision_id(), before.revision_id());
        assert_eq!(
            document.current().canonical_digest(),
            before.canonical_digest()
        );
    }

    let (colliding, accepted) = self::document(sheet(
        rectangle(),
        vec![bend(None, 0, 20.0, 90.0), bend(None, 3, 20.0, 90.0)],
    ));
    assert!(!accepted);
    assert_eq!(
        colliding.current().revision_id(),
        DocumentStore::new().current().revision_id()
    );
    assert!(colliding.current().definition(DefinitionId(1)).is_none());
}

#[test]
fn every_base_edge_and_bend_direction_produces_one_exact_solid() {
    let _turn = crate::integration_support::file_turn();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for edge in 0..4 {
        for angle_degrees in [-90.0, 90.0] {
            let span = if edge % 2 == 0 { 100.0 } else { 50.0 };
            let (document, accepted) = document(sheet(
                rectangle(),
                vec![bend(None, edge, 30.0, angle_degrees)],
            ));
            assert!(accepted);
            let (volume, solids) = evaluate(&mut worker, &document);
            let expected_volume = sheet_volume(5000.0, &[(span, 90.0, 30.0)]);
            assert!(
                (volume - expected_volume).abs() < 1.0e-5,
                "edge {edge} {angle_degrees}: {volume} != {expected_volume}"
            );
            assert_eq!(solids, 1, "edge {edge} {angle_degrees}");
        }
    }
}

#[test]
fn a_slanted_edge_bend_carrying_a_lip_is_one_exact_solid() {
    let _turn = crate::integration_support::file_turn();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    // Edge 1 runs at a slant from (100, 0) to (70, 60); its flange folds a 10 mm lip back
    // over the base from its far edge, and a 60 degree flange leaves the opposite edge.
    let base = vec![[0.0, 0.0], [100.0, 0.0], [70.0, 60.0], [0.0, 60.0]];
    let slant = 30.0_f64.hypot(60.0);
    let (document, accepted) = document(sheet(
        base,
        vec![
            bend(None, 1, 20.0, 90.0),
            bend(None, 3, 15.0, -60.0),
            bend(Some(0), 2, 10.0, 90.0),
        ],
    ));
    assert!(accepted);
    let (volume, solids) = evaluate(&mut worker, &document);
    let expected_volume = sheet_volume(
        (100.0 + 70.0) * 0.5 * 60.0,
        &[(slant, 90.0, 20.0), (60.0, 60.0, 15.0), (slant, 90.0, 10.0)],
    );
    assert!(
        (volume - expected_volume).abs() < 1.0e-4,
        "{volume} != {expected_volume}"
    );
    assert_eq!(solids, 1);
}
