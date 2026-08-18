use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    PrincipalPlane, SketchSolveStatus, WorkplaneFrame, WorkplaneSupport, WorkplaneSupportHealth,
};
use ketchup_interaction::face_intent::{
    FaceIntentError, FaceIntentTarget, HoverFaceCandidate, TransientFaceIntent,
};
use ketchup_interaction::rectangle_face_authoring::{
    RectangleAuthoringError, RectangleDirection, RectangleFaceAuthoring, RectangleFeatureIds,
    RectangleSize,
};
use std::collections::BTreeSet;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);

fn datum_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Target body".to_owned(),
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

#[test]
fn rectangle_on_all_principal_datums_keeps_axes_bounds_and_one_reviewed_undo() {
    let cases = [
        (
            PrincipalPlane::Xy,
            [
                [10.0, 20.0, 0.0],
                [35.0, 20.0, 0.0],
                [35.0, 28.0, 0.0],
                [10.0, 28.0, 0.0],
            ],
        ),
        (
            PrincipalPlane::Yz,
            [
                [0.0, 10.0, 20.0],
                [0.0, 35.0, 20.0],
                [0.0, 35.0, 28.0],
                [0.0, 10.0, 28.0],
            ],
        ),
        (
            PrincipalPlane::Xz,
            [
                [10.0, 0.0, 20.0],
                [35.0, 0.0, 20.0],
                [35.0, 0.0, 28.0],
                [10.0, 0.0, 28.0],
            ],
        ),
    ];

    for (plane, expected_corners) in cases {
        let mut document = datum_document();
        let snapshot = document.current();
        let before = stamp(&document);
        let target = FaceIntentTarget::datum(DEFINITION, BodyId(2), plane);
        let intent =
            TransientFaceIntent::new(&snapshot, DEFINITION, Vec::new(), Some(target)).unwrap();
        let authoring = RectangleFaceAuthoring::begin(&snapshot, &intent, 0, [10.0, 20.0]).unwrap();
        assert_eq!(authoring.body_id(), BodyId(2));
        assert_eq!(
            authoring.workplane_frame(),
            WorkplaneFrame::principal(plane)
        );

        let pointer = authoring.preview_pointer(&snapshot, [35.0, 28.0]).unwrap();
        assert_eq!(pointer.size().width().millimetres(), 25.0);
        assert_eq!(pointer.size().depth().millimetres(), 8.0);
        assert_eq!(pointer.world_corners_mm(), expected_corners);
        assert_eq!(
            stamp(&document),
            before,
            "preview/cancel must be observational"
        );

        let exact = authoring
            .preview_exact(
                &snapshot,
                RectangleSize::exact(
                    Dimension::from_decimal("30").unwrap(),
                    Dimension::from_decimal("12.5").unwrap(),
                )
                .unwrap(),
                RectangleDirection {
                    positive_x: false,
                    positive_y: true,
                },
            )
            .unwrap();
        assert_eq!(exact.opposite_uv_mm(), [-20.0, 32.5]);
        let proposal = authoring
            .plan_proposal(
                &document,
                RectangleFeatureIds {
                    workplane: FeatureId(10),
                    sketch: FeatureId(11),
                },
                &exact,
            )
            .unwrap();
        assert_eq!(proposal.batch().commands().len(), 3);
        assert_eq!(stamp(&document), before, "review must not commit");

        let commit = document.commit_verified_proposal(&proposal).unwrap();
        assert_eq!(
            commit.revision().dirty_features(),
            &BTreeSet::from([FeatureId(10), FeatureId(11)])
        );
        assert_eq!(document.visible_undo_steps(), before.2 + 1);
        let committed = document.current();
        let committed_digest = committed.canonical_digest();
        let definition = committed.definition(DEFINITION).unwrap();
        assert_eq!(definition.active_body_id(), BodyId(2));
        let FeatureKind::Workplane(workplane) = committed.feature(FeatureId(10)).unwrap().kind()
        else {
            panic!("rectangle must retain an explicit workplane");
        };
        assert_eq!(workplane.frame, WorkplaneFrame::principal(plane));
        assert_eq!(workplane.support, WorkplaneSupport::Principal(plane));
        let FeatureKind::Sketch(sketch) = committed.feature(FeatureId(11)).unwrap().kind() else {
            panic!("rectangle must use the existing sketch geometry family");
        };
        assert_eq!(sketch.workplane, FeatureId(10));
        assert_eq!(
            sketch.solve().unwrap().status,
            SketchSolveStatus::FullyConstrained
        );
        assert_eq!(sketch.solved_regions().unwrap().len(), 1);

        assert!(document.undo().is_some());
        assert!(document.current().feature(FeatureId(10)).is_none());
        assert!(document.current().feature(FeatureId(11)).is_none());
        assert!(document.redo().is_some());
        assert_eq!(document.current().canonical_digest(), committed_digest);

        let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
        let reopened = reopened.snapshot();
        assert_eq!(reopened.canonical_digest(), committed_digest);
        assert_eq!(
            reopened.definition(DEFINITION).unwrap().active_body_id(),
            BodyId(2)
        );
        let FeatureKind::Workplane(reopened_plane) =
            reopened.feature(FeatureId(10)).unwrap().kind()
        else {
            panic!("reopened rectangle must retain its workplane");
        };
        assert_eq!(reopened_plane.frame, WorkplaneFrame::principal(plane));
        assert_eq!(reopened_plane.support, WorkplaneSupport::Principal(plane));
    }
}

