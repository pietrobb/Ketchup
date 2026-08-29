use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureEvaluationState, FeatureId, FeatureKind, ProposalCommitError, ProposalContext,
    StableFaceRole,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    ExactProfileSegment, ExactResultRegistry, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, PadPocketOperation, PadSpec, PocketSpec,
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchEntityId, SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec,
    WorkplaneSupport, WorkplaneSupportHealth,
};
use ketchup_core::state_view::encode_semantic_state;
use std::collections::BTreeSet;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const WORKPLANE: FeatureId = FeatureId(10);
const SKETCH: FeatureId = FeatureId(11);
const PAD: FeatureId = FeatureId(12);

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn circle_sketch(workplane: FeatureId, center_mm: [f64; 2], radius_mm: f64) -> SketchSpec {
    SketchSpec {
        workplane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm,
            radius_mm,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::new(radius_mm.to_string(), radius_mm).unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Center),
                    position_mm: center_mm,
                },
            },
        ],
    }
}

fn line_arc_sketch(workplane: FeatureId) -> SketchSpec {
    SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [10.0, 10.0],
                end_mm: [30.0, 10.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [10.0, 10.0],
                end_mm: [30.0, 10.0],
                center_mm: [20.0, 10.0],
                clockwise: true,
            },
        ],
        constraints: Vec::new(),
    }
}

fn rectangle_sketch(workplane: FeatureId, min_mm: [f64; 2], max_mm: [f64; 2]) -> SketchSpec {
    let corners = [
        min_mm,
        [max_mm[0], min_mm[1]],
        max_mm,
        [min_mm[0], max_mm[1]],
    ];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % corners.len()],
        })
        .collect::<Vec<_>>();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = index as u64 + 1;
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::Start),
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::End),
                position_mm: corners[(index + 1) % corners.len()],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn pad_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Parametric part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Solved circle".into(),
                kind: FeatureKind::Sketch(circle_sketch(
                    SKETCH_OFFSET_WORKPLANE,
                    [10.0, 20.0],
                    5.0,
                )),
            },
        ]))
        .unwrap();
    document
}

const SKETCH_OFFSET_WORKPLANE: FeatureId = WORKPLANE;

