use ketchup_core::document::{
    BooleanOperation, CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId,
    CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, InstancePath,
    MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, NodeId, OccurrenceId, Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest, ExactResultRegistry, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::exact_validation::{
    BuiltinGeneralBodyValidator, GeneralBodyParticipant, GeneralBodySource,
    GeneralBodyValidationError, GeneralClearanceCase, general_body_input_bytes,
    general_body_validation_policy,
};
use ketchup_core::fabrication::{
    GeneralFabricationError, GeneralFabricationProjection, GeneralManufacturingKind,
    ProjectionStatus, project_general_fabrication,
};
use ketchup_core::graph::{DerivedIdentity, PortSpec, RuleOutput, SlotPath, SlotSegment};
use ketchup_core::import::{StepImportMesh, StepMeshTriangle};
use ketchup_core::persistence;
use ketchup_core::prismatic::{Aabb, TolerancePolicy};
use ketchup_core::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceOwner, ClearanceSeverity,
    ClearanceValidationError, ClearanceVolumeId, SpaceId, validate_clearance_occupancy,
};
use ketchup_core::validation::{
    EvidenceClass, EvidenceCounts, HostNeutralValidator, VALIDATOR_ROLE_DIMENSION_V1,
    ValidationExecution, ValidationInvocation, ValidationState, ValidatorRoleError,
    ValidatorRoleIndex,
};
use std::sync::Arc;

const EXACT_DEFINITION: DefinitionId = DefinitionId(10);
const EXACT_PROFILE: FeatureId = FeatureId(11);
const EXACT_BODY: FeatureId = FeatureId(12);
const EXACT_LEFT: OccurrenceId = OccurrenceId(13);
const EXACT_RIGHT: OccurrenceId = OccurrenceId(14);
const MESH_DEFINITION: DefinitionId = DefinitionId(20);
const MESH_BODY: FeatureId = FeatureId(21);
const MESH_CLEAR: OccurrenceId = OccurrenceId(22);
const MESH_COLLIDING: OccurrenceId = OccurrenceId(23);
const GRAPH_DEFINITION: DefinitionId = DefinitionId(40);
const GRAPH_BASE_PROFILE: FeatureId = FeatureId(41);
const GRAPH_BASE_BODY: FeatureId = FeatureId(42);
const GRAPH_TOOL_PROFILE: FeatureId = FeatureId(43);
const GRAPH_TOOL_BODY: FeatureId = FeatureId(44);
const GRAPH_BOOLEAN: FeatureId = FeatureId(45);
const GRAPH_LEFT: OccurrenceId = OccurrenceId(46);
const GRAPH_RIGHT: OccurrenceId = OccurrenceId(47);
const SPACE_LEFT: SpaceId = SpaceId(30);
const SPACE_RIGHT: SpaceId = SpaceId(31);
const CLEARANCE: ClearanceVolumeId = ClearanceVolumeId(32);
const SPACE_RULE: NodeId = NodeId(33);
const ROLE_DIMENSION: ClassificationDimensionId = ClassificationDimensionId(900);
const ROLE_CATEGORY_SUBJECT: ClassificationCategoryId = ClassificationCategoryId(901);
const ROLE_CATEGORY_SUPPORT: ClassificationCategoryId = ClassificationCategoryId(902);

