use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    FeatureParameterTarget, ParameterPath, ParameterValueType, SpatialPathSegment,
    WeldmentMemberSpec,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V12, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepPlanarSegment,
};
use ketchup_core::persistence::{self, ContainerData};
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

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
}

#[test]
fn first_class_weldment_member_is_exact_parametric_and_lossless() {
    let definition = DefinitionId(1);
    let profile = FeatureId(1);
    let path = FeatureId(2);
    let member = FeatureId(3);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "General weldment".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Parametric rectangular section".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Spatial member path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 100.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: member,
                definition_id: definition,
                name: "Weldment member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile,
                    path,
                    orientation_degrees: 0.0,
                }),
            },
        ]))
        .unwrap();

    let initial = document.current();
    let initial_graph = ExactBRepGraph::from_snapshot(&initial, definition, member).unwrap();
    assert_eq!(initial_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V12);
    assert_eq!(initial_graph.profiles.len(), 2);
    let ExactBRepOperation::SpatialSweep {
        profile: oriented_profile,
        ..
    } = &initial_graph.nodes[0].operation
    else {
        panic!("weldment member must compile to a native spatial sweep");
    };
    assert_eq!(oriented_profile.0, 1);
    assert_eq!(initial_graph.profiles[1].source_feature_id, member.0);

    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let initial_package = worker.evaluate_exact_brep_graph(&initial_graph).unwrap();
    assert_close(initial_package.volume_mm3, 800.0);
    assert_eq!(initial_package.topology_counts[4], 1);
    assert_close(initial_package.bounds_mm[0][0], -2.0);
    assert_close(initial_package.bounds_mm[1][0], 2.0);
    assert_close(initial_package.bounds_mm[0][1], -1.0);
    assert_close(initial_package.bounds_mm[1][1], 1.0);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: target(profile, "bounds.width", ParameterValueType::Length),
                dimension: dimension(6.0),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: target(member, "orientation", ParameterValueType::Angle),
                dimension: dimension(90.0),
            },
        ]))
        .unwrap();
    let edited = document.current();
    let edited_graph = ExactBRepGraph::from_snapshot(&edited, definition, member).unwrap();
    assert_ne!(edited_graph.graph_digest, initial_graph.graph_digest);
    let ExactBRepPlanarGeometry::Boundary { segments, .. } = &edited_graph.profiles[1].geometry
    else {
        panic!("rectangular weldment profile must remain a boundary");
    };
    let ExactBRepPlanarSegment::Line { start_bits, .. } = &segments[0] else {
        panic!("rectangle starts with a line");
    };
    assert_close(f64::from_bits(start_bits[0]), 1.0);
    assert_close(f64::from_bits(start_bits[1]), -2.0);
    let edited_package = worker.evaluate_exact_brep_graph(&edited_graph).unwrap();
    assert_close(edited_package.volume_mm3, 1_200.0);
    assert_close(edited_package.bounds_mm[0][0], -1.0);
    assert_close(edited_package.bounds_mm[1][0], 1.0);
    assert_close(edited_package.bounds_mm[0][1], -2.0);
    assert_close(edited_package.bounds_mm[1][1], 4.0);

    assert_eq!(
        document
            .undo()
            .unwrap()
            .feature(member)
            .unwrap()
            .kind()
            .parameter_value(&ParameterPath::new("orientation").unwrap()),
        Some(0.0)
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        edited.canonical_digest()
    );

    let bytes = persistence::save_document_store(&document, &ContainerData::default()).unwrap();
    let reopened = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(
        reopened.current().canonical_digest(),
        edited.canonical_digest()
    );
    assert_eq!(
        persistence::save_document_store(&reopened, &ContainerData::default()).unwrap(),
        bytes
    );
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened.current(), definition, member).unwrap();
    assert_eq!(reopened_graph.graph_digest, edited_graph.graph_digest);

    let before_invalid = document.current();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureParameter {
                    target: target(member, "orientation", ParameterValueType::Angle),
                    dimension: dimension(180.0),
                },
            ]))
            .is_err()
    );
    assert_eq!(
        document.current().revision_id(),
        before_invalid.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid.canonical_digest()
    );
}
