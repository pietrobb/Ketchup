use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    FeatureParameterTarget, ParameterPath, ParameterValueType,
};
use ketchup_core::exact_brep_graph::{EXACT_BREP_GRAPH_SCHEMA_V19, ExactBRepGraph};
use ketchup_core::sheet_metal::{SheetMetalEdge, SheetMetalFlange, SheetMetalSpec};
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

#[test]
fn canonical_sheet_metal_bend_is_exact_and_invalid_edits_are_atomic() {
    let definition_id = DefinitionId(1);
    let feature_id = FeatureId(1);
    let spec = SheetMetalSpec {
        width: dimension(100.0),
        depth: dimension(50.0),
        thickness: dimension(2.0),
        k_factor: 0.4,
        flanges: vec![SheetMetalFlange {
            edge: SheetMetalEdge::MaxX,
            length: dimension(30.0),
            angle_degrees: 90.0,
            inner_radius: dimension(3.0),
        }],
    };
    assert!(
        (spec.bend_allowance_mm(&spec.flanges[0]).unwrap() - 5.969_026_041_820_607).abs() < 1.0e-12
    );

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "Sheet metal part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: "Base with flange".into(),
                kind: FeatureKind::SheetMetal(spec),
            },
        ]))
        .unwrap();

    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), definition_id, feature_id).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V19);
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let expected_volume = 100.0 * 50.0 * 2.0
        + std::f64::consts::FRAC_PI_4 * (5.0_f64.powi(2) - 3.0_f64.powi(2)) * 50.0
        + 30.0 * 50.0 * 2.0;
    assert!(
        (package.volume_mm3 - expected_volume).abs() < 1.0e-5,
        "{} != {expected_volume}",
        package.volume_mm3
    );
    assert_eq!(package.topology_counts[4], 1);

    for (path, value, value_type) in [
        ("thickness", 100.0, ParameterValueType::Length),
        ("flanges.0.inner_radius", 0.0, ParameterValueType::Length),
        ("flanges.0.angle", 180.0, ParameterValueType::Angle),
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

    let mut colliding = DocumentStore::new();
    let before = colliding.current();
    assert!(
        colliding
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Unrelieved corner".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(2),
                    name: "Colliding adjacent flanges".into(),
                    kind: FeatureKind::SheetMetal(SheetMetalSpec {
                        width: dimension(100.0),
                        depth: dimension(50.0),
                        thickness: dimension(2.0),
                        k_factor: 0.4,
                        flanges: vec![
                            SheetMetalFlange {
                                edge: SheetMetalEdge::MinX,
                                length: dimension(20.0),
                                angle_degrees: 90.0,
                                inner_radius: dimension(3.0),
                            },
                            SheetMetalFlange {
                                edge: SheetMetalEdge::MinY,
                                length: dimension(20.0),
                                angle_degrees: 90.0,
                                inner_radius: dimension(3.0),
                            },
                        ],
                    }),
                },
            ]))
            .is_err()
    );
    assert_eq!(colliding.current().revision_id(), before.revision_id());
    assert_eq!(
        colliding.current().canonical_digest(),
        before.canonical_digest()
    );
    assert!(colliding.current().definition(DefinitionId(2)).is_none());
}

#[test]
fn every_boundary_edge_and_bend_direction_produces_one_exact_solid() {
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for edge in [
        SheetMetalEdge::MinX,
        SheetMetalEdge::MaxX,
        SheetMetalEdge::MinY,
        SheetMetalEdge::MaxY,
    ] {
        for angle_degrees in [-90.0, 90.0] {
            let span = if matches!(edge, SheetMetalEdge::MinX | SheetMetalEdge::MaxX) {
                50.0
            } else {
                100.0
            };
            let spec = SheetMetalSpec {
                width: dimension(100.0),
                depth: dimension(50.0),
                thickness: dimension(2.0),
                k_factor: 0.4,
                flanges: vec![SheetMetalFlange {
                    edge,
                    length: dimension(30.0),
                    angle_degrees,
                    inner_radius: dimension(3.0),
                }],
            };
            let mut document = DocumentStore::new();
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateDefinition {
                        id: DefinitionId(1),
                        name: "Sheet metal edge variant".into(),
                    },
                    CanonicalCommand::CreateFeature {
                        id: FeatureId(1),
                        definition_id: DefinitionId(1),
                        name: "Boundary flange".into(),
                        kind: FeatureKind::SheetMetal(spec),
                    },
                ]))
                .unwrap();
            let graph =
                ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(1), FeatureId(1))
                    .unwrap();
            let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
            let expected_volume = 100.0 * 50.0 * 2.0
                + std::f64::consts::FRAC_PI_4 * (5.0_f64.powi(2) - 3.0_f64.powi(2)) * span
                + 30.0 * span * 2.0;
            assert!(
                (package.volume_mm3 - expected_volume).abs() < 1.0e-5,
                "{edge:?} {angle_degrees}: {} != {expected_volume}",
                package.volume_mm3
            );
            assert_eq!(package.topology_counts[4], 1, "{edge:?} {angle_degrees}");
        }
    }
}