#[test]
fn validator_roles_are_explicit_name_invariant_and_deterministic() {
    let mut document = exact_only_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ROLE_DIMENSION,
                name: VALIDATOR_ROLE_DIMENSION_V1.to_owned(),
                categories: vec![
                    (ROLE_CATEGORY_SUBJECT, "structure.loaded-subject".to_owned()),
                    (ROLE_CATEGORY_SUPPORT, "structure.support".to_owned()),
                ],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: EXACT_LEFT,
                dimension_id: ROLE_DIMENSION,
                category_id: Some(ROLE_CATEGORY_SUBJECT),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: EXACT_RIGHT,
                dimension_id: ROLE_DIMENSION,
                category_id: Some(ROLE_CATEGORY_SUPPORT),
            },
        ]))
        .unwrap();

    let before = ValidatorRoleIndex::from_snapshot(&document.current()).unwrap();
    assert_eq!(before.dimension_id(), ROLE_DIMENSION);
    assert_eq!(
        before.role(EXACT_LEFT).unwrap().as_str(),
        "structure.loaded-subject"
    );
    assert_eq!(
        before.role(EXACT_RIGHT).unwrap().as_str(),
        "structure.support"
    );
    assert_eq!(before.assignments().count(), 2);
    let input_before_rename = before.input_bytes();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameEntity {
                id: EXACT_LEFT,
                name: "support-looking shelf room passage".to_owned(),
            },
            CanonicalCommand::RenameEntity {
                id: EXACT_RIGHT,
                name: "unrelated opaque name".to_owned(),
            },
        ]))
        .unwrap();
    let after = ValidatorRoleIndex::from_snapshot(&document.current()).unwrap();
    assert_eq!(after, before);
    assert_eq!(after.input_bytes(), input_before_rename);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    let reopened_index = ValidatorRoleIndex::from_snapshot(&reopened.snapshot()).unwrap();
    assert_eq!(reopened_index, after);
    assert_eq!(reopened_index.input_bytes(), input_before_rename);
}

#[test]
fn validator_role_schema_fails_closed_when_missing_ambiguous_or_invalid() {
    let missing = exact_only_document();
    assert_eq!(
        ValidatorRoleIndex::from_snapshot(&missing.current()),
        Err(ValidatorRoleError::DimensionMissing)
    );

    let mut invalid = exact_only_document();
    invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ROLE_DIMENSION,
                name: VALIDATOR_ROLE_DIMENSION_V1.to_owned(),
                categories: vec![(ROLE_CATEGORY_SUBJECT, "invalid role with spaces".to_owned())],
            },
        ]))
        .unwrap();
    assert_eq!(
        ValidatorRoleIndex::from_snapshot(&invalid.current()),
        Err(ValidatorRoleError::InvalidRole(
            "invalid role with spaces".to_owned()
        ))
    );

    let mut ambiguous = exact_only_document();
    ambiguous
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ROLE_DIMENSION,
                name: VALIDATOR_ROLE_DIMENSION_V1.to_owned(),
                categories: vec![(ROLE_CATEGORY_SUBJECT, "structure.subject".to_owned())],
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(910),
                name: VALIDATOR_ROLE_DIMENSION_V1.to_owned(),
                categories: vec![(
                    ClassificationCategoryId(911),
                    "structure.support".to_owned(),
                )],
            },
        ]))
        .unwrap();
    assert_eq!(
        ValidatorRoleIndex::from_snapshot(&ambiguous.current()),
        Err(ValidatorRoleError::DimensionAmbiguous)
    );
}

