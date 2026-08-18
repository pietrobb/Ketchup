use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId,
    Dimension, DocumentStore, FeatureId, FeatureKind, ProposalCommitError, ProposalPrepareError,
    ProposalPrincipal,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactBodyView, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    ExactResultRegistry, build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationError, BodyHistoryMutationRequest, FeatureHistoryError,
    FeatureHistoryQuery, FeatureHistoryState, RollbackPreviewRequest,
    prepare_body_history_mutation, project_feature_history,
};
use ketchup_core::persistence;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const BASE_PROFILE: FeatureId = FeatureId(10);
const BASE_EXTRUSION: FeatureId = FeatureId(11);
const CUT_PROFILE: FeatureId = FeatureId(12);
const POCKET: FeatureId = FeatureId(13);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);

fn profile(min: f64, max: f64) -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[min, min], [max, min], [max, max], [min, max]],
    }
}

fn seed_history() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Verifier part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".to_owned(),
                kind: profile(0.0, 20.0),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("8").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Cut profile".to_owned(),
                kind: profile(4.0, 8.0),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: BASE_EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::from_decimal("3").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Unrelated body".to_owned(),
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
                kind: profile(0.0, 2.0),
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
        ]))
        .unwrap();
    document
}

fn mutation(mutation: BodyHistoryMutation) -> BodyHistoryMutationRequest {
    BodyHistoryMutationRequest {
        definition_id: DEFINITION,
        body_id: BodyId(1),
        mutation,
    }
}

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.revision_count(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

fn evidence<const N: usize>(
    request: &ExactFeatureChainRequest,
    roles: [ExactFaceRole; N],
    suffix: &str,
) -> [(ExactFaceRole, String, String); N] {
    roles.map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                request.document_id,
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{suffix}:{role:?}"),
        )
    })
}

fn package_for(
    snapshot: &ketchup_core::document::Snapshot,
    producer: FeatureId,
    suffix: &str,
) -> Arc<ExactBodyPackage> {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_producer(snapshot, DEFINITION, producer)
            .unwrap();
    let package = if request.pocket_depth_bits.is_some() {
        build_box_render_package(
            &request,
            format!("exact:{suffix}"),
            format!("result:{suffix}"),
            "verifier-backend".to_owned(),
            "verifier-tolerance".to_owned(),
            request.expected_bounds_mm(),
            evidence(
                &request,
                [
                    ExactFaceRole::Top,
                    ExactFaceRole::Bottom,
                    ExactFaceRole::East,
                    ExactFaceRole::PocketFloor,
                    ExactFaceRole::PocketWest,
                    ExactFaceRole::PocketEast,
                    ExactFaceRole::PocketSouth,
                    ExactFaceRole::PocketNorth,
                ],
                suffix,
            ),
        )
        .unwrap()
    } else {
        build_box_render_package(
            &request,
            format!("exact:{suffix}"),
            format!("result:{suffix}"),
            "verifier-backend".to_owned(),
            "verifier-tolerance".to_owned(),
            request.expected_bounds_mm(),
            evidence(
                &request,
                [
                    ExactFaceRole::Top,
                    ExactFaceRole::Bottom,
                    ExactFaceRole::East,
                ],
                suffix,
            ),
        )
        .unwrap()
    };
    Arc::new(ExactBodyPackage::from(package))
}

