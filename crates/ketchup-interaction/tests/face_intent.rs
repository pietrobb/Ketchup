use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind,
};
use ketchup_core::exact_product::{
    BodySubshapeRef, ExactFaceRole, ExactFeatureChainRequest, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::sketch::{PrincipalPlane, WorkplaneFrame, WorkplaneSupportHealth};
use ketchup_core::{persistence, state_view::encode_semantic_state};
use ketchup_interaction::face_intent::{
    FaceIntentError, FaceIntentSource, FaceIntentTarget, HoverFaceCandidate, TransientFaceIntent,
};

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE_ONE: FeatureId = FeatureId(10);
const EXTRUSION_ONE: FeatureId = FeatureId(11);
const PROFILE_TWO: FeatureId = FeatureId(12);
const EXTRUSION_TWO: FeatureId = FeatureId(13);

fn seed() -> (DocumentStore, BodySubshapeRef, BodySubshapeRef) {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE_ONE,
                definition_id: DEFINITION,
                name: "First rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION_ONE,
                definition_id: DEFINITION,
                name: "First extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE_ONE,
                    height: Dimension::from_decimal("5").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Second body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE_TWO,
                definition_id: DEFINITION,
                name: "Second rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 4.0], [0.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION_TWO,
                definition_id: DEFINITION,
                name: "Second extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE_TWO,
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
        ]))
        .unwrap();

    let first = top_reference(&store, EXTRUSION_ONE, [[0.0, 0.0, 0.0], [20.0, 10.0, 5.0]]);
    let second = top_reference(&store, EXTRUSION_TWO, [[0.0, 0.0, 0.0], [8.0, 4.0, 2.0]]);
    store
        .register_exact_reference_evidence(first.clone())
        .unwrap();
    store
        .register_exact_reference_evidence(second.clone())
        .unwrap();
    (store, first, second)
}

fn top_reference(
    store: &DocumentStore,
    producer: FeatureId,
    bounds_mm: [[f64; 3]; 2],
) -> BodySubshapeRef {
    let snapshot = store.current();
    let request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&snapshot, DEFINITION, producer)
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
        format!("exact-input-{}", producer.0),
        format!("result-{}", producer.0),
        "occt".to_owned(),
        "r0".to_owned(),
        bounds_mm,
        evidence,
    )
    .unwrap()
    .reference(ExactFaceRole::Top)
    .unwrap()
    .clone()
}

fn face_target(body_id: BodyId, reference: BodySubshapeRef, height_mm: f64) -> FaceIntentTarget {
    FaceIntentTarget::planar_face(
        DEFINITION,
        body_id,
        reference,
        WorkplaneSupportHealth::Resolved,
        WorkplaneFrame::principal(PrincipalPlane::Xy).offset(height_mm),
    )
}

fn stamp(store: &DocumentStore) -> (u64, String, usize, usize) {
    (
        store.current().revision_id(),
        store.current().canonical_digest(),
        store.visible_undo_steps(),
        store.visible_redo_steps(),
    )
}