#[test]
fn general_collision_and_clearance_bind_current_exact_and_mesh_occurrences() {
    let mut document = mixed_document();
    let snapshot = document.current();
    let package = exact_package(&snapshot);
    let registry =
        ExactResultRegistry::accept(&snapshot, [Arc::new(ExactBodyPackage::from(package))])
            .unwrap();
    let tolerance = TolerancePolicy::default();

    let exact_left = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_LEFT),
        tolerance,
    )
    .unwrap();
    let exact_right = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_RIGHT),
        tolerance,
    )
    .unwrap();
    let mesh_clear = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(MESH_CLEAR),
        tolerance,
    )
    .unwrap();
    let mesh_colliding = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(MESH_COLLIDING),
        tolerance,
    )
    .unwrap();
    assert_eq!(exact_left.evidence_class(), &EvidenceClass::Exact);
    assert!(matches!(
        mesh_clear.evidence_class(),
        EvidenceClass::Tolerant(_)
    ));

    let passing_cases = vec![
        GeneralClearanceCase::new(exact_left.clone(), exact_right.clone(), 10.0).unwrap(),
        GeneralClearanceCase::new(exact_right, mesh_clear.clone(), 10.0).unwrap(),
    ];
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let input = general_body_input_bytes(&passing_cases);
    let invocation =
        ValidationInvocation::bind(&snapshot, validator.descriptor(), &policy, vec![], &input);
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation: invocation.clone(),
        policy: &policy,
        input: &passing_cases,
    });
    assert_eq!(report.state, ValidationState::Passed);
    assert_eq!(
        report.evidence_counts,
        EvidenceCounts {
            exact: 1,
            tolerant: 1,
        }
    );
    assert_eq!(report.diagnostics[0].code, "clearance.minimum-satisfied");
    assert_eq!(report.diagnostics[1].code, "clearance.minimum-satisfied");
    assert!(
        report.diagnostics[1]
            .evidence
            .contains("right=occurrence:22")
    );

    let colliding_cases = vec![GeneralClearanceCase::new(mesh_clear, mesh_colliding, 0.0).unwrap()];
    let collision_input = general_body_input_bytes(&colliding_cases);
    let collision_invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        vec![],
        &collision_input,
    );
    let collision_report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation: collision_invocation,
        policy: &policy,
        input: &colliding_cases,
    });
    assert_eq!(collision_report.state, ValidationState::Failed);
    assert_eq!(collision_report.diagnostics[0].code, "collision.detected");
    assert_eq!(
        collision_report.evidence_counts,
        EvidenceCounts {
            exact: 0,
            tolerant: 1,
        }
    );

    let canonical_extrusion = GeneralBodyParticipant::accept(
        &snapshot,
        &ExactResultRegistry::default(),
        InstancePath::root(EXACT_LEFT),
        tolerance,
    )
    .unwrap();
    assert!(matches!(
        canonical_extrusion.source(),
        GeneralBodySource::CanonicalExtrusion {
            definition_id: EXACT_DEFINITION,
            profile_id: EXACT_PROFILE,
            extrusion_id: EXACT_BODY,
            ..
        }
    ));
    assert_eq!(canonical_extrusion.evidence_class(), &EvidenceClass::Exact);
    assert_eq!(
        canonical_extrusion.bounds(),
        Aabb::bounded_volume([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).unwrap()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: EXACT_RIGHT,
                transform: Transform::from_translation(21.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let current = document.current();
    let stale_report = validator.invoke(ValidationExecution {
        snapshot: &current,
        invocation,
        policy: &policy,
        input: &passing_cases,
    });
    assert_eq!(stale_report.state, ValidationState::NotEvaluated);
    assert_eq!(
        stale_report.diagnostics[0].evidence,
        "snapshot binding is stale or mismatched"
    );
    assert_eq!(
        GeneralBodyParticipant::accept(
            &current,
            &registry,
            InstancePath::root(EXACT_LEFT),
            tolerance,
        ),
        Err(GeneralBodyValidationError::StaleExactResult)
    );
}

#[test]
fn general_fabrication_regenerates_deterministically_and_exports_fail_closed() {
    let mut document = exact_only_document();
    let snapshot = document.current();
    let package = exact_package(&snapshot);
    let registry =
        ExactResultRegistry::accept(&snapshot, [Arc::new(ExactBodyPackage::from(package))])
            .unwrap();
    let tolerance = TolerancePolicy::default();
    let left = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_LEFT),
        tolerance,
    )
    .unwrap();
    let right = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_RIGHT),
        tolerance,
    )
    .unwrap();
    let cases = vec![GeneralClearanceCase::new(left, right, 10.0).unwrap()];
    let report = general_report(&snapshot, &cases, tolerance);
    assert_eq!(report.state, ValidationState::Passed);

    let projection =
        project_general_fabrication(&snapshot, &registry, &cases, &report, tolerance).unwrap();
    let regenerated =
        project_general_fabrication(&snapshot, &registry, &cases, &report, tolerance).unwrap();
    assert_eq!(projection, regenerated);
    assert_eq!(projection.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(projection.bom.rows.len(), 1);
    assert_eq!(projection.bom.rows[0].quantity, 2);
    assert_eq!(projection.bom.rows[0].dimensions.length_mm, 10.0);
    assert_eq!(projection.drawings.drawings.len(), 1);
    assert_eq!(projection.drawings.drawings[0].views.len(), 3);
    assert_eq!(projection.drawings.drawings[0].dimensions.len(), 3);
    assert_eq!(projection.manufacturing.operations.len(), 1);
    assert_eq!(
        projection.manufacturing.operations[0].kind,
        GeneralManufacturingKind::Stock
    );
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    assert!(
        String::from_utf8(projection.bom_export(&snapshot).unwrap())
            .unwrap()
            .contains("quantity=2;length_mm=10;width_mm=10;height_mm=10")
    );
    assert!(
        String::from_utf8(projection.drawing_svg(&snapshot).unwrap())
            .unwrap()
            .contains("ketchup.general-drawing-svg.v1")
    );
    assert!(
        String::from_utf8(projection.manufacturing_export(&snapshot).unwrap())
            .unwrap()
            .contains("kind=stock;frame=definition-local")
    );

    let mut tampered = projection.clone();
    tampered.bom.rows[0].quantity = 3;
    assert_eq!(
        tampered.bom_export(&snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );
    let mut wrong_evaluator = projection.clone();
    wrong_evaluator.drawings.envelope.evaluator_id = "ketchup.tampered-evaluator.v1";
    assert_eq!(
        wrong_evaluator.drawing_svg(&snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: EXACT_RIGHT,
                transform: Transform::from_translation(21.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let current = document.current();
    assert_eq!(
        projection.bom_export(&current),
        Err(GeneralFabricationError::ExportBlocked)
    );
    assert_eq!(
        projection.drawing_svg(&current),
        Err(GeneralFabricationError::ExportBlocked)
    );
    assert_eq!(
        projection.manufacturing_export(&current),
        Err(GeneralFabricationError::ExportBlocked)
    );

    let mut scaled = exact_only_document();
    scaled
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: EXACT_LEFT,
                transform: Transform::from_matrix([
                    2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
            },
        ]))
        .unwrap();
    let scaled_snapshot = scaled.current();
    let scaled_registry = ExactResultRegistry::accept(
        &scaled_snapshot,
        [Arc::new(ExactBodyPackage::from(exact_package(
            &scaled_snapshot,
        )))],
    )
    .unwrap();
    let scaled_left = GeneralBodyParticipant::accept(
        &scaled_snapshot,
        &scaled_registry,
        InstancePath::root(EXACT_LEFT),
        tolerance,
    )
    .unwrap();
    let scaled_right = GeneralBodyParticipant::accept(
        &scaled_snapshot,
        &scaled_registry,
        InstancePath::root(EXACT_RIGHT),
        tolerance,
    )
    .unwrap();
    let scaled_cases = vec![GeneralClearanceCase::new(scaled_left, scaled_right, 0.0).unwrap()];
    let scaled_report = general_report(&scaled_snapshot, &scaled_cases, tolerance);
    assert_eq!(
        project_general_fabrication(
            &scaled_snapshot,
            &scaled_registry,
            &scaled_cases,
            &scaled_report,
            tolerance,
        ),
        Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry)
    );
}

#[test]
fn exact_brep_graph_boolean_cut_emits_host_neutral_manufacturing_evidence() {
    let (snapshot, projection) =
        graph_fabrication_projection(BooleanOperation::Cut, true, "m17-graph-result");
    assert_eq!(projection.bom.rows.len(), 1);
    assert_eq!(projection.bom.rows[0].quantity, 2);
    assert_eq!(projection.drawings.drawings.len(), 1);
    assert_eq!(
        projection.manufacturing.envelope.status,
        ProjectionStatus::Complete
    );
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    assert_eq!(projection.manufacturing.operations.len(), 2);
    assert_eq!(
        projection
            .manufacturing
            .operations
            .iter()
            .map(|operation| operation.kind)
            .collect::<Vec<_>>(),
        vec![
            GeneralManufacturingKind::Stock,
            GeneralManufacturingKind::BooleanCut,
        ]
    );
    let stock = projection
        .manufacturing
        .operations
        .iter()
        .find(|operation| operation.kind == GeneralManufacturingKind::Stock)
        .unwrap();
    assert_eq!(stock.producer_feature_id, GRAPH_BASE_BODY);
    assert!(stock.semantic_inputs.is_empty());
    assert_eq!(stock.bounds.length_mm, 10.0);
    let cut = projection
        .manufacturing
        .operations
        .iter()
        .find(|operation| operation.kind == GeneralManufacturingKind::BooleanCut)
        .unwrap();
    assert_eq!(cut.producer_feature_id, GRAPH_BOOLEAN);
    assert_eq!(cut.semantic_inputs, vec![GRAPH_BASE_BODY, GRAPH_TOOL_BODY]);
    assert_eq!(cut.bounds.length_mm, 8.0);
    assert!(
        projection
            .manufacturing
            .operations
            .iter()
            .all(|operation| operation.frame == "definition-local"
                && operation.source.result_fingerprint == "m17-graph-result")
    );
    let export = String::from_utf8(projection.manufacturing_export(&snapshot).unwrap()).unwrap();
    let stock_position = export
        .find("producer=42;kind=stock;frame=definition-local;inputs=")
        .unwrap();
    let cut_position = export
        .find("producer=45;kind=boolean-cut;frame=definition-local;inputs=42,44")
        .unwrap();
    assert!(stock_position < cut_position);
}

#[test]
fn unsupported_or_unverified_exact_graph_manufacturing_fails_closed_atomically() {
    for operation in [
        BooleanOperation::Union,
        BooleanOperation::Intersect,
        BooleanOperation::Split,
    ] {
        let (snapshot, projection) =
            graph_fabrication_projection(operation, true, "m17-graph-result");
        assert_eq!(projection.bom.rows.len(), 1);
        assert_eq!(projection.drawings.drawings.len(), 1);
        assert!(projection.bom_export(&snapshot).is_ok());
        assert!(projection.drawing_svg(&snapshot).is_ok());
        assert_eq!(
            projection.manufacturing.envelope.status,
            ProjectionStatus::Incomplete
        );
        assert!(projection.manufacturing.operations.is_empty());
        assert_eq!(projection.manufacturing.unresolved_sources.len(), 1);
        assert_eq!(
            projection.manufacturing_export(&snapshot),
            Err(GeneralFabricationError::ExportBlocked)
        );
    }

    let (snapshot, projection) =
        graph_fabrication_projection(BooleanOperation::Cut, false, "m17-graph-result");
    assert!(matches!(
        projection.bom.rows[0].source,
        GeneralBodySource::CanonicalExactGraph { .. }
    ));
    assert!(projection.manufacturing.operations.is_empty());
    assert_eq!(projection.manufacturing.unresolved_sources.len(), 1);
    assert_eq!(
        projection.manufacturing_export(&snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );

    let (snapshot, projection) = graph_fabrication_projection(
        BooleanOperation::Cut,
        true,
        "safe\noperation=forged;kind=stock",
    );
    assert_eq!(
        projection.manufacturing.envelope.status,
        ProjectionStatus::Incomplete
    );
    assert!(projection.manufacturing.operations.is_empty());
    assert_eq!(projection.manufacturing.unresolved_sources.len(), 1);
    assert_eq!(
        projection.manufacturing_export(&snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );
}

#[test]
fn canonical_mesh_keeps_bom_and_drawings_but_blocks_manufacturing_export() {
    let document = mixed_document();
    let snapshot = document.current();
    let package = exact_package(&snapshot);
    let registry =
        ExactResultRegistry::accept(&snapshot, [Arc::new(ExactBodyPackage::from(package))])
            .unwrap();
    let tolerance = TolerancePolicy::default();
    let exact_left = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_LEFT),
        tolerance,
    )
    .unwrap();
    let exact_right = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(EXACT_RIGHT),
        tolerance,
    )
    .unwrap();
    let mesh_clear = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(MESH_CLEAR),
        tolerance,
    )
    .unwrap();
    let mesh_colliding = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(MESH_COLLIDING),
        tolerance,
    )
    .unwrap();
    let cases = vec![
        GeneralClearanceCase::new(exact_left.clone(), exact_right.clone(), 10.0).unwrap(),
        GeneralClearanceCase::new(exact_right, mesh_clear, 10.0).unwrap(),
        GeneralClearanceCase::new(exact_left, mesh_colliding, 30.0).unwrap(),
    ];
    let report = general_report(&snapshot, &cases, tolerance);
    assert_eq!(report.state, ValidationState::Passed);

    let projection =
        project_general_fabrication(&snapshot, &registry, &cases, &report, tolerance).unwrap();
    assert_eq!(projection.bom.rows.len(), 2);
    assert_eq!(projection.bom.evidence_counts.exact, 2);
    assert_eq!(projection.bom.evidence_counts.tolerant, 2);
    assert_eq!(projection.drawings.drawings.len(), 2);
    assert!(projection.bom_export(&snapshot).is_ok());
    assert!(projection.drawing_svg(&snapshot).is_ok());
    assert_eq!(
        projection.manufacturing.envelope.status,
        ProjectionStatus::Incomplete
    );
    assert_eq!(projection.manufacturing.operations.len(), 1);
    assert_eq!(projection.manufacturing.unresolved_sources.len(), 1);
    assert_eq!(
        projection.manufacturing_export(&snapshot),
        Err(GeneralFabricationError::ExportBlocked)
    );
}