#[test]
fn rectangle_on_generated_planar_face_preserves_exact_support_and_target_body() {
    let mut document = datum_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(20),
                definition_id: DEFINITION,
                name: "Base rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 6.0], [0.0, 6.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(21),
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(20),
                    height: Dimension::from_decimal("4").unwrap(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&snapshot, DEFINITION, FeatureId(21))
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
                snapshot.document_id(),
                FeatureId(21),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "input".to_owned(),
        "result".to_owned(),
        "occt".to_owned(),
        "r0".to_owned(),
        [[0.0, 0.0, 0.0], [8.0, 6.0, 4.0]],
        evidence,
    )
    .unwrap();
    let reference = package.reference(ExactFaceRole::Top).unwrap().clone();
    let last_valid =
        ExactResultRegistry::accept(&snapshot, [Arc::new(ExactBodyPackage::from(package))])
            .unwrap();
    document
        .register_exact_reference_evidence(reference.clone())
        .unwrap();
    let snapshot = document.current();
    let frame = WorkplaneFrame::principal(PrincipalPlane::Xy).offset(4.0);
    let before_rejected = stamp(&document);
    let last_valid_stamp = last_valid.contents_stamp();
    for health in [
        WorkplaneSupportHealth::Ambiguous,
        WorkplaneSupportHealth::Lost,
    ] {
        let rejected_target =
            FaceIntentTarget::planar_face(DEFINITION, BodyId(2), reference.clone(), health, frame);
        let rejected_intent = TransientFaceIntent::new(
            &snapshot,
            DEFINITION,
            vec![HoverFaceCandidate {
                target: rejected_target,
                ray_distance_mm: 2.0,
                visible: true,
            }],
            None,
        )
        .unwrap();
        assert!(matches!(
            RectangleFaceAuthoring::begin(&snapshot, &rejected_intent, 0, [1.0, 1.0]),
            Err(RectangleAuthoringError::FaceIntent(
                FaceIntentError::UnresolvedReference(actual)
            )) if actual == health
        ));
        assert_eq!(stamp(&document), before_rejected);
        assert_eq!(last_valid.contents_stamp(), last_valid_stamp);
    }
    let target = FaceIntentTarget::planar_face(
        DEFINITION,
        BodyId(2),
        reference.clone(),
        WorkplaneSupportHealth::Resolved,
        frame,
    );
    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target,
            ray_distance_mm: 2.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    let authoring = RectangleFaceAuthoring::begin(&snapshot, &intent, 0, [1.0, 1.0]).unwrap();
    let preview = authoring.preview_pointer(&snapshot, [5.0, 3.0]).unwrap();
    let proposal = authoring
        .plan_proposal(
            &document,
            RectangleFeatureIds {
                workplane: FeatureId(22),
                sketch: FeatureId(23),
            },
            &preview,
        )
        .unwrap();
    let commit = document.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        commit.revision().dirty_features(),
        &BTreeSet::from([FeatureId(22), FeatureId(23)])
    );

    let committed = document.current();
    let carried = ExactResultRegistry::carried_forward(&committed, &last_valid);
    assert_eq!(carried.len(), 1);
    assert!(
        carried
            .get_body(&committed, DEFINITION, BodyId(2))
            .unwrap()
            .is_some(),
        "unaffected terminal-body geometry must remain last-valid"
    );
    assert_eq!(
        committed.definition(DEFINITION).unwrap().active_body_id(),
        BodyId(2)
    );
    let FeatureKind::Workplane(workplane) = committed.feature(FeatureId(22)).unwrap().kind() else {
        panic!("face rectangle must create a workplane");
    };
    assert_eq!(workplane.frame, frame);
    assert!(matches!(
        &workplane.support,
        WorkplaneSupport::PlanarFace { reference: saved, health: WorkplaneSupportHealth::Resolved }
            if saved.as_ref() == &reference
    ));
    assert_eq!(
        preview.world_corners_mm(),
        [
            [1.0, 1.0, 4.0],
            [5.0, 1.0, 4.0],
            [5.0, 3.0, 4.0],
            [1.0, 3.0, 4.0],
        ]
    );

    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    let reopened = reopened.snapshot();
    assert_eq!(reopened.canonical_digest(), committed.canonical_digest());
    let FeatureKind::Workplane(reopened_plane) = reopened.feature(FeatureId(22)).unwrap().kind()
    else {
        panic!("reopened face rectangle must retain its workplane");
    };
    assert_eq!(reopened_plane.frame, frame);
    assert!(matches!(
        &reopened_plane.support,
        WorkplaneSupport::PlanarFace { reference: saved, health: WorkplaneSupportHealth::Resolved }
            if saved.as_ref() == &reference
    ));

    let current = document.current();
    let fresh_intent = TransientFaceIntent::new(
        &current,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: preview.target().target.clone(),
            ray_distance_mm: 2.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    let fresh_authoring =
        RectangleFaceAuthoring::begin(&current, &fresh_intent, 0, [2.0, 2.0]).unwrap();
    let fresh_preview = fresh_authoring
        .preview_pointer(&current, [4.0, 4.0])
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(21),
                dimension: Dimension::from_decimal("5").unwrap(),
            },
        ]))
        .unwrap();
    let after_support_change = stamp(&document);
    let stale_snapshot = document.current();
    let FeatureKind::Workplane(stale_plane) = stale_snapshot.feature(FeatureId(22)).unwrap().kind()
    else {
        panic!("face workplane must remain present but stale");
    };
    assert!(matches!(
        stale_plane.support,
        WorkplaneSupport::PlanarFace {
            health: WorkplaneSupportHealth::Stale,
            ..
        }
    ));
    assert!(matches!(
        fresh_authoring.plan_proposal(
            &document,
            RectangleFeatureIds {
                workplane: FeatureId(24),
                sketch: FeatureId(25),
            },
            &fresh_preview,
        ),
        Err(RectangleAuthoringError::StaleIntent)
    ));
    assert_eq!(stamp(&document), after_support_change);
    assert_eq!(last_valid.contents_stamp(), last_valid_stamp);
}

