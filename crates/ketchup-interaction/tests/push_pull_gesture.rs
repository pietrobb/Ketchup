use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureEvaluationState, FeatureId, FeatureKind,
};
use ketchup_core::exact_product::{
    BodySubshapeRef, ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    ExactRenderPackage, ExactResultRegistry, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{PrincipalPlane, WorkplaneFrame, WorkplaneSupportHealth};
use ketchup_interaction::face_intent::{
    FaceIntentError, FaceIntentSource, FaceIntentTarget, HoverFaceCandidate, TransientFaceIntent,
};
use ketchup_interaction::push_pull_gesture::{
    PushPullGestureError, PushPullPreviewSource, PushPullSnapCandidate, PushPullSnapKind,
    PushPullSnapSettings, SmartPushPullGesture,
};
use std::collections::BTreeSet;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const BODY: BodyId = BodyId(2);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const OTHER_BODY: BodyId = BodyId(3);
const OTHER_PROFILE: FeatureId = FeatureId(20);
const OTHER_EXTRUSION: FeatureId = FeatureId(21);

fn exact_package(
    snapshot: &ketchup_core::document::Snapshot,
    producer: FeatureId,
    bounds_mm: [[f64; 3]; 2],
) -> ExactRenderPackage {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_producer(snapshot, DEFINITION, producer)
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
                producer,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{producer:?}:{role:?}"),
        )
    });
    build_box_render_package(
        &request,
        format!("input:{producer:?}"),
        format!("result:{producer:?}"),
        "occt".to_owned(),
        "r0".to_owned(),
        bounds_mm,
        evidence,
    )
    .unwrap()
}

fn source_document() -> (DocumentStore, BodySubshapeRef, BodySubshapeRef) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BODY,
                name: "Body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BODY,
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: OTHER_BODY,
                name: "Other body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: OTHER_BODY,
            },
            CanonicalCommand::CreateFeature {
                id: OTHER_PROFILE,
                definition_id: DEFINITION,
                name: "Other rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 5.0], [0.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: OTHER_EXTRUSION,
                definition_id: DEFINITION,
                name: "Other extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: OTHER_PROFILE,
                    height: Dimension::from_decimal("12").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BODY,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let package = exact_package(&snapshot, EXTRUSION, [[0.0, 0.0, 0.0], [10.0, 8.0, 20.0]]);
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    let east = package.reference(ExactFaceRole::East).unwrap().clone();
    document
        .register_exact_reference_evidence(top.clone())
        .unwrap();
    document
        .register_exact_reference_evidence(east.clone())
        .unwrap();
    (document, top, east)
}