fn pad_operation(document: &DocumentStore) -> PadPocketOperation {
    let snapshot = document.current();
    let sketch = match snapshot.feature(SKETCH).unwrap().kind() {
        FeatureKind::Sketch(sketch) => sketch,
        _ => unreachable!(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    PadPocketOperation::Pad(PadSpec {
        sketch: SKETCH,
        region,
        direction: FeatureDirection::AlongNormal,
        extent: FeatureExtent::Blind(Dimension::from_decimal("25").unwrap()),
    })
}

#[test]
fn canonical_line_arc_region_flows_losslessly_into_exact_pad_request() {
    let sketch = line_arc_sketch(WORKPLANE);
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Line-arc part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Line-arc sketch".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: PAD,
                definition_id: DEFINITION,
                name: "Line-arc Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: SKETCH,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("12").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let mixed = request.mixed_profile.as_ref().expect("line-arc profile");
    assert_eq!(mixed.segments.len(), 2);
    assert!(matches!(
        mixed.segments[0],
        ExactProfileSegment::Line { .. }
    ));
    assert!(matches!(
        mixed.segments[1],
        ExactProfileSegment::CircularArc { .. }
    ));
    assert_eq!(
        mixed.bounds_bits.map(f64::from_bits),
        [10.0, 10.0, 30.0, 20.0]
    );
    assert!(request.canonical_input_digest.len() >= 64);

    let bytes = persistence::save(&snapshot);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&reopened.snapshot(), DEFINITION).unwrap(),
        request
    );

    let before_revision_count = document.revision_count();
    let before_undo = document.visible_undo_steps();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::TranslateProfile {
                id: SKETCH,
                delta_mm: [2.0, 0.0],
            },
        ]))
        .unwrap();
    assert_eq!(document.revision_count(), before_revision_count + 1);
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    let changed = document.current();
    let changed_request = ExactFeatureChainRequest::from_snapshot(&changed, DEFINITION).unwrap();
    assert_ne!(
        changed_request.canonical_input_digest,
        request.canonical_input_digest
    );
    let changed_bytes = persistence::save(&changed);
    let changed_reopened = persistence::load(&changed_bytes).unwrap();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&changed_reopened.snapshot(), DEFINITION).unwrap(),
        changed_request
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&document.undo().unwrap(), DEFINITION).unwrap(),
        request
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&document.redo().unwrap(), DEFINITION).unwrap(),
        changed_request
    );

    const FACE_PLANE: FeatureId = FeatureId(13);
    const POCKET_SKETCH: FeatureId = FeatureId(14);
    const POCKET: FeatureId = FeatureId(15);
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::ArcSide,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                changed_request.document_id,
                changed_request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry.{role:?}"),
        )
    });
    let package = build_box_render_package(
        &changed_request,
        "exact-input".into(),
        "mixed-result".into(),
        "test-backend".into(),
        "test-tolerance".into(),
        changed_request.expected_bounds_mm(),
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    assert_eq!(
        document
            .current()
            .exact_reference_by_lineage(&top.lineage_digest),
        Some(&top)
    );
    let top_frame = document
        .current()
        .resolved_planar_face_workplane_frame(&top)
        .expect("mixed Pad top frame");
    let pocket_sketch = rectangle_sketch(FACE_PLANE, [12.0, 12.0], [18.0, 15.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FACE_PLANE,
                definition_id: DEFINITION,
                name: "Mixed Pad top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: top_frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_SKETCH,
                definition_id: DEFINITION,
                name: "Pocket sketch".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
        ]))
        .unwrap();
    let before_pocket = document.current();
    let before_pocket_undo = document.visible_undo_steps();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: POCKET,
            definition_id: DEFINITION,
            name: "Unsupported mixed-base Pocket".into(),
            kind: FeatureKind::SketchPocket(PocketSpec {
                target: PAD,
                sketch: POCKET_SKETCH,
                region: pocket_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("4").unwrap()),
            }),
        }]))
        .unwrap();
    assert_eq!(document.visible_undo_steps(), before_pocket_undo + 1);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&document.current(), DEFINITION),
        Err(ExactProductError::UnsupportedThroughCut)
    );
    let unsupported_bytes = persistence::save(&document.current());
    let unsupported_reopened = persistence::load(&unsupported_bytes).unwrap();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&unsupported_reopened.snapshot(), DEFINITION),
        Err(ExactProductError::UnsupportedThroughCut)
    );
    let undone_request =
        ExactFeatureChainRequest::from_snapshot(&document.undo().unwrap(), DEFINITION).unwrap();
    assert_eq!(
        undone_request.canonical_input_digest,
        changed_request.canonical_input_digest
    );
    assert_eq!(undone_request.mixed_profile, changed_request.mixed_profile);
    assert_eq!(
        document.current().canonical_digest(),
        before_pocket.canonical_digest()
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&document.redo().unwrap(), DEFINITION),
        Err(ExactProductError::UnsupportedThroughCut)
    );
}

