use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId,
    Dimension, DocumentStore, FeatureId, FeatureKind, ProposalCommitError, ProposalPrepareError,
    ProposalPrincipal,
};
use ketchup_core::feature_history::{
    BodyParameterEditError, BodyParameterEditRequest, ExactParameterEdit, ExactParameterEditTarget,
    prepare_body_parameter_edit,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchError, SketchPointKind,
    SketchPointRef, SketchSpec, WorkplaneSpec,
};
use std::collections::BTreeSet;

const PART: DefinitionId = DefinitionId(1);
const OTHER_DEFINITION: DefinitionId = DefinitionId(2);
const BASE_PROFILE: FeatureId = FeatureId(10);
const BASE_EXTRUSION: FeatureId = FeatureId(11);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);
const OTHER_PROFILE: FeatureId = FeatureId(30);
const OTHER_EXTRUSION: FeatureId = FeatureId(31);
const UNION: FeatureId = FeatureId(40);

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn profile() -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 4.0], [0.0, 4.0]],
    }
}

fn seed(include_union: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: PART,
            name: "Part".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: BASE_PROFILE,
            definition_id: PART,
            name: "Base profile".to_owned(),
            kind: profile(),
        },
        CanonicalCommand::CreateFeature {
            id: BASE_EXTRUSION,
            definition_id: PART,
            name: "Base extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: BASE_PROFILE,
                height: Dimension::from_decimal("5").unwrap(),
            },
        },
        CanonicalCommand::CreateBody {
            definition_id: PART,
            id: BodyId(2),
            name: "Tool body".to_owned(),
            visible: true,
        },
        CanonicalCommand::SetActiveBody {
            definition_id: PART,
            id: BodyId(2),
        },
        CanonicalCommand::CreateFeature {
            id: TOOL_PROFILE,
            definition_id: PART,
            name: "Tool profile".to_owned(),
            kind: profile(),
        },
        CanonicalCommand::CreateFeature {
            id: TOOL_EXTRUSION,
            definition_id: PART,
            name: "Tool extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: TOOL_PROFILE,
                height: Dimension::from_decimal("2").unwrap(),
            },
        },
        CanonicalCommand::SetActiveBody {
            definition_id: PART,
            id: BodyId(1),
        },
    ];
    if include_union {
        commands.push(CanonicalCommand::CreateFeature {
            id: UNION,
            definition_id: PART,
            name: "Union".to_owned(),
            kind: FeatureKind::Boolean {
                operation: BooleanOperation::Union,
                target: BASE_EXTRUSION,
                tool: TOOL_EXTRUSION,
            },
        });
    }
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: OTHER_DEFINITION,
            name: "Other".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: OTHER_PROFILE,
            definition_id: OTHER_DEFINITION,
            name: "Other profile".to_owned(),
            kind: profile(),
        },
        CanonicalCommand::CreateFeature {
            id: OTHER_EXTRUSION,
            definition_id: OTHER_DEFINITION,
            name: "Other extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: OTHER_PROFILE,
                height: Dimension::from_decimal("3").unwrap(),
            },
        },
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn edit(target: FeatureId, value: &str) -> ExactParameterEdit {
    ExactParameterEdit {
        target: ExactParameterEditTarget::FeatureDimension(target),
        dimension: Dimension::from_decimal(value).unwrap(),
    }
}

#[test]
fn independent_preview_commit_undo_redo_and_save_open_are_body_stable() {
    let mut document = seed(false);
    let before = stamp(&document);
    let unrelated_body = document
        .current()
        .definition(PART)
        .unwrap()
        .body(BodyId(2))
        .unwrap()
        .clone();
    let unrelated_feature = document.current().feature(TOOL_EXTRUSION).unwrap().clone();
    let request = || BodyParameterEditRequest {
        definition_id: PART,
        body_id: BodyId(1),
        edits: vec![edit(BASE_EXTRUSION, "12")],
    };
    let manual =
        prepare_body_parameter_edit(&document, request(), ProposalPrincipal::ManualClient).unwrap();
    let assistant =
        prepare_body_parameter_edit(&document, request(), ProposalPrincipal::LocalAssistant)
            .unwrap();

    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(
        manual.proposal.authoritative_diff(),
        assistant.proposal.authoritative_diff()
    );
    assert_eq!(manual.affected_feature_ids, vec![BASE_EXTRUSION]);
    assert_eq!(manual.unchanged_body_ids, vec![BodyId(2)]);
    assert_eq!(stamp(&document), before);
    drop(assistant);
    assert_eq!(stamp(&document), before);

    let revision = document.commit_proposal(&manual.proposal).unwrap();
    assert_eq!(revision.dirty_features(), &BTreeSet::from([BASE_EXTRUSION]));
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_eq!(
        committed.definition(PART).unwrap().body(BodyId(2)),
        Some(&unrelated_body)
    );
    assert_eq!(committed.feature(TOOL_EXTRUSION), Some(&unrelated_feature));
    assert_eq!(
        committed.definition(PART).unwrap().active_body_id(),
        BodyId(1)
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before.1);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened
            .snapshot()
            .definition(PART)
            .unwrap()
            .body(BodyId(2)),
        Some(&unrelated_body)
    );
    let reopened_snapshot = reopened.snapshot();
    let FeatureKind::Extrusion { height, .. } =
        reopened_snapshot.feature(BASE_EXTRUSION).unwrap().kind()
    else {
        panic!("expected persisted extrusion");
    };
    assert_eq!(height.millimetres(), 12.0);
}

