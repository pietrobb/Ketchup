use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, NodeId, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{
    ExactFaceRole, ExactFeatureChainRequest, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::exact_validation::*;
use ketchup_core::fabrication::exact_parallel_face_dimension;
use ketchup_core::graph::{
    DerivedIdentity, OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotSegment,
};
use ketchup_core::prismatic::{Aabb, CanonicalJoint, JointId, TolerancePolicy};
use ketchup_core::validation::{
    EvidenceClass, EvidenceCounts, HostNeutralValidator, ValidationExecution, ValidationInvocation,
    ValidationState,
};

const PLAIN_DEFINITION: DefinitionId = DefinitionId(10);
const PLAIN_PROFILE: FeatureId = FeatureId(11);
const PLAIN_EXTRUSION: FeatureId = FeatureId(12);
const BASE: OccurrenceId = OccurrenceId(13);
const CLEAR: OccurrenceId = OccurrenceId(14);
const JOINTED: OccurrenceId = OccurrenceId(15);
const CLEAR_DUPLICATE: OccurrenceId = OccurrenceId(16);
const CUT_DEFINITION: DefinitionId = DefinitionId(20);
const CUT_PROFILE: FeatureId = FeatureId(21);
const CUT_EXTRUSION: FeatureId = FeatureId(22);
const OPENING_PROFILE: FeatureId = FeatureId(23);
const THROUGH_CUT: FeatureId = FeatureId(24);
const CUT_LEFT: OccurrenceId = OccurrenceId(25);
const CUT_RIGHT: OccurrenceId = OccurrenceId(26);
const INPUT_NODE: NodeId = NodeId(100);
const RULE_NODE: NodeId = NodeId(101);
const OUTPUT_PORT: &str = "exact-pieces";

#[test]
fn exact_clearance_dimensions_and_joint_propagate_exact_and_tolerant_evidence() {
    let mut document = exact_fixture();
    let snapshot = document.current();
    let tolerance = TolerancePolicy::default();
    let plain_package = package(&snapshot, PLAIN_DEFINITION);
    let cut_package = package(&snapshot, CUT_DEFINITION);

    let base = participant(&snapshot, "base", BASE, &plain_package, tolerance);
    let clear = participant(&snapshot, "clear", CLEAR, &plain_package, tolerance);
    let clear_duplicate = participant(
        &snapshot,
        "clear-duplicate",
        CLEAR_DUPLICATE,
        &plain_package,
        tolerance,
    );
    let jointed = participant(&snapshot, "jointed", JOINTED, &plain_package, tolerance);
    let cut_left = participant(&snapshot, "cut-left", CUT_LEFT, &cut_package, tolerance);
    let cut_right = participant(&snapshot, "cut-right", CUT_RIGHT, &cut_package, tolerance);
    assert_eq!(base.evidence_class, EvidenceClass::Exact);
    assert!(matches!(
        cut_left.evidence_class,
        EvidenceClass::Tolerant(_)
    ));

    let joint = snapshot.joint(JointId(1)).unwrap().clone();
    let cases = vec![
        ExactValidationCase::Clearance(Box::new(
            ExactClearanceCase::new(base.clone(), clear.clone(), 10.0).unwrap(),
        )),
        ExactValidationCase::Joint(Box::new(ExactJointCase {
            left_contact: base.reference(ExactFaceRole::Top).unwrap().clone(),
            right_contact: jointed.reference(ExactFaceRole::Bottom).unwrap().clone(),
            left: base.clone(),
            right: jointed.clone(),
            declared_joint: joint,
        })),
        ExactValidationCase::Clearance(Box::new(
            ExactClearanceCase::new(cut_left.clone(), cut_right.clone(), 10.0).unwrap(),
        )),
    ];
    let validator = BuiltinExactValidator::new(tolerance);
    let policy = exact_validation_policy();
    let input = exact_input_bytes(&cases);
    let accepted_derived = cases
        .iter()
        .flat_map(|case| match case {
            ExactValidationCase::Clearance(case) => [
                case.left.derived_identity.clone(),
                case.right.derived_identity.clone(),
            ],
            ExactValidationCase::Joint(case) => [
                case.left.derived_identity.clone(),
                case.right.derived_identity.clone(),
            ],
        })
        .collect();
    let accepted_exact = cases
        .iter()
        .flat_map(|case| match case {
            ExactValidationCase::Clearance(case) => [
                case.left.result_identity.clone(),
                case.right.result_identity.clone(),
            ],
            ExactValidationCase::Joint(case) => [
                case.left.result_identity.clone(),
                case.right.result_identity.clone(),
            ],
        })
        .collect();
    let invocation = ValidationInvocation::bind_with_exact_results(
        &snapshot,
        validator.descriptor(),
        &policy,
        accepted_derived,
        accepted_exact,
        &input,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation: invocation.clone(),
        policy: &policy,
        input: &cases,
    });
    assert_eq!(report.state, ValidationState::Passed);
    assert_eq!(
        report.evidence_counts,
        EvidenceCounts {
            exact: 2,
            tolerant: 1,
        }
    );
    assert_eq!(report.diagnostics[0].code, "clearance.minimum-satisfied");
    assert!(
        report.diagnostics[0]
            .evidence
            .contains("minimum_gap_mm=10.000000000")
    );
    assert_eq!(report.diagnostics[1].code, "joint.exact-contact-validated");
    assert_eq!(report.diagnostics[2].code, "clearance.minimum-satisfied");
    assert!(matches!(
        report.diagnostics[2].evidence_class,
        EvidenceClass::Tolerant(_)
    ));
    assert_eq!(
        report.diagnostics[1].location.exact_body.as_ref(),
        Some(&plain_package.identity)
    );

    let exact_dimension = exact_parallel_face_dimension(
        &snapshot,
        "plain/east-to-east",
        &base,
        base.reference(ExactFaceRole::East).unwrap(),
        &clear,
        clear.reference(ExactFaceRole::East).unwrap(),
        tolerance,
    )
    .unwrap();
    assert_eq!(exact_dimension.axis, 0);
    assert_eq!(exact_dimension.value_mm, 110.0);
    assert_eq!(exact_dimension.evidence_class, EvidenceClass::Exact);
    assert!(exact_dimension.envelope.is_current(&snapshot));
    let duplicate_target_dimension = exact_parallel_face_dimension(
        &snapshot,
        "plain/east-to-east",
        &base,
        base.reference(ExactFaceRole::East).unwrap(),
        &clear_duplicate,
        clear_duplicate.reference(ExactFaceRole::East).unwrap(),
        tolerance,
    )
    .unwrap();
    assert_eq!(
        duplicate_target_dimension.value_mm,
        exact_dimension.value_mm
    );
    assert_ne!(
        duplicate_target_dimension.envelope.result_digest,
        exact_dimension.envelope.result_digest
    );

    let tolerant_dimension = exact_parallel_face_dimension(
        &snapshot,
        "cut/east-wall-to-east-wall",
        &cut_left,
        cut_left.reference(ExactFaceRole::CutEast).unwrap(),
        &cut_right,
        cut_right.reference(ExactFaceRole::CutEast).unwrap(),
        tolerance,
    )
    .unwrap();
    assert_eq!(tolerant_dimension.value_mm, 110.0);
    assert!(matches!(
        tolerant_dimension.evidence_class,
        EvidenceClass::Tolerant(_)
    ));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: CLEAR,
                transform: Transform::from_translation(111.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let stale_report = validator.invoke(ValidationExecution {
        snapshot: &document.current(),
        invocation,
        policy: &policy,
        input: &cases,
    });
    assert_eq!(stale_report.state, ValidationState::NotEvaluated);
    assert_eq!(
        stale_report.diagnostics[0].evidence,
        "snapshot binding is stale or mismatched"
    );
}

fn exact_fixture() -> DocumentStore {
    let identities = [
        "base",
        "clear",
        "clear-duplicate",
        "jointed",
        "cut-left",
        "cut-right",
    ];
    let outputs = identities
        .into_iter()
        .map(|key| RuleOutput::new(segment(key), vec![]).unwrap())
        .collect();
    let base_identity = identity("base");
    let jointed_identity = identity("jointed");
    let joint = CanonicalJoint::new(
        JointId(1),
        base_identity,
        jointed_identity,
        Aabb::bounded_volume([0.0, 0.0, 15.0], [100.0, 60.0, 20.0]).unwrap(),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: INPUT_NODE,
                name: "exact fixture input".to_owned(),
                dimension: Dimension::new("1", 1.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE_NODE,
                name: "exact fixture rule".to_owned(),
                expression: "$100".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number(OUTPUT_PORT).unwrap()],
                outputs,
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::CreateDefinition {
                id: PLAIN_DEFINITION,
                name: "plain exact box".to_owned(),
            },
            profile(PLAIN_PROFILE, PLAIN_DEFINITION, [0.0, 0.0, 100.0, 60.0]),
            extrusion(PLAIN_EXTRUSION, PLAIN_DEFINITION, PLAIN_PROFILE),
            occurrence(BASE, PLAIN_DEFINITION, [0.0, 0.0, 0.0]),
            occurrence(CLEAR, PLAIN_DEFINITION, [110.0, 0.0, 0.0]),
            occurrence(CLEAR_DUPLICATE, PLAIN_DEFINITION, [110.0, 0.0, 0.0]),
            occurrence(JOINTED, PLAIN_DEFINITION, [0.0, 0.0, 15.0]),
            CanonicalCommand::CreateDefinition {
                id: CUT_DEFINITION,
                name: "through-cut exact box".to_owned(),
            },
            profile(CUT_PROFILE, CUT_DEFINITION, [0.0, 0.0, 100.0, 60.0]),
            extrusion(CUT_EXTRUSION, CUT_DEFINITION, CUT_PROFILE),
            profile(OPENING_PROFILE, CUT_DEFINITION, [30.0, 20.0, 50.0, 35.0]),
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: CUT_DEFINITION,
                name: "opening".to_owned(),
                kind: FeatureKind::ThroughCut {
                    target: CUT_EXTRUSION,
                    profile: OPENING_PROFILE,
                },
            },
            occurrence(CUT_LEFT, CUT_DEFINITION, [250.0, 0.0, 0.0]),
            occurrence(CUT_RIGHT, CUT_DEFINITION, [360.0, 0.0, 0.0]),
            CanonicalCommand::UpsertJoint(joint),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn profile(id: FeatureId, definition_id: DefinitionId, bounds: [f64; 4]) -> CanonicalCommand {
    let [x0, y0, x1, y1] = bounds;
    CanonicalCommand::CreateFeature {
        id,
        definition_id,
        name: format!("profile-{}", id.0),
        kind: FeatureKind::Profile {
            points_mm: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        },
    }
}

fn extrusion(id: FeatureId, definition_id: DefinitionId, profile: FeatureId) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id,
        definition_id,
        name: format!("extrusion-{}", id.0),
        kind: FeatureKind::Extrusion {
            profile,
            height: Dimension::new("20", 20.0).unwrap(),
        },
    }
}