#[test]
fn canonical_space_and_rule_clearance_round_trip_and_fail_closed_when_slot_is_lost() {
    let tolerance = TolerancePolicy::default();
    let slot = SlotSegment::new(SPACE_RULE, "clearances", "door-swing").unwrap();
    let identity =
        DerivedIdentity::new(SPACE_RULE, SlotPath::new(vec![slot.clone()]).unwrap()).unwrap();
    let mut document = exact_only_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: SPACE_RULE,
                name: "clearance rule".to_owned(),
                expression: "1".to_owned(),
                input_ports: vec![],
                output_ports: vec![PortSpec::number("clearances").unwrap()],
                outputs: vec![RuleOutput::new(slot, vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertSpace(
                CanonicalSpace::new(
                    SPACE_LEFT,
                    "bedroom",
                    Aabb::bounded_volume([-5.0, -5.0, -5.0], [15.0, 15.0, 15.0]).unwrap(),
                    vec![SPACE_RIGHT],
                    vec![SPACE_RIGHT],
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertSpace(
                CanonicalSpace::new(
                    SPACE_RIGHT,
                    "corridor",
                    Aabb::bounded_volume([15.0, -5.0, -5.0], [35.0, 15.0, 15.0]).unwrap(),
                    vec![SPACE_LEFT],
                    vec![],
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertClearanceVolume(
                CanonicalClearanceVolume::new(
                    CLEARANCE,
                    ClearanceOwner::Space(SPACE_RIGHT),
                    "door swing",
                    Aabb::bounded_volume([19.0, 0.0, 0.0], [31.0, 10.0, 10.0]).unwrap(),
                    tolerance,
                    ClearanceSeverity::Required,
                    Some(identity),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    let snapshot = document.current();
    let registry = ExactResultRegistry::accept(
        &snapshot,
        [Arc::new(ExactBodyPackage::from(exact_package(&snapshot)))],
    )
    .unwrap();
    let result = validate_clearance_occupancy(
        &snapshot,
        &registry,
        CLEARANCE,
        [
            InstancePath::root(EXACT_RIGHT),
            InstancePath::root(EXACT_LEFT),
        ],
    )
    .unwrap();
    assert_eq!(result.occupants, vec![InstancePath::root(EXACT_RIGHT)]);
    assert_eq!(
        result.evidence_counts,
        EvidenceCounts {
            exact: 2,
            tolerant: 0,
        }
    );

    let saved_digest = snapshot.canonical_digest();
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), saved_digest);
    assert_eq!(
        reopened.snapshot().space(SPACE_LEFT).unwrap().purpose(),
        "bedroom"
    );
    assert_eq!(
        reopened
            .snapshot()
            .clearance_volume(CLEARANCE)
            .unwrap()
            .derived_from(),
        snapshot.clearance_volume(CLEARANCE).unwrap().derived_from()
    );

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: SPACE_RULE,
            outputs: vec![
                RuleOutput::new(
                    SlotSegment::new(SPACE_RULE, "clearances", "window-swing").unwrap(),
                    vec![],
                )
                .unwrap(),
            ],
        }]))
        .unwrap();
    assert_eq!(
        validate_clearance_occupancy(
            &document.current(),
            &registry,
            CLEARANCE,
            [InstancePath::root(EXACT_RIGHT)],
        ),
        Err(ClearanceValidationError::UnresolvedDerivedIdentity)
    );

    let mut invalid = DocumentStore::new();
    let before = invalid.current();
    let asymmetric = CanonicalSpace::new(
        SPACE_LEFT,
        "invalid",
        Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap(),
        vec![SPACE_RIGHT],
        vec![],
    )
    .unwrap();
    assert!(
        invalid
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
                asymmetric,
            )]))
            .is_err()
    );
    assert_eq!(invalid.current().revision_id(), before.revision_id());
    assert_eq!(
        invalid.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(invalid.current().spaces().count(), 0);
}