#[test]
fn explicit_workplane_pad_has_shared_proposal_preview_persistence_and_stale_refusal() {
    let mut document = pad_document();
    let before = document.current();
    let before_digest = before.canonical_digest();
    let before_revision_count = document.revision_count();
    let before_undo = document.visible_undo_steps();
    let before_redo = document.visible_redo_steps();
    let operation = pad_operation(&document);

    let manual = document
        .plan_pad_pocket(
            PAD,
            DEFINITION,
            "Pad",
            operation.clone(),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    let assistant = document
        .plan_pad_pocket(
            PAD,
            DEFINITION,
            "Pad",
            operation,
            ProposalContext::local_assistant_model(),
        )
        .unwrap();
    assert_eq!(manual.batch(), assistant.batch());
    assert_eq!(manual.command_digest(), assistant.command_digest());
    assert_eq!(
        manual.intended_result_digest(),
        assistant.intended_result_digest()
    );
    assert_eq!(
        manual.authoritative_dependencies(),
        assistant.authoritative_dependencies()
    );
    assert_eq!(
        manual.authoritative_writes(),
        assistant.authoritative_writes()
    );
    assert_eq!(manual.authoritative_diff(), assistant.authoritative_diff());
    assert_ne!(manual.principal(), assistant.principal());

    let preview = document.preview_batch(manual.batch()).unwrap();
    assert_eq!(
        preview.canonical_digest(),
        document
            .preview_batch(manual.batch())
            .unwrap()
            .canonical_digest()
    );
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.revision_count(), before_revision_count);
    assert_eq!(document.visible_undo_steps(), before_undo);
    assert_eq!(document.visible_redo_steps(), before_redo);

    let verified = document.commit_verified_proposal(&manual).unwrap();
    assert_eq!(verified.revision().batch_digest(), manual.command_digest());
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    let committed = document.current();
    let FeatureKind::Pad(spec) = committed.feature(PAD).unwrap().kind() else {
        panic!("expected canonical Pad");
    };
    assert_eq!(spec.sketch, SKETCH);
    let committed_digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );

    let mut stale_document = pad_document();
    let stale = stale_document
        .plan_pad_pocket(
            PAD,
            DEFINITION,
            "Pad",
            pad_operation(&stale_document),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    stale_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: SKETCH },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Changed solved circle".into(),
                kind: FeatureKind::Sketch(circle_sketch(WORKPLANE, [10.0, 20.0], 6.0)),
            },
        ]))
        .unwrap();
    let changed = stale_document.current();
    let changed_digest = changed.canonical_digest();
    let changed_revision_count = stale_document.revision_count();
    let changed_undo = stale_document.visible_undo_steps();
    let changed_redo = stale_document.visible_redo_steps();
    assert!(matches!(
        stale_document.commit_verified_proposal(&stale),
        Err(ProposalCommitError::Stale { .. })
    ));
    assert_eq!(stale_document.current().canonical_digest(), changed_digest);
    assert_eq!(stale_document.revision_count(), changed_revision_count);
    assert_eq!(stale_document.visible_undo_steps(), changed_undo);
    assert_eq!(stale_document.visible_redo_steps(), changed_redo);
}

