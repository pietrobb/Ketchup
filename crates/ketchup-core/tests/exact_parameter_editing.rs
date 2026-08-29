use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, ProfileSegment, ProposalContext, ProposalPrincipal,
};
use ketchup_core::exact_product::{
    ExactFaceRole, ExactFeatureChainRequest, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::feature_history::{
    BodyParameterEditError, BodyParameterEditRequest, BodyProfileTranslationRequest,
    ExactParameterEdit, ExactParameterEditTarget, prepare_body_parameter_edit,
    prepare_body_profile_translation,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use std::collections::BTreeSet;

const DEFINITION: DefinitionId = DefinitionId(1);
const PRINCIPAL: FeatureId = FeatureId(10);
const OFFSET: FeatureId = FeatureId(11);
const SKETCH: FeatureId = FeatureId(12);
const PAD: FeatureId = FeatureId(13);
const CUT_SKETCH: FeatureId = FeatureId(14);
const CUT: FeatureId = FeatureId(15);
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
    assert_eq!(pad.extent.blind_distance().unwrap().millimetres(), 9.0);

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
}

fn seed_movable_circular_pocket() -> DocumentStore {
    let base_corners = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]];
    let base_entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: base_corners[index],
            end_mm: base_corners[(index + 1) % 4],
        })
        .collect::<Vec<_>>();
    let mut base_constraints = Vec::new();
    for index in 0..4 {
        for (offset, point_kind, position_mm) in [
            (0, SketchPointKind::Start, base_corners[index]),
            (1, SketchPointKind::End, base_corners[(index + 1) % 4]),
        ] {
            base_constraints.push(SketchConstraint {
                id: SketchConstraintId(index as u64 * 2 + offset + 1),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(index as u64 + 1),
                        point: point_kind,
                    },
                    position_mm,
                },
            });
        }
    }
    let base_sketch = SketchSpec {
        workplane: PRINCIPAL,
        entities: base_entities,
        constraints: base_constraints,
    };
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Movable hole part".to_owned(),
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
                name: "Base rectangle".to_owned(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: PAD,
                definition_id: DEFINITION,
                name: "Base pad".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: OFFSET,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("10").unwrap()),
                }),
            },
        ]))
        .unwrap();
    let base = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&base, DEFINITION).unwrap();
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
                role.expected_type(),
            ),
            format!("geometry.{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "exact-input".to_owned(),
        "base-result".to_owned(),
        "test-backend".to_owned(),
        "test-tolerance".to_owned(),
        request.expected_bounds_mm(),
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let cut_sketch = SketchSpec {
        workplane: FeatureId(16),
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [12.0, 14.0],
            radius_mm: 2.5,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("2.5").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [12.0, 14.0],
                },
            },
        ],
    };
    let cut_region = cut_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(16),
                definition_id: DEFINITION,
                name: "Top face".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [0.0, 0.0, 10.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: CUT_SKETCH,
                definition_id: DEFINITION,
                name: "Hole position".to_owned(),
                kind: FeatureKind::Sketch(cut_sketch),
            },
        ]))
        .unwrap();
    let proposal = document
        .plan_pad_pocket(
            CUT,
            DEFINITION,
            "Circular cut",
            PadPocketOperation::Pocket(PocketSpec {
                target: PAD,
                sketch: CUT_SKETCH,
                region: cut_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("8").unwrap()),
            }),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    document.commit_proposal(&proposal).unwrap();
    document.discard_history_before_current();
    document
}

fn circle_segments(center: [f64; 2], radius: f64) -> Vec<ProfileSegment> {
    let east = [center[0] + radius, center[1]];
    let west = [center[0] - radius, center[1]];
    vec![
        ProfileSegment::CircularArc {
            start_mm: east,
            end_mm: west,
            center_mm: center,
            clockwise: false,
        },
        ProfileSegment::CircularArc {
            start_mm: west,
            end_mm: east,
            center_mm: center,
            clockwise: false,
        },
    ]
}

fn rounded_slot_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [10.0, 10.0],
            end_mm: [20.0, 10.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [20.0, 10.0],
            end_mm: [20.0, 16.0],
            center_mm: [20.0, 13.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [20.0, 16.0],
            end_mm: [10.0, 16.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [10.0, 16.0],
            end_mm: [10.0, 10.0],
            center_mm: [10.0, 13.0],
            clockwise: false,
        },
    ]
}

fn seed_movable_rounded_slot_pocket() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Movable fitting pocket".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Board outline".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PAD,
                definition_id: DEFINITION,
                name: "Board".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: OFFSET,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT_SKETCH,
                definition_id: DEFINITION,
                name: "Rounded fitting slot".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: rounded_slot_segments(),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT,
                definition_id: DEFINITION,
                name: "6 mm fitting pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: PAD,
                    profile: CUT_SKETCH,
                    depth: Dimension::from_decimal("6").unwrap(),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn seed_movable_circular_through_cut() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Movable through hole part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Base rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PAD,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: OFFSET,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT_SKETCH,
                definition_id: DEFINITION,
                name: "Circular cut profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circle_segments([12.0, 14.0], 2.5),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(16),
                definition_id: DEFINITION,
                name: "Cutting cylinder".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: CUT_SKETCH,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT,
                definition_id: DEFINITION,
                name: "Through hole".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target: PAD,
                    tool: FeatureId(16),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