fn general_report(
    snapshot: &ketchup_core::document::Snapshot,
    cases: &[GeneralClearanceCase],
    tolerance: TolerancePolicy,
) -> ketchup_core::validation::ValidationReport {
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let input = general_body_input_bytes(cases);
    let invocation =
        ValidationInvocation::bind(snapshot, validator.descriptor(), &policy, vec![], &input);
    validator.invoke(ValidationExecution {
        snapshot,
        invocation,
        policy: &policy,
        input: cases,
    })
}

fn graph_fabrication_projection(
    operation: BooleanOperation,
    verified: bool,
    result_fingerprint: &str,
) -> (Snapshot, GeneralFabricationProjection) {
    let document = graph_boolean_document(operation);
    let snapshot = document.current();
    let registry = if verified {
        ExactResultRegistry::accept(
            &snapshot,
            [Arc::new(ExactBodyPackage::from(graph_package(
                &snapshot,
                result_fingerprint,
            )))],
        )
        .unwrap()
    } else {
        ExactResultRegistry::default()
    };
    let tolerance = TolerancePolicy::default();
    let left = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(GRAPH_LEFT),
        tolerance,
    )
    .unwrap();
    let right = GeneralBodyParticipant::accept(
        &snapshot,
        &registry,
        InstancePath::root(GRAPH_RIGHT),
        tolerance,
    )
    .unwrap();
    let cases = vec![GeneralClearanceCase::new(left, right, 5.0).unwrap()];
    let report = general_report(&snapshot, &cases, tolerance);
    assert_eq!(report.state, ValidationState::Passed);
    let projection =
        project_general_fabrication(&snapshot, &registry, &cases, &report, tolerance).unwrap();
    assert_eq!(
        projection,
        project_general_fabrication(&snapshot, &registry, &cases, &report, tolerance).unwrap()
    );
    (snapshot, projection)
}

