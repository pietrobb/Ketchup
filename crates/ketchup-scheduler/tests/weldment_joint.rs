use ketchup_core::document::{
    CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, FeatureParameterTarget,
    OccurrenceId, ParameterPath, ParameterValueType, Snapshot, SpatialPathSegment, Transform,
    WeldmentJointPolicy, WeldmentJointPrimary, WeldmentJointSpec, WeldmentMemberSpec,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V19, ExactBRepGraph, ExactBRepOperation,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::exact_validation::{
    BuiltinGeneralBodyValidator, general_body_input_bytes, general_body_validation_policy,
};
use ketchup_core::fabrication::{
    FABRICATION_ROLE_DIMENSION_V1, GeneralFabricationError, GeneralFabricationProjection,
    MANUFACTURED_ITEM_ROLE_V1, MATERIAL_DIMENSION_V1, WELDMENT_CUT_LIST_EXPORT_V1,
    WELDMENT_DRAWING_SVG_V1, WeldmentCutTreatment, project_general_fabrication,
};
use ketchup_core::persistence::{self, ContainerData};
use ketchup_core::prismatic::TolerancePolicy;
use ketchup_core::validation::{
    HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationState,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::sync::Arc;

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn document_with_members(
    first_segments: Vec<SpatialPathSegment>,
    second_segments: Vec<SpatialPathSegment>,
) -> DocumentStore {
    document_with_profile_and_members(
        vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
        first_segments,
        second_segments,
    )
}

fn document_with_profile_and_members(
    profile_points: Vec<[f64; 2]>,
    first_segments: Vec<SpatialPathSegment>,
    second_segments: Vec<SpatialPathSegment>,
) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "General weldment joint".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Shared square section".into(),
                kind: FeatureKind::Profile {
                    points_mm: profile_points,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "First path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: first_segments,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "First member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile: FeatureId(1),
                    path: FeatureId(2),
                    orientation_degrees: 0.0,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Second path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: second_segments,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(1),
                name: "Second member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile: FeatureId(1),
                    path: FeatureId(4),
                    orientation_degrees: 0.0,
                }),
            },
        ]))
        .unwrap();
    document
}

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialPathSegment {
    SpatialPathSegment::Line { start_mm, end_mm }
}

fn add_joint(
    document: &mut DocumentStore,
    policy: WeldmentJointPolicy,
    primary: WeldmentJointPrimary,
) -> Result<(), ketchup_core::document::CanonicalError> {
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(6),
            definition_id: DefinitionId(1),
            name: "Associative corner joint".into(),
            kind: FeatureKind::WeldmentJoint(WeldmentJointSpec {
                first_member: FeatureId(3),
                second_member: FeatureId(5),
                policy,
                primary,
            }),
        }]))
        .map(|_| ())
}