#[test]
fn circular_pocket_profile_moves_without_changing_radius_and_is_one_undo_step() {
    let mut document = seed_movable_circular_pocket();
    let before = stamp(&document);
    let request = BodyProfileTranslationRequest {
        definition_id: DEFINITION,
        body_id: BodyId(1),
        profile_id: CUT_SKETCH,
        delta_mm: [7.0, -3.0],
    };
    let manual = prepare_body_profile_translation(
        &document,
        request.clone(),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let assistant =
        prepare_body_profile_translation(&document, request, ProposalPrincipal::LocalAssistant)
            .unwrap();

    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(manual.affected_feature_ids, vec![CUT_SKETCH, CUT]);
    let preview = document.preview_batch(manual.proposal.batch()).unwrap();
    assert_eq!(stamp(&document), before);
    let FeatureKind::Sketch(preview_sketch) = preview.feature(CUT_SKETCH).unwrap().kind() else {
        panic!("expected translated cut sketch")
    };
    let SketchEntity::Circle {
        center_mm,
        radius_mm,
        ..
    } = &preview_sketch.entities[0]
    else {
        panic!("expected circular cut profile")
    };
    assert_eq!(*center_mm, [19.0, 11.0]);
    assert_eq!(*radius_mm, 2.5);

    document.commit_proposal(&manual.proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    let committed_snapshot = document.current();
    let moved_digest = committed_snapshot.canonical_digest();
    let FeatureKind::Sketch(committed) = committed_snapshot.feature(CUT_SKETCH).unwrap().kind()
    else {
        panic!("expected committed cut sketch")
    };
    assert!(matches!(
        &committed.constraints[1].kind,
        SketchConstraintKind::FixedPoint { position_mm, .. }
            if *position_mm == [19.0, 11.0]
    ));
    assert!(matches!(
        committed_snapshot.feature(CUT).unwrap().kind(),
        FeatureKind::SketchPocket(spec)
            if spec.target == PAD
                && spec.sketch == CUT_SKETCH
                && spec.extent.blind_distance().unwrap().millimetres() == 8.0
    ));

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), moved_digest);
}

#[test]
fn rounded_fitting_pocket_moves_without_changing_shape_or_depth() {
    let mut document = seed_movable_rounded_slot_pocket();
    let before = stamp(&document);
    let original_segments = rounded_slot_segments();
    let preview = prepare_body_profile_translation(
        &document,
        BodyProfileTranslationRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            profile_id: CUT_SKETCH,
            delta_mm: [2.0, 3.0],
        },
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    assert_eq!(preview.affected_feature_ids, vec![CUT_SKETCH, CUT]);
    assert_eq!(stamp(&document), before);
    document.commit_proposal(&preview.proposal).unwrap();
    let moved = document.current();
    let FeatureKind::SegmentProfile { segments, closed } =
        moved.feature(CUT_SKETCH).unwrap().kind()
    else {
        panic!("expected moved rounded fitting slot")
    };
    assert!(*closed);
    assert_eq!(segments.len(), original_segments.len());
    for (moved, original) in segments.iter().zip(&original_segments) {
        match (moved, original) {
            (
                ProfileSegment::Line {
                    start_mm: moved_start,
                    end_mm: moved_end,
                },
                ProfileSegment::Line { start_mm, end_mm },
            ) => {
                assert_eq!(*moved_start, [start_mm[0] + 2.0, start_mm[1] + 3.0]);
                assert_eq!(*moved_end, [end_mm[0] + 2.0, end_mm[1] + 3.0]);
            }
            (
                ProfileSegment::CircularArc {
                    start_mm: moved_start,
                    end_mm: moved_end,
                    center_mm: moved_center,
                    clockwise: moved_clockwise,
                },
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                },
            ) => {
                assert_eq!(*moved_start, [start_mm[0] + 2.0, start_mm[1] + 3.0]);
                assert_eq!(*moved_end, [end_mm[0] + 2.0, end_mm[1] + 3.0]);
                assert_eq!(*moved_center, [center_mm[0] + 2.0, center_mm[1] + 3.0]);
                assert_eq!(moved_clockwise, clockwise);
            }
            _ => panic!("rounded slot segment kind changed"),
        }
    }
    assert!(matches!(
        moved.feature(CUT).unwrap().kind(),
        FeatureKind::Pocket { target, profile, depth }
            if *target == PAD
                && *profile == CUT_SKETCH
                && depth.millimetres() == 6.0
    ));
    ExactFeatureChainRequest::from_snapshot_for_body(&moved, DEFINITION, BodyId(1)).unwrap();

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
}

#[test]
fn circular_through_cut_profile_moves_and_remains_exact() {
    let mut document = seed_movable_circular_through_cut();
    let before = stamp(&document);
    let preview = prepare_body_profile_translation(
        &document,
        BodyProfileTranslationRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            profile_id: CUT_SKETCH,
            delta_mm: [7.0, -3.0],
        },
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(
        preview.affected_feature_ids,
        vec![CUT_SKETCH, CUT, FeatureId(16)]
    );
    document.commit_proposal(&preview.proposal).unwrap();
    let moved = document.current();
    let FeatureKind::SegmentProfile { segments, closed } =
        moved.feature(CUT_SKETCH).unwrap().kind()
    else {
        panic!("expected moved circular profile")
    };
    assert!(*closed);
    assert!(segments.iter().all(|segment| matches!(
        segment,
        ProfileSegment::CircularArc { center_mm, .. } if *center_mm == [19.0, 11.0]
    )));
    ExactFeatureChainRequest::from_snapshot_for_body(&moved, DEFINITION, BodyId(1)).unwrap();
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
}

#[test]
fn circular_cut_profile_move_outside_host_is_refused_without_mutation() {
    let document = seed_movable_circular_through_cut();
    let before = stamp(&document);
    assert_eq!(
        prepare_body_profile_translation(
            &document,
            BodyProfileTranslationRequest {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                profile_id: CUT_SKETCH,
                delta_mm: [100.0, 0.0],
            },
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyParameterEditError::InvalidCutPosition)
    );
    assert_eq!(stamp(&document), before);
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