#[test]
fn independent_preview_suppress_resume_undo_redo_and_save_open_are_exact() {
    let mut document = seed_history();
    let before_stamp = stamp(&document);
    let before = document.current();
    let before_body = before
        .definition(DEFINITION)
        .unwrap()
        .body(BodyId(1))
        .unwrap()
        .clone();
    let before_features = [BASE_PROFILE, BASE_EXTRUSION, CUT_PROFILE, POCKET]
        .map(|id| before.feature(id).unwrap().clone());
    let before_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&before, DEFINITION, BodyId(1)).unwrap();
    let before_package = package_for(&before, POCKET, "pocket");
    assert_eq!(before_request.producer_feature_id(), POCKET);
    assert_eq!(
        before_request.expected_bounds_mm(),
        [[0.0, 0.0, 0.0], [20.0, 20.0, 8.0]]
    );
    assert_eq!(
        before_package.bounds_mm(),
        before_request.expected_bounds_mm()
    );
    assert_eq!(before_package.vertex_count(), 16);
    assert_eq!(before_package.triangle_count(), 28);

    let preview = project_feature_history(
        &before,
        &ExactResultRegistry::default(),
        DEFINITION,
        &FeatureHistoryQuery {
            rollback_preview: Some(RollbackPreviewRequest {
                body_id: BodyId(1),
                first_suppressed_feature_id: CUT_PROFILE,
            }),
            ..FeatureHistoryQuery::default()
        },
    )
    .unwrap();
    assert_eq!(
        preview.rollback_preview.unwrap().suppressed_feature_ids,
        vec![CUT_PROFILE, POCKET]
    );
    assert_eq!(
        preview.bodies[0]
            .features
            .iter()
            .map(|feature| feature.state)
            .collect::<Vec<_>>(),
        vec![
            FeatureHistoryState::Active,
            FeatureHistoryState::Active,
            FeatureHistoryState::RollbackSuppressed,
            FeatureHistoryState::RollbackSuppressed,
        ]
    );
    assert_eq!(stamp(&document), before_stamp);

    let suppression = prepare_body_history_mutation(
        &document,
        mutation(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(suppression.source_revision, before_stamp.0);
    assert_eq!(suppression.source_digest, before_stamp.1);
    assert_eq!(stamp(&document), before_stamp);
    document.commit_proposal(&suppression.proposal).unwrap();

    let suppressed = document.current();
    let suppressed_digest = suppressed.canonical_digest();
    let suppressed_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&suppressed, DEFINITION, BodyId(1))
            .unwrap();
    let suppressed_package = package_for(&suppressed, BASE_EXTRUSION, "base");
    assert_eq!(suppressed_request.producer_feature_id(), BASE_EXTRUSION);
    assert!(suppressed_request.pocket_depth_bits.is_none());
    assert_eq!(
        suppressed_request.expected_bounds_mm(),
        before_request.expected_bounds_mm()
    );
    assert_eq!(
        suppressed_package.bounds_mm(),
        before_request.expected_bounds_mm()
    );
    assert_eq!(suppressed_package.vertex_count(), 8);
    assert_eq!(suppressed_package.triangle_count(), 12);
    assert_eq!(
        suppressed.definition(DEFINITION).unwrap().body(BodyId(1)),
        Some(&before_body)
    );
    for (id, feature) in [BASE_PROFILE, BASE_EXTRUSION, CUT_PROFILE, POCKET]
        .into_iter()
        .zip(before_features.iter())
    {
        assert_eq!(suppressed.feature(id), Some(feature));
    }

    assert_eq!(document.undo().unwrap().canonical_digest(), before_stamp.1);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_body(
            &document.current(),
            DEFINITION,
            BodyId(1)
        )
        .unwrap()
        .producer_feature_id(),
        POCKET
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        suppressed_digest
    );

    let reopened_suppressed = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(
        reopened_suppressed.snapshot().canonical_digest(),
        suppressed_digest
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_body(
            &reopened_suppressed.snapshot(),
            DEFINITION,
            BodyId(1)
        )
        .unwrap()
        .producer_feature_id(),
        BASE_EXTRUSION
    );

    let resume = prepare_body_history_mutation(
        &document,
        mutation(BodyHistoryMutation::Resume),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let before_resume_undo = document.visible_undo_steps();
    document.commit_proposal(&resume.proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before_resume_undo + 1);
    let resumed = document.current();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_body(&resumed, DEFINITION, BodyId(1))
            .unwrap()
            .producer_feature_id(),
        POCKET
    );
    let reopened_resumed = persistence::load(&persistence::save(&resumed)).unwrap();
    assert_eq!(
        reopened_resumed.snapshot().canonical_digest(),
        resumed.canonical_digest()
    );
    assert_eq!(
        reopened_resumed
            .snapshot()
            .suppressed_feature_ids(DEFINITION, BodyId(1)),
        None
    );
}