#[test]
fn face_supported_pocket_keeps_exact_target_support_and_is_lossless() {
    const BASE_PLANE: FeatureId = FeatureId(20);
    const BASE_SKETCH: FeatureId = FeatureId(21);
    const BASE_PAD: FeatureId = FeatureId(22);
    const FACE_PLANE: FeatureId = FeatureId(23);
    const POCKET_SKETCH: FeatureId = FeatureId(24);
    const POCKET: FeatureId = FeatureId(25);

    let base_sketch = rectangle_sketch(BASE_PLANE, [10.0, 20.0], [110.0, 80.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Pocket part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_SKETCH,
                definition_id: DEFINITION,
                name: "Base rectangle".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PAD,
                definition_id: DEFINITION,
                name: "Base Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: BASE_SKETCH,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("18").unwrap()),
                }),
            },
        ]))
        .unwrap();
    let base = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&base, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), BASE_PAD);
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
        "exact-input".into(),
        "base-result".into(),
        "test-backend".into(),
        "test-tolerance".into(),
        request.expected_bounds_mm(),
        evidence,
    )
    .unwrap();
    assert_eq!(package.bounds_mm, [[10.0, 20.0, 0.0], [110.0, 80.0, 18.0]]);
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let frame = WorkplaneFrame {
        origin_mm: [10.0, 20.0, 18.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    let sketch = line_arc_sketch(FACE_PLANE);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FACE_PLANE,
                definition_id: DEFINITION,
                name: "Base top face".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_SKETCH,
                definition_id: DEFINITION,
                name: "Pocket rectangle".into(),
                kind: FeatureKind::Sketch(sketch.clone()),
            },
        ]))
        .unwrap();
    let region = sketch.solved_regions().unwrap()[0].id;
    let operation = PadPocketOperation::Pocket(PocketSpec {
        target: BASE_PAD,
        sketch: POCKET_SKETCH,
        region,
        support: Box::new(top.clone()),
        direction: FeatureDirection::OppositeNormal,
        extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
    });
    let proposal = document
        .plan_pad_pocket(
            POCKET,
            DEFINITION,
            "Pocket",
            operation.clone(),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    let assistant = document
        .plan_pad_pocket(
            POCKET,
            DEFINITION,
            "Pocket",
            operation,
            ProposalContext::local_assistant_model(),
        )
        .unwrap();
    assert_eq!(proposal.batch(), assistant.batch());
    assert_eq!(proposal.command_digest(), assistant.command_digest());
    assert_eq!(
        proposal.intended_result_digest(),
        assistant.intended_result_digest()
    );
    assert_eq!(
        proposal.authoritative_dependencies(),
        assistant.authoritative_dependencies()
    );
    assert_eq!(
        proposal.authoritative_writes(),
        assistant.authoritative_writes()
    );
    assert_eq!(
        proposal.authoritative_diff(),
        assistant.authoritative_diff()
    );
    assert_ne!(proposal.principal(), assistant.principal());
    let before = document.current().canonical_digest();
    let preview = document.preview_batch(proposal.batch()).unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(
        preview.canonical_digest(),
        document
            .preview_batch(proposal.batch())
            .unwrap()
            .canonical_digest()
    );
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    let FeatureKind::SketchPocket(spec) = committed.feature(POCKET).unwrap().kind() else {
        panic!("expected canonical sketch Pocket");
    };
    assert_eq!(spec.target, BASE_PAD);
    assert_eq!(spec.support.as_ref(), &top);
    assert_eq!(spec.extent.blind_distance().unwrap().millimetres(), 6.0);
    let pocket_request = ExactFeatureChainRequest::from_snapshot(&committed, DEFINITION).unwrap();
    assert_eq!(pocket_request.producer_feature_id(), POCKET);
    assert_eq!(pocket_request.pocket_depth_bits, Some(6.0_f64.to_bits()));
    let pocket_profile = pocket_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .expect("canonical line-arc pocket profile");
    assert_eq!(pocket_profile.segments.len(), 2);
    assert!(matches!(
        pocket_profile.segments[1],
        ExactProfileSegment::CircularArc { .. }
    ));
    assert_ne!(
        pocket_request.canonical_input_digest,
        request.canonical_input_digest
    );
    let committed_digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn offset_workplane_dimension_recomputes_pad_frame_in_one_undoable_step() {
    const BASE_PLANE: FeatureId = FeatureId(30);
    const OFFSET_PLANE: FeatureId = FeatureId(31);
    const OFFSET_SKETCH: FeatureId = FeatureId(32);
    const OFFSET_PAD: FeatureId = FeatureId(33);

    let sketch = rectangle_sketch(OFFSET_PLANE, [0.0, 0.0], [20.0, 10.0]);
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Offset Pad".into(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_PLANE,
                definition_id: DEFINITION,
                name: "Offset XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset {
                        base: BASE_PLANE,
                        distance: Dimension::from_decimal("5").unwrap(),
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_SKETCH,
                definition_id: DEFINITION,
                name: "Offset rectangle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_PAD,
                definition_id: DEFINITION,
                name: "Offset Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: OFFSET_SKETCH,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("10").unwrap()),
                }),
            },
        ]))
        .unwrap();
    let before = document.current();
    let before_request = ExactFeatureChainRequest::from_snapshot(&before, DEFINITION).unwrap();
    assert_eq!(
        before_request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [20.0, 10.0, 10.0]]
    );
    assert_eq!(
        before_request.workplane_frame_bits.unwrap()[0..3]
            .iter()
            .copied()
            .map(f64::from_bits)
            .collect::<Vec<_>>(),
        vec![0.0, 0.0, 5.0]
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET_PLANE,
                dimension: Dimension::from_decimal("8").unwrap(),
            },
        ]))
        .unwrap();
    let changed = document.current();
    let FeatureKind::Workplane(changed_plane) = changed.feature(OFFSET_PLANE).unwrap().kind()
    else {
        panic!("expected offset workplane");
    };
    assert_eq!(changed_plane.frame.origin_mm, [0.0, 0.0, 8.0]);
    let changed_request = ExactFeatureChainRequest::from_snapshot(&changed, DEFINITION).unwrap();
    assert_eq!(
        changed_request.workplane_frame_bits.unwrap()[0..3]
            .iter()
            .copied()
            .map(f64::from_bits)
            .collect::<Vec<_>>(),
        vec![0.0, 0.0, 8.0]
    );
    let changed_digest = changed.canonical_digest();
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);
}

