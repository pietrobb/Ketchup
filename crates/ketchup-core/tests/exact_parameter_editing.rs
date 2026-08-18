use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, ProposalPrincipal,
};
use ketchup_core::feature_history::{
    BodyParameterEditError, BodyParameterEditRequest, ExactParameterEdit, ExactParameterEditTarget,
    prepare_body_parameter_edit,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchSpec, WorkplaneSpec, WorkplaneSupport,
};
use std::collections::BTreeSet;

const DEFINITION: DefinitionId = DefinitionId(1);
const PRINCIPAL: FeatureId = FeatureId(10);
const OFFSET: FeatureId = FeatureId(11);
const SKETCH: FeatureId = FeatureId(12);
const PAD: FeatureId = FeatureId(13);
const RADIUS: SketchConstraintId = SketchConstraintId(1);

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn seed_body_parameter_edit() -> DocumentStore {
    let mut document = DocumentStore::new();
    let sketch = SketchSpec {
        workplane: OFFSET,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [4.0, 5.0],
            radius_mm: 3.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: RADIUS,
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("3").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [4.0, 5.0],
                },
            },
        ],
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Editable part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PRINCIPAL,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Offset".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset {
                        base: PRINCIPAL,
                        distance: Dimension::from_decimal("2").unwrap(),
                    },
                    frame: WorkplaneSpec::principal(PrincipalPlane::Xy)
                        .frame
                        .offset(2.0),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Circle".to_owned(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: PAD,
                definition_id: DEFINITION,
                name: "Pad".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: SKETCH,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("5").unwrap()),
                }),
            },
        ]))
        .unwrap();
    document
}

fn body_parameter_request() -> BodyParameterEditRequest {
    BodyParameterEditRequest {
        definition_id: DEFINITION,
        body_id: BodyId(1),
        edits: vec![
            ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(OFFSET),
                dimension: Dimension::from_decimal("4").unwrap(),
            },
            ExactParameterEdit {
                target: ExactParameterEditTarget::SketchConstraintDimension {
                    sketch_id: SKETCH,
                    constraint_id: RADIUS,
                },
                dimension: Dimension::from_decimal("6").unwrap(),
            },
            ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(PAD),
                dimension: Dimension::from_decimal("9").unwrap(),
            },
        ],
    }
}

#[test]
fn exact_body_parameter_preview_has_manual_ai_parity_and_one_undo() {
    let mut document = seed_body_parameter_edit();
    let before = stamp(&document);
    let manual = prepare_body_parameter_edit(
        &document,
        body_parameter_request(),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let assistant = prepare_body_parameter_edit(
        &document,
        body_parameter_request(),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(
        manual.proposal.command_digest(),
        assistant.proposal.command_digest()
    );
    assert_eq!(
        manual.proposal.intended_result_digest(),
        assistant.proposal.intended_result_digest()
    );
    assert_eq!(manual.affected_feature_ids, vec![OFFSET, SKETCH, PAD]);
    assert!(manual.unchanged_body_ids.is_empty());
    assert_eq!(stamp(&document), before);
    let preview = document.preview_batch(manual.proposal.batch()).unwrap();
    assert_ne!(preview.canonical_digest(), before.1);
    assert_eq!(stamp(&document), before);

    let revision = document.commit_proposal(&manual.proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    assert_eq!(
        revision.dirty_features(),
        &BTreeSet::from([OFFSET, SKETCH, PAD])
    );
    let snapshot = document.current();
    let FeatureKind::Workplane(offset) = snapshot.feature(OFFSET).unwrap().kind() else {
        panic!("expected offset workplane");
    };
    let WorkplaneSupport::Offset { distance, .. } = &offset.support else {
        panic!("expected offset support");
    };
    assert_eq!(distance.millimetres(), 4.0);
    let FeatureKind::Sketch(sketch) = snapshot.feature(SKETCH).unwrap().kind() else {
        panic!("expected sketch");
    };
    let SketchConstraintKind::Radius { value, .. } = &sketch.constraints[0].kind else {
        panic!("expected radius constraint");
    };
    assert_eq!(value.millimetres(), 6.0);
    let FeatureKind::Pad(pad) = snapshot.feature(PAD).unwrap().kind() else {
        panic!("expected Pad");
    };
    assert_eq!(pad.extent.distance().millimetres(), 9.0);

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
}

#[test]
fn duplicate_parameter_edits_fail_without_mutation() {
    let document = seed_body_parameter_edit();
    let before = stamp(&document);
    let duplicate = ExactParameterEdit {
        target: ExactParameterEditTarget::FeatureDimension(PAD),
        dimension: Dimension::from_decimal("7").unwrap(),
    };
    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                edits: vec![duplicate.clone(), duplicate],
            },
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyParameterEditError::Duplicate(
            ExactParameterEditTarget::FeatureDimension(PAD)
        ))
    );
    assert_eq!(stamp(&document), before);
}

const BASE_PROFILE: FeatureId = FeatureId(20);
const BASE_EXTRUSION: FeatureId = FeatureId(21);
const TOOL_PROFILE: FeatureId = FeatureId(30);
const TOOL_EXTRUSION: FeatureId = FeatureId(31);
const UNION: FeatureId = FeatureId(40);

fn profile() -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
    }
}

fn seed_cross_body_history() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Two body part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".to_owned(),
                kind: profile(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Tool body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_PROFILE,
                definition_id: DEFINITION,
                name: "Tool profile".to_owned(),
                kind: profile(),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
            CanonicalCommand::CreateFeature {
                id: UNION,
                definition_id: DEFINITION,
                name: "Union".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: BASE_EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    document
}

#[test]
fn cross_body_affected_closure_fails_without_mutation() {
    let document = seed_cross_body_history();
    let before = stamp(&document);
    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: DEFINITION,
                body_id: BodyId(2),
                edits: vec![ExactParameterEdit {
                    target: ExactParameterEditTarget::FeatureDimension(TOOL_EXTRUSION),
                    dimension: Dimension::from_decimal("4").unwrap(),
                }],
            },
            ProposalPrincipal::LocalAssistant,
        ),
        Err(BodyParameterEditError::CrossBodyAffected(UNION, BodyId(1)))
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn body_edit_recomputes_its_branch_and_preserves_unrelated_body_identity() {
    let mut document = seed_cross_body_history();
    let before = document.current();
    let unrelated_body = before
        .definition(DEFINITION)
        .unwrap()
        .body(BodyId(2))
        .unwrap()
        .clone();
    let unrelated_profile = before.feature(TOOL_PROFILE).unwrap().clone();
    let unrelated_extrusion = before.feature(TOOL_EXTRUSION).unwrap().clone();
    let preview = prepare_body_parameter_edit(
        &document,
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(BASE_EXTRUSION),
                dimension: Dimension::from_decimal("8").unwrap(),
            }],
        },
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    assert_eq!(preview.affected_feature_ids, vec![BASE_EXTRUSION, UNION]);
    assert_eq!(preview.unchanged_body_ids, vec![BodyId(2)]);
    let revision = document.commit_proposal(&preview.proposal).unwrap();
    assert_eq!(
        revision.dirty_features(),
        &BTreeSet::from([BASE_EXTRUSION, UNION])
    );
    let after = document.current();
    assert_eq!(
        after.definition(DEFINITION).unwrap().body(BodyId(2)),
        Some(&unrelated_body)
    );
    assert_eq!(after.feature(TOOL_PROFILE), Some(&unrelated_profile));
    assert_eq!(after.feature(TOOL_EXTRUSION), Some(&unrelated_extrusion));
}