#[test]
fn stale_duplicate_cross_definition_and_unsupported_targets_are_atomic() {
    let mut document = seed(false);
    let stale = prepare_body_parameter_edit(
        &document,
        BodyParameterEditRequest {
            definition_id: PART,
            body_id: BodyId(1),
            edits: vec![edit(BASE_EXTRUSION, "8")],
        },
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BASE_EXTRUSION,
                dimension: Dimension::from_decimal("7").unwrap(),
            },
        ]))
        .unwrap();
    let before_stale = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale.proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale);

    let duplicate = edit(BASE_EXTRUSION, "9");
    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(1),
                edits: vec![duplicate.clone(), duplicate],
            },
            ProposalPrincipal::LocalAssistant,
        ),
        Err(BodyParameterEditError::Duplicate(
            ExactParameterEditTarget::FeatureDimension(BASE_EXTRUSION)
        ))
    );
    assert_eq!(stamp(&document), before_stale);

    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(1),
                edits: vec![edit(OTHER_EXTRUSION, "9")],
            },
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyParameterEditError::FeatureOutsideDefinition(
            OTHER_EXTRUSION,
            PART
        ))
    );
    assert_eq!(stamp(&document), before_stale);

    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(1),
                edits: vec![edit(BASE_PROFILE, "9")],
            },
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyParameterEditError::UnsupportedTarget(
            ExactParameterEditTarget::FeatureDimension(BASE_PROFILE)
        ))
    );
    assert_eq!(stamp(&document), before_stale);
}

#[test]
fn cross_body_dependent_rejection_preserves_last_valid_outputs() {
    let document = seed(true);
    let before = stamp(&document);
    let base_before = document.current().feature(BASE_EXTRUSION).unwrap().clone();
    let tool_before = document.current().feature(TOOL_EXTRUSION).unwrap().clone();
    let union_before = document.current().feature(UNION).unwrap().clone();

    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(2),
                edits: vec![edit(TOOL_EXTRUSION, "15")],
            },
            ProposalPrincipal::LocalAssistant,
        ),
        Err(BodyParameterEditError::CrossBodyAffected(UNION, BodyId(1)))
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(
        document.current().feature(BASE_EXTRUSION),
        Some(&base_before)
    );
    assert_eq!(
        document.current().feature(TOOL_EXTRUSION),
        Some(&tool_before)
    );
    assert_eq!(document.current().feature(UNION), Some(&union_before));
}

const PAD_PLANE: FeatureId = FeatureId(50);
const PAD_SKETCH: FeatureId = FeatureId(51);
const PAD_FEATURE: FeatureId = FeatureId(52);

fn seed_pad() -> DocumentStore {
    let mut document = DocumentStore::new();
    let sketch = SketchSpec {
        workplane: PAD_PLANE,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [2.0, 2.0],
            radius_mm: 2.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("2").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [2.0, 2.0],
                },
            },
        ],
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: PART,
                name: "Pad part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PAD_PLANE,
                definition_id: PART,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: PAD_SKETCH,
                definition_id: PART,
                name: "Circle".to_owned(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: PAD_FEATURE,
                definition_id: PART,
                name: "Pad".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: PAD_SKETCH,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("4").unwrap()),
                }),
            },
        ]))
        .unwrap();
    document
}

#[test]
fn invalid_pad_and_sketch_values_and_non_dimension_constraints_are_atomic() {
    let document = seed_pad();
    let before = stamp(&document);
    for target in [
        ExactParameterEditTarget::FeatureDimension(PAD_FEATURE),
        ExactParameterEditTarget::SketchConstraintDimension {
            sketch_id: PAD_SKETCH,
            constraint_id: SketchConstraintId(1),
        },
    ] {
        let result = prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(1),
                edits: vec![ExactParameterEdit {
                    target,
                    dimension: Dimension::from_decimal("-1").unwrap(),
                }],
            },
            ProposalPrincipal::ManualClient,
        );
        assert!(matches!(
            result,
            Err(BodyParameterEditError::Proposal(
                ProposalPrepareError::Canonical(CanonicalError::Sketch(
                    SketchError::InvalidDimension
                ))
            ))
        ));
        assert_eq!(stamp(&document), before);
    }

    assert_eq!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: PART,
                body_id: BodyId(1),
                edits: vec![ExactParameterEdit {
                    target: ExactParameterEditTarget::SketchConstraintDimension {
                        sketch_id: PAD_SKETCH,
                        constraint_id: SketchConstraintId(2),
                    },
                    dimension: Dimension::from_decimal("7").unwrap(),
                }],
            },
            ProposalPrincipal::LocalAssistant,
        ),
        Err(BodyParameterEditError::UnsupportedTarget(
            ExactParameterEditTarget::SketchConstraintDimension {
                sketch_id: PAD_SKETCH,
                constraint_id: SketchConstraintId(2),
            }
        ))
    );
    assert_eq!(stamp(&document), before);
}