#[test]
fn affected_only_carry_forward_preserves_unrelated_and_stable_base_lineage_on_failure() {
    let mut document = seed_history();
    let before = document.current();
    let base_package = package_for(&before, BASE_EXTRUSION, "base-stable");
    let base_top = base_package
        .references()
        .iter()
        .find(|reference| reference.role() == Some(ExactFaceRole::Top))
        .unwrap()
        .clone();
    let pocket_package = package_for(&before, POCKET, "pocket-current");
    let tool_package = package_for(&before, TOOL_EXTRUSION, "tool-stable");
    let tool_bounds = tool_package.bounds_mm();
    let tool_top = tool_package
        .references()
        .iter()
        .find(|reference| reference.role() == Some(ExactFaceRole::Top))
        .unwrap()
        .clone();
    let registry = ExactResultRegistry::accept(
        &before,
        [base_package, pocket_package.clone(), tool_package],
    )
    .unwrap();

    let suppression = prepare_body_history_mutation(
        &document,
        mutation(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let revision = document.commit_proposal(&suppression.proposal).unwrap();
    assert_eq!(
        revision.dirty_features(),
        &[CUT_PROFILE, POCKET].into_iter().collect()
    );
    let suppressed = document.current();
    let carried = ExactResultRegistry::carried_forward(&suppressed, &registry);
    assert_eq!(carried.len(), 2);

    let rolled_back = carried
        .get_body(&suppressed, DEFINITION, BodyId(1))
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back.producer_feature_id(), BASE_EXTRUSION);
    let rolled_back_top = rolled_back
        .references()
        .iter()
        .find(|reference| reference.role() == Some(ExactFaceRole::Top))
        .unwrap();
    assert_eq!(rolled_back_top.lineage_digest, base_top.lineage_digest);
    assert_eq!(
        rolled_back_top.source_element_id,
        base_top.source_element_id
    );

    let unrelated = carried
        .get_body(&suppressed, DEFINITION, BodyId(2))
        .unwrap()
        .unwrap();
    assert_eq!(unrelated.bounds_mm(), tool_bounds);
    let unrelated_top = unrelated
        .references()
        .iter()
        .find(|reference| reference.role() == Some(ExactFaceRole::Top))
        .unwrap();
    assert_eq!(unrelated_top.lineage_digest, tool_top.lineage_digest);
    assert_eq!(unrelated_top.source_element_id, tool_top.source_element_id);

    let before_failure = stamp(&document);
    let carried_stamp = carried.contents_stamp();
    assert!(matches!(
        ExactResultRegistry::publish_body_results(&suppressed, &carried, [pocket_package]),
        Err(ExactProductError::StaleResult)
    ));
    assert_eq!(stamp(&document), before_failure);
    assert_eq!(carried.contents_stamp(), carried_stamp);
    assert_eq!(
        carried
            .get_body(&suppressed, DEFINITION, BodyId(2))
            .unwrap()
            .unwrap()
            .bounds_mm(),
        tool_bounds
    );
}

#[test]
fn lost_and_ambiguous_rollback_references_are_observational() {
    let mut document = seed_history();
    let before = document.current();
    let base = package_for(&before, BASE_EXTRUSION, "base-reference");
    let reference = base
        .references()
        .iter()
        .find(|candidate| candidate.role() == Some(ExactFaceRole::Top))
        .unwrap()
        .clone();
    let suppression = prepare_body_history_mutation(
        &document,
        mutation(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document.commit_proposal(&suppression.proposal).unwrap();
    let suppressed = document.current();
    let unchanged = stamp(&document);
    let query = FeatureHistoryQuery {
        selected_feature_id: Some(BASE_EXTRUSION),
        selected_subshape: Some(reference.clone()),
        rollback_preview: None,
    };

    assert_eq!(
        project_feature_history(
            &suppressed,
            &ExactResultRegistry::default(),
            DEFINITION,
            &query
        ),
        Err(FeatureHistoryError::SubshapeLost)
    );
    assert_eq!(stamp(&document), unchanged);

    let current_base = package_for(&suppressed, BASE_EXTRUSION, "base-current");
    let mut alternate_package = (*current_base).clone();
    let ExactBodyPackage::Rectangle(alternate_render) = &mut alternate_package else {
        panic!("expected rectangle package");
    };
    alternate_render.identity.backend.push_str("-alternate");
    for candidate in &mut alternate_render.references {
        candidate.backend = alternate_render.identity.backend.clone();
    }
    let ambiguous =
        ExactResultRegistry::accept(&suppressed, [current_base, Arc::new(alternate_package)])
            .unwrap();
    assert_eq!(
        project_feature_history(&suppressed, &ambiguous, DEFINITION, &query),
        Err(FeatureHistoryError::SubshapeAmbiguous(2))
    );
    assert_eq!(stamp(&document), unchanged);
}

#[test]
fn invalid_non_suffix_duplicate_cross_body_cycle_noop_and_stale_requests_are_atomic() {
    let mut document = seed_history();
    let before = stamp(&document);
    for invalid in [
        vec![BASE_EXTRUSION, POCKET],
        vec![CUT_PROFILE, CUT_PROFILE, POCKET],
        vec![TOOL_PROFILE, TOOL_EXTRUSION],
    ] {
        assert!(matches!(
            document.preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetBodyFeatureSuppression {
                    definition_id: DEFINITION,
                    body_id: BodyId(1),
                    suppressed_feature_ids: invalid,
                },
            ])),
            Err(CanonicalError::InvalidFeatureSuppression(
                DEFINITION,
                BodyId(1)
            ))
        ));
        assert_eq!(stamp(&document), before);
    }
    assert_eq!(
        prepare_body_history_mutation(
            &document,
            mutation(BodyHistoryMutation::Resume),
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyHistoryMutationError::NoSuppressedSuffix(BodyId(1)))
    );
    assert_eq!(stamp(&document), before);

    let stale = prepare_body_history_mutation(
        &document,
        mutation(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyVisibility {
                definition_id: DEFINITION,
                id: BodyId(2),
                visible: false,
            },
        ]))
        .unwrap();
    let before_stale = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale.proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale);

    let before_cycle = stamp(&document);
    assert_eq!(
        document.prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(30),
                definition_id: DEFINITION,
                name: "Tool into base".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: POCKET,
                    tool: TOOL_EXTRUSION,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(31),
                definition_id: DEFINITION,
                name: "Base into tool".to_owned(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: TOOL_EXTRUSION,
                    tool: FeatureId(30),
                },
            },
        ])),
        Err(ProposalPrepareError::Canonical(
            CanonicalError::BodyDependencyCycle(DEFINITION)
        ))
    );
    assert_eq!(stamp(&document), before_cycle);
}

#[test]
fn dependency_crossing_body_boundary_refuses_suppression_without_losing_outputs() {
    let mut document = seed_history();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(30),
            definition_id: DEFINITION,
            name: "Union".to_owned(),
            kind: FeatureKind::Boolean {
                operation: BooleanOperation::Union,
                target: POCKET,
                tool: TOOL_EXTRUSION,
            },
        }]))
        .unwrap();
    let before = stamp(&document);
    let tool = document.current().feature(TOOL_EXTRUSION).unwrap().clone();
    let union = document.current().feature(FeatureId(30)).unwrap().clone();

    assert_eq!(
        prepare_body_history_mutation(
            &document,
            BodyHistoryMutationRequest {
                definition_id: DEFINITION,
                body_id: BodyId(2),
                mutation: BodyHistoryMutation::SuppressFrom(TOOL_EXTRUSION),
            },
            ProposalPrincipal::LocalAssistant,
        ),
        Err(BodyHistoryMutationError::History(
            FeatureHistoryError::RollbackNotDependencyClosed(TOOL_EXTRUSION)
        ))
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(document.current().feature(TOOL_EXTRUSION), Some(&tool));
    assert_eq!(document.current().feature(FeatureId(30)), Some(&union));
}