fn perpendicular_document() -> DocumentStore {
    document_with_members(
        vec![line([-100.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
        vec![line([0.0, 0.0, 0.0], [0.0, 100.0, 0.0])],
    )
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
}

fn evaluated_fabrication(
    snapshot: &Snapshot,
    worker: &mut ExactWorkerSupervisor,
    producer_feature_id: FeatureId,
) -> Result<GeneralFabricationProjection, GeneralFabricationError> {
    let graph =
        ExactBRepGraph::from_snapshot(snapshot, DefinitionId(1), producer_feature_id).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let registry = ExactResultRegistry::accept(snapshot, [Arc::new(package.into())]).unwrap();
    let tolerance = TolerancePolicy::default();
    let cases = Vec::new();
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let input = general_body_input_bytes(&cases);
    let invocation =
        ValidationInvocation::bind(snapshot, validator.descriptor(), &policy, vec![], &input);
    let report = validator.invoke(ValidationExecution {
        snapshot,
        invocation,
        policy: &policy,
        input: &cases,
    });
    assert_eq!(report.state, ValidationState::Passed);
    project_general_fabrication(snapshot, &registry, &cases, &report, tolerance)
}

#[test]
fn butt_and_miter_joints_are_exact_associative_and_lossless() {
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    for (policy, primary) in [
        (WeldmentJointPolicy::Butt, WeldmentJointPrimary::First),
        (WeldmentJointPolicy::Butt, WeldmentJointPrimary::Second),
        (WeldmentJointPolicy::Miter, WeldmentJointPrimary::First),
    ] {
        let mut document = perpendicular_document();
        add_joint(&mut document, policy, primary).unwrap();
        let snapshot = document.current();
        assert!(snapshot.feature_dependency_graph().is_ok());
        let graph =
            ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(1), FeatureId(6)).unwrap();
        assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V19);
        assert!(matches!(
            graph.nodes.last().unwrap().operation,
            ExactBRepOperation::WeldmentJoint { .. }
        ));
        let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(package.topology_counts[4], 2);
        assert_close(package.volume_mm3, 3_184.0);
    }

    let mut document = perpendicular_document();
    add_joint(
        &mut document,
        WeldmentJointPolicy::Miter,
        WeldmentJointPrimary::First,
    )
    .unwrap();
    let original = document.current();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: FeatureParameterTarget {
                    feature_id: FeatureId(1),
                    path: ParameterPath::new("bounds.width").unwrap(),
                    value_type: ParameterValueType::Length,
                },
                dimension: dimension(6.0),
            },
        ]))
        .unwrap();
    let edited = document.current();
    let edited_graph =
        ExactBRepGraph::from_snapshot(&edited, DefinitionId(1), FeatureId(6)).unwrap();
    let edited_package = worker.evaluate_exact_brep_graph(&edited_graph).unwrap();
    assert_eq!(edited_package.topology_counts[4], 2);
    assert_close(edited_package.volume_mm3, 4_784.0);
    assert_ne!(edited.canonical_digest(), original.canonical_digest());
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        original.canonical_digest()
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
        document.current().canonical_digest()
    );
    assert_eq!(
        persistence::save_document_store(&reopened, &ContainerData::default()).unwrap(),
        bytes
    );
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened.current(), DefinitionId(1), FeatureId(6)).unwrap();
    assert_eq!(reopened_graph.graph_digest, edited_graph.graph_digest);
}