fn top_target(reference: BodySubshapeRef) -> FaceIntentTarget {
    FaceIntentTarget::planar_face(
        DEFINITION,
        BODY,
        reference,
        WorkplaneSupportHealth::Resolved,
        WorkplaneFrame::principal(PrincipalPlane::Xy).offset(20.0),
    )
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
fn p_binds_hover_before_selection_and_commits_pointer_or_exact_preview_as_one_undo() {
    let (mut document, top, east) = source_document();
    let snapshot = document.current();
    let target_package = exact_package(&snapshot, EXTRUSION, [[0.0, 0.0, 0.0], [10.0, 8.0, 20.0]]);
    let other_package = exact_package(
        &snapshot,
        OTHER_EXTRUSION,
        [[0.0, 0.0, 0.0], [6.0, 5.0, 12.0]],
    );
    let last_valid = ExactResultRegistry::accept(
        &snapshot,
        [
            Arc::new(ExactBodyPackage::from(target_package.clone())),
            Arc::new(ExactBodyPackage::from(other_package.clone())),
        ],
    )
    .unwrap();
    let last_valid_stamp = last_valid.contents_stamp();
    let last_valid_target_fingerprint = last_valid
        .get_body(&snapshot, DEFINITION, BODY)
        .unwrap()
        .unwrap()
        .result_key()
        .result_fingerprint
        .clone();
    let hover_target = top_target(top.clone());
    let selected_target = FaceIntentTarget::planar_face(
        DEFINITION,
        BODY,
        east,
        WorkplaneSupportHealth::Resolved,
        WorkplaneFrame::principal(PrincipalPlane::Yz).offset(10.0),
    );
    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: hover_target,
            ray_distance_mm: 1.0,
            visible: true,
        }],
        Some(selected_target),
    )
    .unwrap();
    let gesture = SmartPushPullGesture::begin(&snapshot, &intent, 0).unwrap();
    assert_eq!(gesture.body_id(), BODY);
    assert_eq!(gesture.producer_feature_id(), EXTRUSION);
    assert_eq!(
        gesture.target_reference().lineage_digest,
        top.lineage_digest
    );

    let selection_intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        Vec::new(),
        Some(top_target(top.clone())),
    )
    .unwrap();
    let selection_gesture = SmartPushPullGesture::begin(&snapshot, &selection_intent, 0).unwrap();
    let settings = PushPullSnapSettings::new(10.0, 3.0).unwrap();
    let selection_pointer = selection_gesture
        .preview_pointer(&snapshot, -9.4, &settings, &[])
        .unwrap();
    let selection_exact = selection_gesture
        .preview_exact(&snapshot, Dimension::from_decimal("-10").unwrap())
        .unwrap();
    assert_eq!(
        selection_exact.target().source,
        FaceIntentSource::StableSelection
    );
    assert_eq!(
        selection_pointer.resulting_extent(),
        selection_exact.resulting_extent(),
        "inward mouse and exact entry must resolve to the same extent"
    );

    let before = stamp(&document);
    let pointer = gesture
        .preview_pointer(&snapshot, 7.8, &settings, &[])
        .unwrap();
    assert_eq!(pointer.source(), PushPullPreviewSource::Pointer);
    assert_eq!(
        pointer.target().source,
        FaceIntentSource::Hover {
            pick_through_index: 0
        }
    );
    assert_eq!(pointer.signed_distance().millimetres(), 10.0);
    assert_eq!(pointer.resulting_extent().millimetres(), 30.0);
    assert_eq!(
        pointer.snap_feedback().unwrap().kind(),
        PushPullSnapKind::Grid
    );
    let outward_exact = gesture
        .preview_exact(&snapshot, Dimension::from_decimal("10").unwrap())
        .unwrap();
    assert_eq!(
        pointer.resulting_extent(),
        outward_exact.resulting_extent(),
        "outward mouse and exact entry must resolve to the same extent"
    );
    assert_eq!(stamp(&document), before, "pointer preview is observational");

    assert_eq!(selection_exact.source(), PushPullPreviewSource::Exact);
    assert_eq!(selection_exact.signed_distance().source_token(), "-10");
    assert_eq!(selection_exact.resulting_extent().millimetres(), 10.0);
    assert!(selection_exact.snap_feedback().is_none());
    let proposal = selection_gesture
        .plan_proposal(&document, &selection_exact)
        .unwrap();
    assert_eq!(proposal.batch().commands().len(), 2);
    assert_eq!(stamp(&document), before, "review is observational");

    let committed = document.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(
        committed.revision().dirty_features(),
        &BTreeSet::from([EXTRUSION])
    );
    assert_eq!(
        committed.revision().feature_states()[&OTHER_EXTRUSION],
        FeatureEvaluationState::Current
    );
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    let committed_snapshot = document.current();
    let committed_digest = committed_snapshot.canonical_digest();
    let definition = committed_snapshot.definition(DEFINITION).unwrap();
    assert_eq!(definition.active_body_id(), BODY);
    assert_eq!(
        definition
            .feature_body_ownership(EXTRUSION)
            .and_then(|ownership| ownership.output_body_id()),
        Some(BODY)
    );
    assert_eq!(
        definition
            .feature_body_ownership(OTHER_EXTRUSION)
            .and_then(|ownership| ownership.output_body_id()),
        Some(OTHER_BODY)
    );
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 10.0
    ));

    assert!(matches!(
        ExactResultRegistry::publish_body_results(
            &committed_snapshot,
            &last_valid,
            [Arc::new(ExactBodyPackage::from(target_package.clone()))]
        ),
        Err(ExactProductError::StaleResult)
    ));
    assert_eq!(last_valid.contents_stamp(), last_valid_stamp);
    assert_eq!(
        last_valid
            .get_body(&snapshot, DEFINITION, BODY)
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        last_valid_target_fingerprint
    );
    let carried = ExactResultRegistry::carried_forward(&committed_snapshot, &last_valid);
    assert!(
        carried
            .get_body(&committed_snapshot, DEFINITION, BODY)
            .unwrap()
            .is_none()
    );
    assert!(
        carried
            .get_body(&committed_snapshot, DEFINITION, OTHER_BODY)
            .unwrap()
            .is_some()
    );

    let refreshed = exact_package(
        &committed_snapshot,
        EXTRUSION,
        [[0.0, 0.0, 0.0], [10.0, 8.0, 10.0]],
    );
    let refreshed_top = refreshed.reference(ExactFaceRole::Top).unwrap();
    assert_eq!(refreshed_top.producer_feature_id, EXTRUSION);
    assert_eq!(refreshed_top.profile_feature_id, PROFILE);
    assert_eq!(refreshed_top.lineage_digest, top.lineage_digest);

    let saved = persistence::save(&committed_snapshot);
    let reopened = persistence::load(&saved).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened), saved);
    assert_eq!(
        reopened
            .definition(DEFINITION)
            .unwrap()
            .feature_body_ownership(EXTRUSION)
            .and_then(|ownership| ownership.output_body_id()),
        Some(BODY)
    );
    assert!(matches!(
        reopened.feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 10.0
    ));

    assert!(document.undo().is_some());
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    assert!(document.redo().is_some());
    assert_eq!(document.current().canonical_digest(), committed_digest);
}