#[test]
fn hover_wins_over_selection_and_pick_through_order_is_permutation_stable() {
    let (store, first_reference, second_reference) = seed();
    let snapshot = store.current();
    let first = face_target(BodyId(1), first_reference, 5.0);
    let second = face_target(BodyId(2), second_reference, 2.0);
    let selection = FaceIntentTarget::datum(DEFINITION, BodyId(2), PrincipalPlane::Yz);
    let candidates = vec![
        HoverFaceCandidate {
            target: second.clone(),
            ray_distance_mm: 8.0,
            visible: true,
        },
        HoverFaceCandidate {
            target: first.clone(),
            ray_distance_mm: 3.0,
            visible: true,
        },
    ];
    let reversed = candidates.iter().cloned().rev().collect();
    let intent =
        TransientFaceIntent::new(&snapshot, DEFINITION, candidates, Some(selection.clone()))
            .unwrap();
    let permuted =
        TransientFaceIntent::new(&snapshot, DEFINITION, reversed, Some(selection)).unwrap();

    let ordered = |intent: &TransientFaceIntent| {
        intent
            .ordered_hover_candidates()
            .iter()
            .map(|candidate| candidate.target.body_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ordered(&intent), vec![BodyId(1), BodyId(2)]);
    assert_eq!(ordered(&intent), ordered(&permuted));
    let tied = intent
        .ordered_hover_candidates()
        .iter()
        .cloned()
        .map(|mut candidate| {
            candidate.ray_distance_mm = 4.0;
            candidate
        })
        .collect::<Vec<_>>();
    let tied_reversed = tied.iter().cloned().rev().collect();
    let tied_intent = TransientFaceIntent::new(&snapshot, DEFINITION, tied, None).unwrap();
    let tied_permuted =
        TransientFaceIntent::new(&snapshot, DEFINITION, tied_reversed, None).unwrap();
    assert_eq!(ordered(&tied_intent), vec![BodyId(1), BodyId(2)]);
    assert_eq!(ordered(&tied_intent), ordered(&tied_permuted));
    assert_eq!(
        intent.resolve(&snapshot, 0).unwrap(),
        ketchup_interaction::face_intent::ResolvedFaceIntent {
            source: FaceIntentSource::Hover {
                pick_through_index: 0,
            },
            target: first,
        }
    );
    assert_eq!(
        intent.resolve(&snapshot, 1).unwrap().target,
        second,
        "deliberate pick-through must select the next ordered candidate"
    );
    assert_eq!(
        intent.resolve(&snapshot, 3).unwrap().target.body_id,
        BodyId(2)
    );
}

#[test]
fn stable_selection_supplies_explicit_datum_and_body_context_without_mutation() {
    let (store, _, _) = seed();
    let snapshot = store.current();
    let before = stamp(&store);
    let selected = FaceIntentTarget::datum(DEFINITION, BodyId(2), PrincipalPlane::Xz);
    let intent =
        TransientFaceIntent::new(&snapshot, DEFINITION, Vec::new(), Some(selected.clone()))
            .unwrap();
    let resolved = intent.resolve(&snapshot, 99).unwrap();

    assert_eq!(resolved.source, FaceIntentSource::StableSelection);
    assert_eq!(resolved.target, selected);
    assert_eq!(
        resolved.target.workplane.frame(),
        WorkplaneFrame::principal(PrincipalPlane::Xz)
    );
    assert_eq!(stamp(&store), before);
}

#[test]
fn hidden_unresolved_cross_context_and_body_mismatch_targets_fail_closed() {
    let (store, first_reference, _) = seed();
    let snapshot = store.current();
    let before = stamp(&store);
    let before_state = encode_semantic_state(&snapshot);
    let before_bytes = persistence::save(&snapshot);
    let first = face_target(BodyId(1), first_reference.clone(), 5.0);
    let resolve = |target: FaceIntentTarget, visible| {
        TransientFaceIntent::new(
            &snapshot,
            DEFINITION,
            vec![HoverFaceCandidate {
                target,
                ray_distance_mm: 1.0,
                visible,
            }],
            None,
        )
        .unwrap()
        .resolve(&snapshot, 0)
    };

    assert_eq!(
        resolve(first.clone(), false),
        Err(FaceIntentError::HiddenTarget)
    );
    for health in [
        WorkplaneSupportHealth::Ambiguous,
        WorkplaneSupportHealth::Lost,
        WorkplaneSupportHealth::Stale,
    ] {
        let unresolved = FaceIntentTarget::planar_face(
            DEFINITION,
            BodyId(1),
            first_reference.clone(),
            health,
            WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
        );
        assert_eq!(
            resolve(unresolved, true),
            Err(FaceIntentError::UnresolvedReference(health))
        );
    }
    let mut cross_context = first.clone();
    cross_context.definition_id = DefinitionId(99);
    assert_eq!(
        resolve(cross_context, true),
        Err(FaceIntentError::CrossContext)
    );
    let mut wrong_body = first;
    wrong_body.body_id = BodyId(2);
    assert_eq!(
        resolve(wrong_body, true),
        Err(FaceIntentError::ReferenceBodyMismatch)
    );
    assert_eq!(stamp(&store), before);
    let after_state = encode_semantic_state(&store.current());
    assert_eq!(after_state.complete_v1(), before_state.complete_v1());
    assert_eq!(after_state.agent_v1(), before_state.agent_v1());
    assert_eq!(persistence::save(&store.current()), before_bytes);
}

#[test]
fn stale_intent_and_invalid_hover_distance_never_fall_back_or_mutate() {
    let (mut store, first_reference, _) = seed();
    let snapshot = store.current();
    let selection = FaceIntentTarget::datum(DEFINITION, BodyId(2), PrincipalPlane::Xy);
    let hidden_hover = HoverFaceCandidate {
        target: face_target(BodyId(1), first_reference, 5.0),
        ray_distance_mm: 1.0,
        visible: false,
    };
    let intent =
        TransientFaceIntent::new(&snapshot, DEFINITION, vec![hidden_hover], Some(selection))
            .unwrap();
    assert_eq!(
        intent.resolve(&snapshot, 0),
        Err(FaceIntentError::HiddenTarget),
        "an invalid current hover must not silently fall back to selection"
    );
    assert!(matches!(
        TransientFaceIntent::new(
            &snapshot,
            DEFINITION,
            vec![HoverFaceCandidate {
                target: FaceIntentTarget::datum(DEFINITION, BodyId(1), PrincipalPlane::Xy),
                ray_distance_mm: f64::NAN,
                visible: true,
            }],
            None,
        ),
        Err(FaceIntentError::InvalidRayDistance)
    ));

    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Renamed body".to_owned(),
        }]))
        .unwrap();
    let current = store.current();
    let before_stale_resolve = stamp(&store);
    assert_eq!(
        intent.resolve(&current, 0),
        Err(FaceIntentError::StaleSnapshot)
    );
    assert_eq!(stamp(&store), before_stale_resolve);
}