fn occurrence(
    id: OccurrenceId,
    definition_id: DefinitionId,
    translation: [f64; 3],
) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id,
        definition_id,
        name: format!("occurrence-{}", id.0),
        transform: Transform::from_translation(translation[0], translation[1], translation[2])
            .unwrap(),
        parent: None,
        tag: None,
        visible: true,
    }
}

fn package(
    snapshot: &ketchup_core::document::Snapshot,
    definition_id: DefinitionId,
) -> ketchup_core::exact_product::ExactRenderPackage {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, definition_id).unwrap();
    let roles = if request.boolean.is_some() {
        vec![
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
        ]
    } else {
        vec![
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ]
    };
    let evidence = roles.into_iter().map(|role| {
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
    if request.boolean.is_some() {
        build_box_render_package::<7>(
            &request,
            "exact-input-cut".to_owned(),
            "result-cut".to_owned(),
            "backend-v1".to_owned(),
            "tolerance-v1".to_owned(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            evidence.collect::<Vec<_>>().try_into().unwrap(),
        )
        .unwrap()
    } else {
        build_box_render_package::<3>(
            &request,
            "exact-input-plain".to_owned(),
            "result-plain".to_owned(),
            "backend-v1".to_owned(),
            "tolerance-v1".to_owned(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            evidence.collect::<Vec<_>>().try_into().unwrap(),
        )
        .unwrap()
    }
}

fn participant(
    snapshot: &ketchup_core::document::Snapshot,
    key: &str,
    occurrence: OccurrenceId,
    package: &ketchup_core::exact_product::ExactRenderPackage,
    tolerance: TolerancePolicy,
) -> ExactBodyParticipant {
    ExactBodyParticipant::accept(
        snapshot,
        identity(key),
        InstancePath::root(occurrence),
        package,
        tolerance,
    )
    .unwrap()
}

fn segment(key: &str) -> SlotSegment {
    SlotSegment::new(RULE_NODE, OUTPUT_PORT, key).unwrap()
}

fn identity(key: &str) -> DerivedIdentity {
    DerivedIdentity::new(RULE_NODE, SlotPath::new(vec![segment(key)]).unwrap()).unwrap()
}