#[test]
fn bounded_snap_toggles_and_feedback_are_deterministic() {
    let (document, top, _) = source_document();
    let snapshot = document.current();
    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: top_target(top),
            ray_distance_mm: 1.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    let gesture = SmartPushPullGesture::begin(&snapshot, &intent, 0).unwrap();
    let candidates = [
        PushPullSnapKind::Endpoint,
        PushPullSnapKind::Midpoint,
        PushPullSnapKind::Edge,
        PushPullSnapKind::Face,
        PushPullSnapKind::Coplanar,
    ]
    .map(|kind| PushPullSnapCandidate::new(kind, 6.0, format!("target:{kind:?}")).unwrap());
    let mut settings = PushPullSnapSettings::new(10.0, 1.0).unwrap();
    settings.set_enabled(PushPullSnapKind::Grid, false);

    for expected in [
        PushPullSnapKind::Endpoint,
        PushPullSnapKind::Midpoint,
        PushPullSnapKind::Edge,
        PushPullSnapKind::Face,
        PushPullSnapKind::Coplanar,
    ] {
        let preview = gesture
            .preview_pointer(&snapshot, 5.5, &settings, &candidates)
            .unwrap();
        assert_eq!(preview.snap_feedback().unwrap().kind(), expected);
        settings.set_enabled(expected, false);
    }
    let unsnapped = gesture
        .preview_pointer(&snapshot, 5.5, &settings, &candidates)
        .unwrap();
    assert!(unsnapped.snap_feedback().is_none());
    assert_eq!(unsnapped.signed_distance().millimetres(), 5.5);

    settings.set_enabled(PushPullSnapKind::Grid, true);
    let grid = gesture
        .preview_pointer(&snapshot, 9.4, &settings, &[])
        .unwrap();
    assert_eq!(grid.snap_feedback().unwrap().kind(), PushPullSnapKind::Grid);
    assert_eq!(grid.signed_distance().millimetres(), 10.0);

    settings.set_enabled(PushPullSnapKind::Grid, false);
    settings.set_enabled(PushPullSnapKind::Coplanar, true);
    let first = PushPullSnapCandidate::new(PushPullSnapKind::Coplanar, 8.0, "z-face").unwrap();
    let second = PushPullSnapCandidate::new(PushPullSnapKind::Coplanar, 8.0, "a-face").unwrap();
    for ordered in [
        vec![first.clone(), second.clone()],
        vec![second.clone(), first.clone()],
    ] {
        let preview = gesture
            .preview_pointer(&snapshot, 7.5, &settings, &ordered)
            .unwrap();
        let feedback = preview.snap_feedback().unwrap();
        assert_eq!(feedback.kind(), PushPullSnapKind::Coplanar);
        assert_eq!(feedback.stable_key(), "a-face");
        assert_eq!(feedback.raw_signed_distance_mm(), 7.5);
        assert_eq!(feedback.snapped_signed_distance_mm(), 8.0);
    }
}