fn graph_boolean_document(operation: BooleanOperation) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: GRAPH_DEFINITION,
                name: "Graph fabrication".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: GRAPH_BASE_PROFILE,
                definition_id: GRAPH_DEFINITION,
                name: "Base profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: GRAPH_BASE_BODY,
                definition_id: GRAPH_DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: GRAPH_BASE_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: GRAPH_TOOL_PROFILE,
                definition_id: GRAPH_DEFINITION,
                name: "Tool profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[8.0, 0.0], [10.0, 0.0], [10.0, 10.0], [8.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: GRAPH_TOOL_BODY,
                definition_id: GRAPH_DEFINITION,
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: GRAPH_TOOL_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: GRAPH_BOOLEAN,
                definition_id: GRAPH_DEFINITION,
                name: "Boolean result".to_owned(),
                kind: FeatureKind::Boolean {
                    operation,
                    target: GRAPH_BASE_BODY,
                    tool: GRAPH_TOOL_BODY,
                },
            },
            occurrence(GRAPH_LEFT, GRAPH_DEFINITION, 0.0),
            occurrence(GRAPH_RIGHT, GRAPH_DEFINITION, 20.0),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn graph_package(snapshot: &Snapshot, result_fingerprint: &str) -> ExactBRepGraphPackage {
    let graph = ExactBRepGraph::from_snapshot(snapshot, GRAPH_DEFINITION, GRAPH_BOOLEAN).unwrap();
    let mut bounds_mm = graph.producer_bounds_mm().unwrap().unwrap();
    if matches!(
        snapshot
            .feature(GRAPH_BOOLEAN)
            .map(|feature| feature.kind()),
        Some(FeatureKind::Boolean {
            operation: BooleanOperation::Cut,
            ..
        })
    ) {
        bounds_mm[1][0] = 8.0;
    }
    let [minimum, maximum] = bounds_mm;
    let mesh = StepImportMesh {
        vertices_mm: vec![
            [minimum[0], minimum[1], minimum[2]],
            [maximum[0], minimum[1], minimum[2]],
            [maximum[0], maximum[1], minimum[2]],
            [minimum[0], maximum[1], minimum[2]],
            [minimum[0], minimum[1], maximum[2]],
            [maximum[0], minimum[1], maximum[2]],
            [maximum[0], maximum[1], maximum[2]],
            [minimum[0], maximum[1], maximum[2]],
        ],
        triangles: vec![StepMeshTriangle {
            vertex_indices: [0, 1, 2],
            face_ordinal: 0,
        }],
    };
    ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: "m17-graph-input".to_owned(),
            result_fingerprint: result_fingerprint.to_owned(),
            volume_mm3: (maximum[0] - minimum[0])
                * (maximum[1] - minimum[1])
                * (maximum[2] - minimum[2]),
            area_mm2: 0.0,
            topology_counts: [8, 12, 6, 1, 1],
            wire_count: None,
            bounds_mm,
            backend: "m17-graph-backend".to_owned(),
            tolerance: "m17-graph-tolerance".to_owned(),
        },
        &mesh,
    )
    .unwrap()
}