#[test]
fn branched_feature_dag_recomputes_only_the_dirty_closure_and_keeps_unrelated_exact_results() {
    const PLANE_A: FeatureId = FeatureId(100);
    const SKETCH_A: FeatureId = FeatureId(101);
    const PAD_A: FeatureId = FeatureId(102);
    const PLANE_B: FeatureId = FeatureId(200);
    const SKETCH_B: FeatureId = FeatureId(201);
    const PAD_B: FeatureId = FeatureId(202);

    let sketch_a = rectangle_sketch(PLANE_A, [0.0, 0.0], [20.0, 10.0]);
    let sketch_b = rectangle_sketch(PLANE_B, [0.0, 0.0], [30.0, 15.0]);
    let region_a = sketch_a.solved_regions().unwrap()[0].id;
    let region_b = sketch_b.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Branched part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PLANE_A,
                definition_id: DEFINITION,
                name: "Plane A".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH_A,
                definition_id: DEFINITION,
                name: "Sketch A".into(),
                kind: FeatureKind::Sketch(sketch_a.clone()),
            },
            CanonicalCommand::CreateFeature {
                id: PAD_A,
                definition_id: DEFINITION,
                name: "Pad A".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: SKETCH_A,
                    region: region_a,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("10").unwrap()),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: PLANE_B,
                definition_id: DEFINITION,
                name: "Plane B".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH_B,
                definition_id: DEFINITION,
                name: "Sketch B".into(),
                kind: FeatureKind::Sketch(sketch_b),
            },
            CanonicalCommand::CreateFeature {
                id: PAD_B,
                definition_id: DEFINITION,
                name: "Pad B".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: SKETCH_B,
                    region: region_b,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("12").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let before = document.current();
    let graph = before.feature_dependency_graph().unwrap();
    assert_eq!(
        graph.topological_order(),
        &[PLANE_A, SKETCH_A, PAD_A, PLANE_B, SKETCH_B, PAD_B]
    );
    assert_eq!(
        graph.dependencies(PAD_A).unwrap(),
        &BTreeSet::from([SKETCH_A])
    );
    assert_eq!(
        graph.dependents(SKETCH_B).unwrap(),
        &BTreeSet::from([PAD_B])
    );

    let package_for = |producer| {
        let request =
            ExactFeatureChainRequest::from_snapshot_for_producer(&before, DEFINITION, producer)
                .unwrap();
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
                    producer,
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                ),
                format!("geometry.{producer:?}.{role:?}"),
            )
        });
        Arc::new(ExactBodyPackage::from(
            build_box_render_package(
                &request,
                format!("exact-{producer:?}"),
                format!("result-{producer:?}"),
                "test-backend".into(),
                "test-tolerance".into(),
                request.expected_bounds_mm(),
                evidence,
            )
            .unwrap(),
        ))
    };
    let registry =
        ExactResultRegistry::accept(&before, [package_for(PAD_A), package_for(PAD_B)]).unwrap();

    let changed_sketch_a = rectangle_sketch(PLANE_A, [0.0, 0.0], [25.0, 10.0]);
    let revision = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: SKETCH_A },
            CanonicalCommand::CreateFeature {
                id: SKETCH_A,
                definition_id: DEFINITION,
                name: "Sketch A changed".into(),
                kind: FeatureKind::Sketch(changed_sketch_a),
            },
        ]))
        .unwrap();
    assert_eq!(
        revision.dirty_features(),
        &BTreeSet::from([SKETCH_A, PAD_A])
    );
    assert_eq!(
        revision.feature_states()[&PAD_A],
        FeatureEvaluationState::Stale
    );
    assert_eq!(
        revision.feature_states()[&PAD_B],
        FeatureEvaluationState::Current
    );
    let error_states = document
        .current()
        .feature_dependency_graph()
        .unwrap()
        .evaluation_states(&BTreeSet::new(), &BTreeSet::from([SKETCH_A]));
    assert_eq!(
        error_states[&PAD_A],
        FeatureEvaluationState::Error {
            failed_at: SKETCH_A
        }
    );
    assert_eq!(error_states[&PAD_B], FeatureEvaluationState::Current);

    let changed = document.current();
    let carried = ExactResultRegistry::carried_forward(&changed, &registry);
    assert_eq!(carried.len(), 1);
    let remaining = carried.values().next().unwrap();
    assert_eq!(remaining.producer_feature_id(), PAD_B);
    assert!(remaining.is_current(&changed));
}