#[test]
fn degenerate_preview_and_stale_confirmation_fail_without_canonical_mutation() {
    let mut document = datum_document();
    let snapshot = document.current();
    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        Vec::new(),
        Some(FaceIntentTarget::datum(
            DEFINITION,
            BodyId(2),
            PrincipalPlane::Xy,
        )),
    )
    .unwrap();
    let authoring = RectangleFaceAuthoring::begin(&snapshot, &intent, 0, [0.0, 0.0]).unwrap();
    let before = stamp(&document);
    let before_bytes = persistence::save(&snapshot);
    for duplicate_or_degenerate in [[0.0, 0.0], [0.0, 2.0], [2.0, 0.0]] {
        assert!(matches!(
            authoring.preview_pointer(&snapshot, duplicate_or_degenerate),
            Err(RectangleAuthoringError::InvalidDimensions)
        ));
    }
    assert!(
        RectangleSize::exact(
            Dimension::from_decimal("0.01").unwrap(),
            Dimension::from_decimal("2").unwrap(),
        )
        .is_err()
    );
    assert_eq!(stamp(&document), before);

    let preview = authoring.preview_pointer(&snapshot, [4.0, 2.0]).unwrap();
    assert!(matches!(
        authoring.plan_proposal(
            &document,
            RectangleFeatureIds {
                workplane: FeatureId(10),
                sketch: FeatureId(10),
            },
            &preview,
        ),
        Err(RectangleAuthoringError::Proposal(_))
    ));
    assert_eq!(stamp(&document), before, "failed review must be atomic");
    assert_eq!(persistence::save(&document.current()), before_bytes);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Changed".to_owned(),
        }]))
        .unwrap();
    let after_external_change = stamp(&document);
    assert!(matches!(
        authoring.plan_proposal(
            &document,
            RectangleFeatureIds {
                workplane: FeatureId(10),
                sketch: FeatureId(11),
            },
            &preview,
        ),
        Err(RectangleAuthoringError::StaleIntent)
    ));
    assert_eq!(stamp(&document), after_external_change);
}