fn exact_only_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: EXACT_DEFINITION,
                name: "Exact box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: EXACT_PROFILE,
                definition_id: EXACT_DEFINITION,
                name: "Exact profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXACT_BODY,
                definition_id: EXACT_DEFINITION,
                name: "Exact body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: EXACT_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            occurrence(EXACT_LEFT, EXACT_DEFINITION, 0.0),
            occurrence(EXACT_RIGHT, EXACT_DEFINITION, 20.0),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn mixed_document() -> DocumentStore {
    let mesh = MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        authority: MeshAuthority::Authored {
            provenance: "m17-general-validation-fixture".to_owned(),
        },
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: EXACT_DEFINITION,
                name: "Exact box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: EXACT_PROFILE,
                definition_id: EXACT_DEFINITION,
                name: "Exact profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXACT_BODY,
                definition_id: EXACT_DEFINITION,
                name: "Exact body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: EXACT_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            occurrence(EXACT_LEFT, EXACT_DEFINITION, 0.0),
            occurrence(EXACT_RIGHT, EXACT_DEFINITION, 20.0),
            CanonicalCommand::CreateDefinition {
                id: MESH_DEFINITION,
                name: "Canonical mesh".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: MESH_BODY,
                definition_id: MESH_DEFINITION,
                name: "Mesh body".to_owned(),
                kind: FeatureKind::MeshBody(mesh),
            },
            occurrence(MESH_CLEAR, MESH_DEFINITION, 40.0),
            occurrence(MESH_COLLIDING, MESH_DEFINITION, 45.0),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn occurrence(id: OccurrenceId, definition_id: DefinitionId, x_mm: f64) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id,
        definition_id,
        name: format!("Occurrence {}", id.0),
        transform: Transform::from_translation(x_mm, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    }
}

fn exact_package(
    snapshot: &ketchup_core::document::Snapshot,
) -> ketchup_core::exact_product::ExactRenderPackage {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, EXACT_DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                request.document_id,
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                "planar_face",
            ),
            format!("geometry:{role:?}"),
        )
    });
    build_box_render_package(
        &request,
        "m17-exact-input".to_owned(),
        "m17-exact-result".to_owned(),
        "m17-backend".to_owned(),
        "m17-tolerance".to_owned(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        evidence,
    )
    .unwrap()
}
