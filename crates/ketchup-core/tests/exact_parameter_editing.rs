use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, ChamferEdgeSide, ChamferMode, CommandBatch,
    DefinitionId, Dimension, DocumentStore, EdgeFinishKind, FeatureId, FeatureKind,
    FeatureParameterTarget, FilletRadiusStation, LoftContinuity, LoftSection, ParameterValueType,
    ProfileSegment, ProposalContext, ProposalPrincipal, SpatialPathSegment,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactFaceRole, ExactFeatureChainRequest, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::feature_history::{
    BodyParameterEditError, BodyParameterEditRequest, BodyProfileTranslationRequest,
    ExactParameterEdit, ExactParameterEditTarget, prepare_body_parameter_edit,
    prepare_body_profile_translation,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};
use std::collections::BTreeSet;

const DEFINITION: DefinitionId = DefinitionId(1);
const PRINCIPAL: FeatureId = FeatureId(10);
const OFFSET: FeatureId = FeatureId(11);
const SKETCH: FeatureId = FeatureId(12);
const PAD: FeatureId = FeatureId(13);
const CUT_SKETCH: FeatureId = FeatureId(14);
const CUT: FeatureId = FeatureId(15);
const CHILD_OFFSET: FeatureId = FeatureId(16);
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

#[test]
fn offset_workplane_edits_recompute_the_full_descendant_frame_chain() {
    let mut document = seed_body_parameter_edit();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: CHILD_OFFSET,
            definition_id: DEFINITION,
            name: "Child offset".to_owned(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: OFFSET,
                    distance: Dimension::from_decimal("3").unwrap(),
                },
                frame: WorkplaneSpec::principal(PrincipalPlane::Xy)
                    .frame
                    .offset(5.0),
            }),
        }]))
        .unwrap();
    let before = stamp(&document);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: FeatureParameterTarget::new(
                    OFFSET,
                    "support.offset.distance",
                    ParameterValueType::Length,
                )
                .unwrap(),
                dimension: Dimension::from_decimal("8").unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(document.current().revision_id(), before.0 + 1);
    let snapshot = document.current();
    for (feature_id, expected_z) in [(OFFSET, 8.0), (CHILD_OFFSET, 11.0)] {
        let FeatureKind::Workplane(workplane) = snapshot.feature(feature_id).unwrap().kind() else {
            panic!("expected offset workplane");
        };
        assert_eq!(workplane.frame.origin_mm, [0.0, 0.0, expected_z]);
    }

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::from_decimal("5").unwrap(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    for (feature_id, expected_z) in [(OFFSET, 5.0), (CHILD_OFFSET, 8.0)] {
        let FeatureKind::Workplane(workplane) = snapshot.feature(feature_id).unwrap().kind() else {
            panic!("expected offset workplane");
        };
        assert_eq!(workplane.frame.origin_mm, [0.0, 0.0, expected_z]);
    }

    document.undo().unwrap();
    assert_eq!(document.current().revision_id(), before.0 + 1);
    document.undo().unwrap();
    let restored = stamp(&document);
    assert_eq!(restored.0, before.0);
    assert_eq!(restored.1, before.1);
    assert_eq!(restored.2, before.2);
    assert_eq!(restored.3, 2);
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[6.0, 0.0], [12.0, 0.0], [12.0, 8.0], [6.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: Dimension::from_decimal("5").unwrap(),
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
fn cross_body_affected_closure_previews_and_commits_atomically() {
    let mut document = seed_cross_body_history();
    let before = stamp(&document);
    let before_exact = ExactFeatureChainRequest::from_snapshot_for_body(
        &document.current(),
        DEFINITION,
        BodyId(1),
    )
    .unwrap();
    let preview = prepare_body_parameter_edit(
        &document,
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(2),
            edits: vec![generic_parameter(TOOL_PROFILE, "bounds.width", 7.0)],
        },
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(preview.affected_body_ids, vec![BodyId(1), BodyId(2)]);
    assert_eq!(
        preview.affected_feature_ids,
        vec![TOOL_PROFILE, TOOL_EXTRUSION, UNION]
    );
    assert!(preview.unchanged_body_ids.is_empty());
    assert_eq!(stamp(&document), before);
    let candidate = document.preview_batch(preview.proposal.batch()).unwrap();
    let candidate_exact =
        ExactFeatureChainRequest::from_snapshot_for_body(&candidate, DEFINITION, BodyId(1))
            .unwrap();
    assert_ne!(
        candidate_exact.canonical_input_digest,
        before_exact.canonical_input_digest
    );
    assert_eq!(stamp(&document), before);

    let revision = document.commit_proposal(&preview.proposal).unwrap();
    assert_eq!(
        revision.dirty_features(),
        &BTreeSet::from([TOOL_PROFILE, TOOL_EXTRUSION, UNION])
    );
    let edited_digest = document.current().canonical_digest();
    assert_eq!(document.undo().unwrap().canonical_digest(), before.1);
    assert_eq!(document.redo().unwrap().canonical_digest(), edited_digest);
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), edited_digest);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_body(
            &reopened.snapshot(),
            DEFINITION,
            BodyId(1)
        )
        .unwrap()
        .canonical_input_digest,
        candidate_exact.canonical_input_digest
    );
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