#[test]
fn unsupported_invalid_stale_and_cancelled_gestures_are_observational() {
    let (mut document, top, east) = source_document();
    let snapshot = document.current();
    let before = stamp(&document);
    let last_valid = ExactResultRegistry::accept(
        &snapshot,
        [
            Arc::new(ExactBodyPackage::from(exact_package(
                &snapshot,
                EXTRUSION,
                [[0.0, 0.0, 0.0], [10.0, 8.0, 20.0]],
            ))),
            Arc::new(ExactBodyPackage::from(exact_package(
                &snapshot,
                OTHER_EXTRUSION,
                [[0.0, 0.0, 0.0], [6.0, 5.0, 12.0]],
            ))),
        ],
    )
    .unwrap();
    let last_valid_stamp = last_valid.contents_stamp();
    let datum_intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        Vec::new(),
        Some(FaceIntentTarget::datum(
            DEFINITION,
            BODY,
            PrincipalPlane::Xy,
        )),
    )
    .unwrap();
    assert!(matches!(
        SmartPushPullGesture::begin(&snapshot, &datum_intent, 0),
        Err(PushPullGestureError::UnsupportedFace)
    ));
    let side_intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: FaceIntentTarget::planar_face(
                DEFINITION,
                BODY,
                east,
                WorkplaneSupportHealth::Resolved,
                WorkplaneFrame::principal(PrincipalPlane::Yz).offset(10.0),
            ),
            ray_distance_mm: 1.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    assert!(matches!(
        SmartPushPullGesture::begin(&snapshot, &side_intent, 0),
        Err(PushPullGestureError::UnsupportedFace)
    ));
    for health in [
        WorkplaneSupportHealth::Ambiguous,
        WorkplaneSupportHealth::Lost,
    ] {
        let unresolved_intent = TransientFaceIntent::new(
            &snapshot,
            DEFINITION,
            vec![HoverFaceCandidate {
                target: FaceIntentTarget::planar_face(
                    DEFINITION,
                    BODY,
                    top.clone(),
                    health,
                    WorkplaneFrame::principal(PrincipalPlane::Xy).offset(20.0),
                ),
                ray_distance_mm: 1.0,
                visible: true,
            }],
            None,
        )
        .unwrap();
        assert!(matches!(
            SmartPushPullGesture::begin(&snapshot, &unresolved_intent, 0),
            Err(PushPullGestureError::FaceIntent(
                FaceIntentError::UnresolvedReference(actual)
            )) if actual == health
        ));
        assert_eq!(stamp(&document), before);
        assert_eq!(last_valid.contents_stamp(), last_valid_stamp);
    }

    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: top_target(top.clone()),
            ray_distance_mm: 1.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    let gesture = SmartPushPullGesture::begin(&snapshot, &intent, 0).unwrap();
    assert!(matches!(
        gesture.preview_exact(&snapshot, Dimension::from_decimal("-20").unwrap()),
        Err(PushPullGestureError::InvalidDistance)
    ));
    assert!(matches!(
        gesture.preview_pointer(
            &snapshot,
            f64::NAN,
            &PushPullSnapSettings::new(10.0, 1.0).unwrap(),
            &[]
        ),
        Err(PushPullGestureError::InvalidDistance)
    ));
    assert!(PushPullSnapSettings::new(0.0, 1.0).is_err());
    assert!(PushPullSnapCandidate::new(PushPullSnapKind::Grid, 2.0, "grid").is_err());
    let stale_confirmation = gesture
        .preview_exact(&snapshot, Dimension::from_decimal("4").unwrap())
        .unwrap();
    let cancelled = gesture
        .preview_exact(&snapshot, Dimension::from_decimal("3").unwrap())
        .unwrap();
    drop(cancelled);
    assert_eq!(
        stamp(&document),
        before,
        "cancel is dropping transient state"
    );

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameBody {
            definition_id: DEFINITION,
            id: BODY,
            name: "Changed".to_owned(),
        }]))
        .unwrap();
    let after_external_edit = stamp(&document);
    assert!(matches!(
        gesture.plan_proposal(&document, &stale_confirmation),
        Err(PushPullGestureError::StaleIntent)
    ));
    assert_eq!(stamp(&document), after_external_edit);
    assert_eq!(last_valid.contents_stamp(), last_valid_stamp);
}