#[test]
fn transient_intent_is_state_view_and_save_open_observational_and_cancel_safe() {
    let (store, first_reference, _) = seed();
    let snapshot = store.current();
    let before = stamp(&store);
    let before_state = encode_semantic_state(&snapshot);
    let before_bytes = persistence::save(&snapshot);
    let target = face_target(BodyId(1), first_reference, 5.0);

    {
        let intent = TransientFaceIntent::new(
            &snapshot,
            DEFINITION,
            vec![HoverFaceCandidate {
                target: target.clone(),
                ray_distance_mm: 2.0,
                visible: true,
            }],
            None,
        )
        .unwrap();
        assert_eq!(intent.resolve(&snapshot, 0).unwrap().target, target);
    }

    assert_eq!(stamp(&store), before, "dropping the intent is cancel");
    let after_state = encode_semantic_state(&store.current());
    assert_eq!(after_state.complete_v1(), before_state.complete_v1());
    assert_eq!(after_state.agent_v1(), before_state.agent_v1());
    assert_eq!(persistence::save(&store.current()), before_bytes);

    let reopened = persistence::load(&before_bytes).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(
        reopened_snapshot.canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(persistence::save(&reopened_snapshot), before_bytes);
    let reopened_state = encode_semantic_state(&reopened_snapshot);
    assert_eq!(reopened_state.complete_v1(), before_state.complete_v1());
    assert_eq!(reopened_state.agent_v1(), before_state.agent_v1());
    let reopened_intent = TransientFaceIntent::new(
        &reopened_snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: target.clone(),
            ray_distance_mm: 2.0,
            visible: true,
        }],
        None,
    )
    .unwrap();
    assert_eq!(
        reopened_intent
            .resolve(&reopened_snapshot, 0)
            .unwrap()
            .target,
        target
    );
}

#[test]
fn canonically_hidden_body_is_rejected_without_additional_mutation() {
    let (mut store, _, _) = seed();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyVisibility {
                definition_id: DEFINITION,
                id: BodyId(2),
                visible: false,
            },
        ]))
        .unwrap();
    let snapshot = store.current();
    let before = stamp(&store);
    let before_state = encode_semantic_state(&snapshot);
    let before_bytes = persistence::save(&snapshot);
    let intent = TransientFaceIntent::new(
        &snapshot,
        DEFINITION,
        vec![HoverFaceCandidate {
            target: FaceIntentTarget::datum(DEFINITION, BodyId(2), PrincipalPlane::Yz),
            ray_distance_mm: 1.0,
            visible: true,
        }],
        None,
    )
    .unwrap();

    assert_eq!(
        intent.resolve(&snapshot, 0),
        Err(FaceIntentError::HiddenTarget)
    );
    assert_eq!(stamp(&store), before);
    let after_state = encode_semantic_state(&store.current());
    assert_eq!(after_state.complete_v1(), before_state.complete_v1());
    assert_eq!(after_state.agent_v1(), before_state.agent_v1());
    assert_eq!(persistence::save(&store.current()), before_bytes);
}