fn topology_parameter_reference(
    snapshot: &ketchup_core::document::Snapshot,
    producer: FeatureId,
    kind: TopologicalElementKind,
) -> TopologicalElementRef {
    TopologicalElementRef::new(
        snapshot.document_id(),
        DEFINITION,
        producer,
        producer,
        kind,
        format!("feature/{}/{}", producer.0, kind.token()),
        format!("result/{}/{}", producer.0, kind.token()),
        TopologicalReferenceStability::Guaranteed,
        "ketchup.exact-brep-graph-evaluator.v1",
        "occt.v1",
        "1e-7-mm",
        format!("result-{}", producer.0),
        format!("geometry-{}", producer.0),
    )
    .unwrap()
}

fn generic_parameter(feature_id: FeatureId, path: &str, value: f64) -> ExactParameterEdit {
    ExactParameterEdit {
        target: ExactParameterEditTarget::FeatureParameter(
            FeatureParameterTarget::new(feature_id, path, ParameterValueType::Length).unwrap(),
        ),
        dimension: Dimension::new(value.to_string(), value).unwrap(),
    }
}

#[test]
fn general_feature_parameters_preview_recompute_undo_and_round_trip() {
    const REVOLVE_PROFILE: FeatureId = FeatureId(101);
    const REVOLVE: FeatureId = FeatureId(102);
    const SWEEP_PROFILE: FeatureId = FeatureId(103);
    const SWEEP_PATH: FeatureId = FeatureId(104);
    const SWEEP: FeatureId = FeatureId(105);
    const LOFT_LOWER: FeatureId = FeatureId(106);
    const LOFT_UPPER: FeatureId = FeatureId(107);
    const LOFT: FeatureId = FeatureId(108);
    const FINISH_PROFILE: FeatureId = FeatureId(109);
    const FINISH_BASE: FeatureId = FeatureId(110);
    const FACE_OFFSET: FeatureId = FeatureId(111);
    const SHELL: FeatureId = FeatureId(112);
    const EDGE_FINISH: FeatureId = FeatureId(113);

    let rectangle = || FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "General editable features".into(),
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE_PROFILE,
                definition_id: DEFINITION,
                name: "Revolve profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [3.0, 0.0], [3.0, 8.0], [0.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: DEFINITION,
                name: "Revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: REVOLVE_PROFILE,
                    axis_start_mm: [0.0, 0.0],
                    axis_end_mm: [0.0, 1.0],
                    angle_degrees: 360.0,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP_PROFILE,
                definition_id: DEFINITION,
                name: "Sweep profile".into(),
                kind: rectangle(),
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP_PATH,
                definition_id: DEFINITION,
                name: "Sweep path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 12.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Sweep".into(),
                kind: FeatureKind::Sweep {
                    profile: SWEEP_PROFILE,
                    path: SWEEP_PATH,
                },
            },
            CanonicalCommand::CreateFeature {
                id: LOFT_LOWER,
                definition_id: DEFINITION,
                name: "Loft lower".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: LOFT_UPPER,
                definition_id: DEFINITION,
                name: "Loft upper".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-2.0, -1.0], [3.0, -1.0], [2.5, 2.0], [-1.5, 2.5]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: LOFT,
                definition_id: DEFINITION,
                name: "Loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: LOFT_LOWER,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: LOFT_UPPER,
                            elevation_mm: 10.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FINISH_PROFILE,
                definition_id: DEFINITION,
                name: "Finish profile".into(),
                kind: rectangle(),
            },
            CanonicalCommand::CreateFeature {
                id: FINISH_BASE,
                definition_id: DEFINITION,
                name: "Finish base".into(),
                kind: FeatureKind::Extrusion {
                    profile: FINISH_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
        ]))
        .unwrap();
    let face = topology_parameter_reference(
        &document.current(),
        FINISH_BASE,
        TopologicalElementKind::Face,
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FACE_OFFSET,
            definition_id: DEFINITION,
            name: "Face offset".into(),
            kind: FeatureKind::TopologyFaceOffset {
                target: FINISH_BASE,
                face,
                distance: Dimension::from_decimal("0.5").unwrap(),
            },
        }]))
        .unwrap();
    let shell_face = topology_parameter_reference(
        &document.current(),
        FACE_OFFSET,
        TopologicalElementKind::Face,
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: SHELL,
            definition_id: DEFINITION,
            name: "Shell".into(),
            kind: FeatureKind::TopologyShell {
                target: FACE_OFFSET,
                removed_faces: vec![shell_face],
                thickness: Dimension::from_decimal("1").unwrap(),
                direction: ketchup_core::document::ShellDirection::Inward,
            },
        }]))
        .unwrap();
    let edge =
        topology_parameter_reference(&document.current(), SHELL, TopologicalElementKind::Edge);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: EDGE_FINISH,
            definition_id: DEFINITION,
            name: "Edge finish".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: SHELL,
                edges: vec![edge],
                kind: EdgeFinishKind::Fillet,
                amount: Dimension::from_decimal("0.5").unwrap(),
                fillet_radius_stations: vec![
                    FilletRadiusStation {
                        position: 0.5,
                        radius: Dimension::from_decimal("0.75").unwrap(),
                    },
                    FilletRadiusStation {
                        position: 1.0,
                        radius: Dimension::from_decimal("0.4").unwrap(),
                    },
                ],
                chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        }]))
        .unwrap();
    document.discard_history_before_current();

    let before = stamp(&document);
    let preview = prepare_body_parameter_edit(
        &document,
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![
                ExactParameterEdit {
                    target: ExactParameterEditTarget::FeatureParameter(
                        FeatureParameterTarget::new(REVOLVE, "angle", ParameterValueType::Angle)
                            .unwrap(),
                    ),
                    dimension: Dimension::from_decimal("270").unwrap(),
                },
                generic_parameter(SWEEP_PROFILE, "bounds.width", 6.0),
                generic_parameter(LOFT, "sections.1.elevation", 20.0),
                generic_parameter(FACE_OFFSET, "distance", 1.0),
                generic_parameter(SHELL, "thickness", 1.5),
                generic_parameter(EDGE_FINISH, "amount", 0.75),
                generic_parameter(EDGE_FINISH, "fillet_radius_stations.0.radius", 1.25),
            ],
        },
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(stamp(&document), before);
    let candidate = document.preview_batch(preview.proposal.batch()).unwrap();
    assert_eq!(stamp(&document), before);
    ExactBRepGraph::from_snapshot(&candidate, DEFINITION, SWEEP).unwrap();
    ExactBRepGraph::from_snapshot(&candidate, DEFINITION, LOFT).unwrap();
    ExactBRepGraph::from_snapshot(&candidate, DEFINITION, EDGE_FINISH).unwrap();

    document.commit_proposal(&preview.proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    let edited_digest = document.current().canonical_digest();
    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), edited_digest);
    assert_eq!(persistence::save(&reopened), bytes);

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), edited_digest);

    let invalid_before = stamp(&document);
    assert!(
        prepare_body_parameter_edit(
            &document,
            BodyParameterEditRequest {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                edits: vec![generic_parameter(SHELL, "thickness", -1.0)],
            },
            ProposalPrincipal::ManualClient,
        )
        .is_err()
    );
    assert_eq!(stamp(&document), invalid_before);
}