#[test]
fn joint_without_an_exact_body_intersection_is_rejected_fail_closed() {
    let mut document = document_with_profile_and_members(
        vec![[10.0, 10.0], [14.0, 10.0], [14.0, 14.0], [10.0, 14.0]],
        vec![line([-100.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
        vec![line([0.0, 0.0, 0.0], [0.0, 100.0, 0.0])],
    );
    add_joint(
        &mut document,
        WeldmentJointPolicy::Miter,
        WeldmentJointPrimary::First,
    )
    .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(1), FeatureId(6)).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    assert!(worker.evaluate_exact_brep_graph(&graph).is_err());
    assert_eq!(document.current().revision_id(), snapshot.revision_id());
    assert_eq!(
        document.current().canonical_digest(),
        snapshot.canonical_digest()
    );
}

#[test]
fn invalid_joint_geometry_is_rejected_without_a_revision() {
    let cases = [
        document_with_members(
            vec![line([-100.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
            vec![line([0.0, 10.0, 0.0], [0.0, 100.0, 0.0])],
        ),
        document_with_members(
            vec![line([-100.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
            vec![line([0.0, 0.0, 0.0], [100.0, 0.0, 0.0])],
        ),
        document_with_members(
            vec![line([-100.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
            vec![
                line([0.0, 0.0, 0.0], [0.0, 50.0, 0.0]),
                line([0.0, 50.0, 0.0], [0.0, 100.0, 0.0]),
            ],
        ),
    ];

    for mut document in cases {
        let before = document.current();
        assert!(
            add_joint(
                &mut document,
                WeldmentJointPolicy::Miter,
                WeldmentJointPrimary::First,
            )
            .is_err()
        );
        assert_eq!(document.current().revision_id(), before.revision_id());
        assert_eq!(
            document.current().canonical_digest(),
            before.canonical_digest()
        );
    }
}

#[test]
fn weldment_cut_list_is_exact_associative_stable_and_fail_closed() {
    const OCCURRENCE: OccurrenceId = OccurrenceId(10);
    const ROLE_DIMENSION: ClassificationDimensionId = ClassificationDimensionId(20);
    const ROLE_CATEGORY: ClassificationCategoryId = ClassificationCategoryId(21);
    const MATERIAL_DIMENSION: ClassificationDimensionId = ClassificationDimensionId(22);
    const MATERIAL_CATEGORY: ClassificationCategoryId = ClassificationCategoryId(23);

    let mut document = perpendicular_document();
    add_joint(
        &mut document,
        WeldmentJointPolicy::Miter,
        WeldmentJointPrimary::First,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DefinitionId(1),
                name: "Welded frame".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ROLE_DIMENSION,
                name: FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(ROLE_CATEGORY, MANUFACTURED_ITEM_ROLE_V1.into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OCCURRENCE,
                dimension_id: ROLE_DIMENSION,
                category_id: Some(ROLE_CATEGORY),
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: MATERIAL_DIMENSION,
                name: MATERIAL_DIMENSION_V1.into(),
                categories: vec![(MATERIAL_CATEGORY, "material.steel.s355.v1".into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OCCURRENCE,
                dimension_id: MATERIAL_DIMENSION,
                category_id: Some(MATERIAL_CATEGORY),
            },
        ]))
        .unwrap();
    let original = document.current();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let projection = evaluated_fabrication(&original, &mut worker, FeatureId(6)).unwrap();
    let weldment = projection.weldment.as_ref().unwrap();
    assert_eq!(weldment.rows.len(), 2);
    assert_eq!(weldment.rows[0].position, 1);
    assert_eq!(weldment.rows[1].position, 2);
    assert_eq!(weldment.rows[0].member_feature_id, FeatureId(3));
    assert_eq!(weldment.rows[1].member_feature_id, FeatureId(5));
    assert_eq!(weldment.rows[0].profile_feature_id, FeatureId(1));
    assert_eq!(weldment.rows[1].profile_feature_id, FeatureId(1));
    assert_eq!(weldment.rows[0].quantity, 1);
    assert_eq!(weldment.rows[1].quantity, 1);
    assert_eq!(weldment.rows[0].material_key, "material.steel.s355.v1");
    assert_close(weldment.rows[0].centerline_length_mm, 100.0);
    assert_close(weldment.rows[1].centerline_length_mm, 100.0);
    assert_eq!(
        weldment.rows[0].end_cut.treatment,
        WeldmentCutTreatment::Miter
    );
    assert_eq!(
        weldment.rows[1].start_cut.treatment,
        WeldmentCutTreatment::Miter
    );
    assert_close(weldment.rows[0].end_cut.angle_degrees, 45.0);
    assert_close(weldment.rows[1].start_cut.angle_degrees, 45.0);
    let cut_list = String::from_utf8(weldment.cut_list_export(&original).unwrap()).unwrap();
    assert!(cut_list.starts_with(WELDMENT_CUT_LIST_EXPORT_V1));
    assert!(cut_list.contains(&format!(
        "drawing_result_digest={}",
        weldment.drawing_envelope.result_digest
    )));
    let drawing = String::from_utf8(weldment.drawing_svg(&original).unwrap()).unwrap();
    assert!(drawing.contains(WELDMENT_DRAWING_SVG_V1));
    assert!(drawing.contains(&format!(
        "cut_list={}",
        weldment.cut_list_envelope.result_digest
    )));
    let saved = persistence::save_document_store(&document, &ContainerData::default()).unwrap();
    let reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_snapshot = reopened.current();
    assert_eq!(
        weldment.cut_list_export(&reopened_snapshot).unwrap(),
        cut_list.as_bytes()
    );
    assert_eq!(
        weldment.drawing_svg(&reopened_snapshot).unwrap(),
        drawing.as_bytes()
    );

    let mut tampered = weldment.clone();
    tampered.rows[0].centerline_length_mm = 99.0;
    assert_eq!(
        tampered.cut_list_export(&original),
        Err(GeneralFabricationError::ExportBlocked)
    );
    assert_eq!(
        tampered.drawing_svg(&original),
        Err(GeneralFabricationError::ExportBlocked)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: FeatureId(6) },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(1),
                name: "Associative corner joint".into(),
                kind: FeatureKind::WeldmentJoint(WeldmentJointSpec {
                    first_member: FeatureId(3),
                    second_member: FeatureId(5),
                    policy: WeldmentJointPolicy::Butt,
                    primary: WeldmentJointPrimary::First,
                }),
            },
        ]))
        .unwrap();
    let butt_snapshot = document.current();
    assert_eq!(
        weldment.cut_list_export(&butt_snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );
    let butt = evaluated_fabrication(&butt_snapshot, &mut worker, FeatureId(6))
        .unwrap()
        .weldment
        .unwrap();
    assert_eq!(butt.rows[0].end_cut.treatment, WeldmentCutTreatment::Square);
    assert_close(butt.rows[0].end_cut.angle_degrees, 90.0);
    assert_eq!(butt.rows[1].start_cut.treatment, WeldmentCutTreatment::Butt);
    assert_close(butt.rows[1].start_cut.angle_degrees, 90.0);
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        original.canonical_digest()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: FeatureId(6) },
            CanonicalCommand::DeleteFeature { id: FeatureId(5) },
            CanonicalCommand::DeleteFeature { id: FeatureId(4) },
            CanonicalCommand::DeleteFeature { id: FeatureId(3) },
            CanonicalCommand::DeleteFeature { id: FeatureId(2) },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "First path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![line([-120.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "First member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile: FeatureId(1),
                    path: FeatureId(2),
                    orientation_degrees: 0.0,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Second path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![line([0.0, 0.0, 0.0], [0.0, 100.0, 0.0])],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(1),
                name: "Second member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile: FeatureId(1),
                    path: FeatureId(4),
                    orientation_degrees: 0.0,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(1),
                name: "Associative corner joint".into(),
                kind: FeatureKind::WeldmentJoint(WeldmentJointSpec {
                    first_member: FeatureId(3),
                    second_member: FeatureId(5),
                    policy: WeldmentJointPolicy::Miter,
                    primary: WeldmentJointPrimary::First,
                }),
            },
        ]))
        .unwrap();
    let edited = document.current();
    assert_eq!(
        weldment.cut_list_export(&edited),
        Err(GeneralFabricationError::ExportBlocked)
    );
    let regenerated = evaluated_fabrication(&edited, &mut worker, FeatureId(6))
        .unwrap()
        .weldment
        .unwrap();
    assert_close(regenerated.rows[0].centerline_length_mm, 120.0);
    assert_eq!(
        regenerated
            .rows
            .iter()
            .map(|row| (&row.stable_row_id, row.position))
            .collect::<Vec<_>>(),
        weldment
            .rows
            .iter()
            .map(|row| (&row.stable_row_id, row.position))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        original.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        edited.canonical_digest()
    );
}

#[test]
fn multisegment_weldment_has_exact_geometry_but_no_ambiguous_cut_list() {
    const OCCURRENCE: OccurrenceId = OccurrenceId(10);
    const ROLE_DIMENSION: ClassificationDimensionId = ClassificationDimensionId(20);
    const ROLE_CATEGORY: ClassificationCategoryId = ClassificationCategoryId(21);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Bent weldment".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Multisegment path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![
                        line([0.0, 0.0, 0.0], [0.0, 0.0, 50.0]),
                        line([0.0, 0.0, 50.0], [0.0, 0.0, 100.0]),
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Member".into(),
                kind: FeatureKind::WeldmentMember(WeldmentMemberSpec {
                    profile: FeatureId(1),
                    path: FeatureId(2),
                    orientation_degrees: 0.0,
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DefinitionId(1),
                name: "Bent member".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ROLE_DIMENSION,
                name: FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(ROLE_CATEGORY, MANUFACTURED_ITEM_ROLE_V1.into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OCCURRENCE,
                dimension_id: ROLE_DIMENSION,
                category_id: Some(ROLE_CATEGORY),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    assert_eq!(
        evaluated_fabrication(&snapshot, &mut worker, FeatureId(3)),
        Err(GeneralFabricationError::InvalidWeldmentGeometry)
    );
}