#[test]
fn feature_dependency_cycle_is_rejected_atomically() {
    const FIRST_SHELL: FeatureId = FeatureId(300);
    const SECOND_SHELL: FeatureId = FeatureId(301);
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let revisions = document.revision_count();
    let undo = document.visible_undo_steps();
    let redo = document.visible_redo_steps();
    let shell = |target| FeatureKind::Shell {
        target,
        removed_faces: vec![StableFaceRole::new("test.face").unwrap()],
        thickness: Dimension::from_decimal("1").unwrap(),
    };

    let error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Cycle".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FIRST_SHELL,
            definition_id: DEFINITION,
            name: "First".into(),
            kind: shell(SECOND_SHELL),
        },
        CanonicalCommand::CreateFeature {
            id: SECOND_SHELL,
            definition_id: DEFINITION,
            name: "Second".into(),
            kind: shell(FIRST_SHELL),
        },
    ])) {
        Ok(_) => panic!("feature cycle was accepted"),
        Err(error) => error,
    };
    assert_eq!(error, CanonicalError::FeatureDependencyCycle(FIRST_SHELL));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.revision_count(), revisions);
    assert_eq!(document.visible_undo_steps(), undo);
    assert_eq!(document.visible_redo_steps(), redo);
}

#[test]
fn universal_extent_contract_round_trips_changes_digest_and_fails_closed_before_graph_resolution() {
    let mut document = pad_document();
    let proposal = document
        .plan_pad_pocket(
            PAD,
            DEFINITION,
            "Pad",
            pad_operation(&document),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
    let blind = document.current();
    let blind_digest = blind.canonical_digest();
    let request = ExactFeatureChainRequest::from_snapshot(&blind, DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::CircleSide,
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
            format!("extent-contract.{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "extent-contract-input".into(),
        "extent-contract-result".into(),
        "test-backend".into(),
        "test-tolerance".into(),
        request.expected_bounds_mm(),
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    let region = match blind.feature(SKETCH).unwrap().kind() {
        FeatureKind::Sketch(sketch) => sketch.solved_regions().unwrap()[0].id,
        _ => unreachable!(),
    };
    let baseline_bytes = persistence::save(&blind);
    let extents = [
        FeatureExtent::Blind(Dimension::from_decimal("8").unwrap()),
        FeatureExtent::ThroughAll,
        FeatureExtent::UpToFace(Box::new(top.clone())),
        FeatureExtent::Symmetric(Dimension::from_decimal("16").unwrap()),
        FeatureExtent::Bidirectional {
            along: FeatureExtentEnd::Blind(Dimension::from_decimal("8").unwrap()),
            opposite: FeatureExtentEnd::UpToFace(Box::new(top.clone())),
        },
    ];
    let mut extent_digests = BTreeSet::new();
    for extent in extents {
        let expected = FeatureKind::Pad(PadSpec {
            sketch: SKETCH,
            region,
            direction: FeatureDirection::Vector([1.0, 2.0, 3.0]),
            extent,
        });
        let mut candidate = persistence::load(&baseline_bytes)
            .unwrap()
            .into_editable()
            .unwrap_or_else(|_| panic!("current extent schema must remain editable"));
        candidate
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteFeature { id: PAD },
                CanonicalCommand::CreateFeature {
                    id: PAD,
                    definition_id: DEFINITION,
                    name: "Extent candidate".into(),
                    kind: expected.clone(),
                },
            ]))
            .unwrap();
        let candidate_snapshot = candidate.current();
        extent_digests.insert(candidate_snapshot.canonical_digest());
        let candidate_bytes = persistence::save(&candidate_snapshot);
        let candidate_reopened = persistence::load(&candidate_bytes).unwrap();
        assert_eq!(
            candidate_reopened.snapshot().feature(PAD).unwrap().kind(),
            &expected
        );
        assert_eq!(
            persistence::save(&candidate_reopened.snapshot()),
            candidate_bytes
        );
    }
    assert_eq!(extent_digests.len(), 5);

    let generalized_pad = FeatureId(13);
    let generalized_spec = PadSpec {
        sketch: SKETCH,
        region,
        direction: FeatureDirection::Vector([1.0, 2.0, 3.0]),
        extent: FeatureExtent::Bidirectional {
            along: FeatureExtentEnd::Blind(Dimension::from_decimal("8").unwrap()),
            opposite: FeatureExtentEnd::UpToFace(Box::new(top)),
        },
    };
    let manual = document
        .plan_pad_pocket(
            generalized_pad,
            DEFINITION,
            "Generalized Pad",
            PadPocketOperation::Pad(generalized_spec.clone()),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    let assistant = document
        .plan_pad_pocket(
            generalized_pad,
            DEFINITION,
            "Generalized Pad",
            PadPocketOperation::Pad(generalized_spec.clone()),
            ProposalContext::local_assistant_model(),
        )
        .unwrap();
    assert_eq!(manual.provenance_revision(), blind.revision_id());
    assert_eq!(manual.provenance_digest(), blind_digest);
    assert_eq!(manual.batch().commands().len(), 1);
    assert_eq!(manual.cost().commands, 1);
    assert_eq!(manual.batch(), assistant.batch());
    assert_eq!(manual.command_digest(), assistant.command_digest());
    assert_eq!(
        manual.intended_result_digest(),
        assistant.intended_result_digest()
    );
    assert_ne!(manual.principal(), assistant.principal());
    let undo_before = document.visible_undo_steps();
    let revisions_before = document.revision_count();
    let preview = document.preview_batch(manual.batch()).unwrap();
    let preview_digest = preview.canonical_digest();
    assert_eq!(
        preview_digest,
        document
            .preview_batch(assistant.batch())
            .unwrap()
            .canonical_digest()
    );
    assert_eq!(document.current().canonical_digest(), blind_digest);
    assert_eq!(document.visible_undo_steps(), undo_before);
    assert_eq!(document.revision_count(), revisions_before);
    let committed = document.commit_verified_proposal(&manual).unwrap();
    assert_eq!(committed.revision().batch_digest(), manual.command_digest());
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    assert_eq!(document.revision_count(), revisions_before + 1);
    let changed = document.current();
    let changed_digest = changed.canonical_digest();
    assert_eq!(changed_digest, preview_digest);
    assert_ne!(changed_digest, blind_digest);
    assert_eq!(
        changed.feature(generalized_pad).unwrap().kind(),
        &FeatureKind::Pad(generalized_spec.clone())
    );
    assert_eq!(
        FeatureDirection::Vector([1.0, 2.0, 3.0])
            .vector([0.0, 0.0, 1.0])
            .unwrap(),
        [
            1.0_f64 / 14.0_f64.sqrt(),
            2.0 / 14.0_f64.sqrt(),
            3.0 / 14.0_f64.sqrt()
        ]
    );
    assert!(ExactBRepGraph::from_snapshot(&changed, DEFINITION, generalized_pad).is_err());

    let bytes = persistence::save(&changed);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), changed_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        reopened.snapshot().feature(generalized_pad).unwrap().kind(),
        &FeatureKind::Pad(generalized_spec)
    );
    let state = encode_semantic_state(&reopened.snapshot()).complete_v1();
    assert!(state.contains("feature.13.extent.mode=bidirectional"));
    assert!(state.contains("feature.13.extent.along.mode=blind"));
    assert!(state.contains("feature.13.extent.opposite.mode=up_to_face"));

    assert_eq!(document.undo().unwrap().canonical_digest(), blind_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);
    let before_invalid = document.current().canonical_digest();
    let revisions_before_invalid = document.revision_count();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteFeature { id: PAD },
                CanonicalCommand::CreateFeature {
                    id: PAD,
                    definition_id: DEFINITION,
                    name: "Invalid Pad".into(),
                    kind: FeatureKind::Pad(PadSpec {
                        sketch: SKETCH,
                        region,
                        direction: FeatureDirection::Vector([0.0, 0.0, 0.0]),
                        extent: FeatureExtent::ThroughAll,
                    }),
                },
            ]))
            .is_err()
    );
    assert_eq!(document.current().canonical_digest(), before_invalid);
    assert_eq!(document.revision_count(), revisions_before_invalid);
    assert!(
        FeatureDirection::Vector([f64::NAN, 0.0, 1.0])
            .validate()
            .is_err()
    );
    assert!(
        FeatureDirection::Vector([f64::INFINITY, 0.0, 1.0])
            .validate()
            .is_err()
    );
    assert!(Dimension::new("1000001", 1_000_001.0).is_err());
}