#[test]
fn advanced_chamfer_parameters_preview_recompute_undo_and_schema_76_round_trip() {
    const PROFILE: FeatureId = FeatureId(201);
    const BASE: FeatureId = FeatureId(202);
    const TWO_DISTANCE: FeatureId = FeatureId(203);
    const DISTANCE_ANGLE: FeatureId = FeatureId(204);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Advanced chamfer parameters".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 14.0], [0.0, 14.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BASE,
                definition_id: DEFINITION,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("12").unwrap(),
                },
            },
        ]))
        .unwrap();
    let two_edge =
        topology_parameter_reference(&document.current(), BASE, TopologicalElementKind::Edge);
    let two_face =
        topology_parameter_reference(&document.current(), BASE, TopologicalElementKind::Face);
    let angle_edge = two_edge.clone();
    let angle_face = two_face.clone();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: TWO_DISTANCE,
                definition_id: DEFINITION,
                name: "Two-distance chamfer".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: BASE,
                    edges: vec![two_edge.clone()],
                    kind: EdgeFinishKind::Chamfer,
                    amount: Dimension::from_decimal("1").unwrap(),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ChamferMode::TwoDistance {
                        second_distance: Dimension::from_decimal("2").unwrap(),
                    },
                    chamfer_edge_sides: vec![ChamferEdgeSide {
                        edge: two_edge,
                        side_face: two_face,
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: DISTANCE_ANGLE,
                definition_id: DEFINITION,
                name: "Distance-angle chamfer".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: BASE,
                    edges: vec![angle_edge.clone()],
                    kind: EdgeFinishKind::Chamfer,
                    amount: Dimension::from_decimal("1").unwrap(),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ChamferMode::DistanceAngle {
                        angle_degrees: 30.0,
                    },
                    chamfer_edge_sides: vec![ChamferEdgeSide {
                        edge: angle_edge,
                        side_face: angle_face,
                    }],
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();

    let before = stamp(&document);
    let preview = prepare_body_parameter_edit(
        &document,
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![
                generic_parameter(TWO_DISTANCE, "chamfer_second_distance", 3.0),
                ExactParameterEdit {
                    target: ExactParameterEditTarget::FeatureParameter(
                        FeatureParameterTarget::new(
                            DISTANCE_ANGLE,
                            "chamfer_angle",
                            ParameterValueType::Angle,
                        )
                        .unwrap(),
                    ),
                    dimension: Dimension::from_decimal("45").unwrap(),
                },
            ],
        },
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    assert_eq!(stamp(&document), before);
    let candidate = document.preview_batch(preview.proposal.batch()).unwrap();
    assert_eq!(stamp(&document), before);
    assert!(matches!(
        candidate.feature(TWO_DISTANCE).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish {
            chamfer_mode: ChamferMode::TwoDistance { second_distance },
            ..
        } if second_distance.millimetres() == 3.0
    ));
    assert!(matches!(
        candidate.feature(DISTANCE_ANGLE).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish {
            chamfer_mode: ChamferMode::DistanceAngle { angle_degrees },
            ..
        } if *angle_degrees == 45.0
    ));
    ExactBRepGraph::from_snapshot(&candidate, DEFINITION, TWO_DISTANCE).unwrap();
    ExactBRepGraph::from_snapshot(&candidate, DEFINITION, DISTANCE_ANGLE).unwrap();

    document.commit_proposal(&preview.proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    let edited_digest = document.current().canonical_digest();
    assert_eq!(persistence::CURRENT_SCHEMA, 91);
    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), edited_digest);
    assert_eq!(persistence::save(&reopened), bytes);

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.1);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), edited_digest);

    for (index, edit) in [
        generic_parameter(TWO_DISTANCE, "chamfer_second_distance", 0.009),
        ExactParameterEdit {
            target: ExactParameterEditTarget::FeatureParameter(
                FeatureParameterTarget::new(
                    DISTANCE_ANGLE,
                    "chamfer_angle",
                    ParameterValueType::Angle,
                )
                .unwrap(),
            ),
            dimension: Dimension::from_decimal("0.1").unwrap(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let invalid_before = stamp(&document);
        assert!(
            prepare_body_parameter_edit(
                &document,
                BodyParameterEditRequest {
                    definition_id: DEFINITION,
                    body_id: BodyId(1),
                    edits: vec![edit],
                },
                ProposalPrincipal::LocalAssistant,
            )
            .is_err(),
            "invalid advanced Chamfer parameter case {index} was accepted"
        );
        assert_eq!(stamp(&document), invalid_before);
    }
}
