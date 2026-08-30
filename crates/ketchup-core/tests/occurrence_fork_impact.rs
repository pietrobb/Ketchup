use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CanonicalError, CollectionId, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, GroupId, OccurrenceId, ProposalPrincipal,
    StableFaceRole, Transform,
};
use ketchup_core::drawing::{
    DrawingError, DrawingSheet, DrawingSheetId, DrawingSource, OrthographicViewKind,
    project_orthographic_drawing,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactRenderPackage,
    ExactResultRegistry, build_box_render_package, canonical_reference_lineage_digest,
    exact_model_stl_export,
};
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationRequest, BodyParameterEditRequest, ExactParameterEdit,
    ExactParameterEditTarget,
};
use ketchup_core::persistence;
use ketchup_core::shared_change::{
    OccurrenceForkBodyLineage, OccurrenceForkChangeRequest, OccurrenceForkFeatureLineage,
    OccurrenceForkImpactError, OccurrenceForkPropagationError, SharedChangeExportEligibility,
    SharedChangeExportFormat, commit_occurrence_fork_change, project_occurrence_fork_impact,
};
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const OTHER_DEFINITION: DefinitionId = DefinitionId(2);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const FIRST: OccurrenceId = OccurrenceId(20);
const SECOND: OccurrenceId = OccurrenceId(21);
const THIRD: OccurrenceId = OccurrenceId(22);
const OTHER: OccurrenceId = OccurrenceId(30);
const MATE: AssemblyMateId = AssemblyMateId(40);
const AXIAL_MATE: AssemblyMateId = AssemblyMateId(41);
const SHEET: DrawingSheetId = DrawingSheetId(50);
const COLLECTION: CollectionId = CollectionId(60);
const GROUP: GroupId = GroupId(70);

#[derive(Debug, Eq, PartialEq)]
struct Stamp {
    revision: u64,
    digest: String,
    revisions: usize,
    undo: usize,
    redo: usize,
}

fn stamp(document: &DocumentStore) -> Stamp {
    Stamp {
        revision: document.current().revision_id(),
        digest: document.current().canonical_digest(),
        revisions: document.revision_count(),
        undo: document.visible_undo_steps(),
        redo: document.visible_redo_steps(),
    }
}

fn exact_package(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> ExactRenderPackage {
    exact_package_for(snapshot, DEFINITION, fingerprint)
}

fn exact_package_for(
    snapshot: &ketchup_core::document::Snapshot,
    definition_id: DefinitionId,
    fingerprint: &str,
) -> ExactRenderPackage {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_body(snapshot, definition_id, BodyId(1))
            .unwrap();
    let evidence = |role: ExactFaceRole| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:{fingerprint}"),
        )
    };
    if request.pocket_depth_bits.is_some() {
        build_box_render_package(
            &request,
            format!("exact-input:{fingerprint}"),
            fingerprint.to_owned(),
            "occt".into(),
            "r0".into(),
            request.expected_bounds_mm(),
            [
                ExactFaceRole::Top,
                ExactFaceRole::Bottom,
                ExactFaceRole::East,
                ExactFaceRole::PocketFloor,
                ExactFaceRole::PocketWest,
                ExactFaceRole::PocketEast,
                ExactFaceRole::PocketSouth,
                ExactFaceRole::PocketNorth,
            ]
            .map(evidence),
        )
        .unwrap()
    } else {
        build_box_render_package(
            &request,
            format!("exact-input:{fingerprint}"),
            fingerprint.to_owned(),
            "occt".into(),
            "r0".into(),
            request.expected_bounds_mm(),
            [
                ExactFaceRole::Top,
                ExactFaceRole::Bottom,
                ExactFaceRole::East,
            ]
            .map(evidence),
        )
        .unwrap()
    }
}

fn registry(snapshot: &ketchup_core::document::Snapshot, fingerprint: &str) -> ExactResultRegistry {
    ExactResultRegistry::accept(
        snapshot,
        [Arc::new(ExactBodyPackage::from(exact_package(
            snapshot,
            fingerprint,
        )))],
    )
    .unwrap()
}

fn seed(reverse_occurrences: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let first = CanonicalCommand::CreateOccurrence {
        id: FIRST,
        definition_id: DEFINITION,
        name: "Selected reuse".into(),
        transform: Transform::identity(),
        parent: None,
        tag: None,
        visible: true,
    };
    let second = CanonicalCommand::CreateOccurrence {
        id: SECOND,
        definition_id: DEFINITION,
        name: "Sibling reuse".into(),
        transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    };
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Shared part".into(),
        },
        CanonicalCommand::CreateDefinition {
            id: OTHER_DEFINITION,
            name: "Unrelated".into(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: EXTRUSION,
            definition_id: DEFINITION,
            name: "Extrusion".into(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
    ];
    commands.extend(if reverse_occurrences {
        vec![second, first]
    } else {
        vec![first, second]
    });
    commands.push(CanonicalCommand::CreateOccurrence {
        id: OTHER,
        definition_id: OTHER_DEFINITION,
        name: "Unrelated occurrence".into(),
        transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let evidence = exact_package(&document.current(), "mate-evidence");
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::resolved(
                    FIRST,
                    evidence.reference(ExactFaceRole::Top).unwrap().clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    SECOND,
                    evidence.reference(ExactFaceRole::Bottom).unwrap().clone(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateDrawingSheet(
                DrawingSheet::new(SHEET, "Reused part", DrawingSource::Definition(DEFINITION))
                    .unwrap(),
            ),
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Assembly selection".into(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![FIRST, SECOND, OTHER],
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SECOND,
                grounded: true,
            },
            CanonicalCommand::UpdateDrawingSheet(
                DrawingSheet::new(
                    SHEET,
                    "Rigid reused assembly",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![FIRST, SECOND],
                    },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    document
}

fn edit_request(
    snapshot: &ketchup_core::document::Snapshot,
    occurrence_id: OccurrenceId,
) -> OccurrenceForkChangeRequest {
    OccurrenceForkChangeRequest::exact_parameter_edit(
        snapshot,
        occurrence_id,
        "Selected reuse unique",
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
                dimension: Dimension::from_decimal("15").unwrap(),
            }],
        },
    )
}

#[test]
fn fork_impact_is_deterministic_complete_and_non_mutating() {
    let first = seed(false);
    let second = seed(true);
    let first_results = registry(&first.current(), "last-valid");
    let second_results = registry(&second.current(), "last-valid");
    let first_before = stamp(&first);
    let second_before = stamp(&second);
    let first_results_before = first_results.contents_stamp();
    let saved_before = persistence::save(&first.current());

    let first_impact = project_occurrence_fork_impact(
        &first,
        &first_results,
        edit_request(&first.current(), FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let second_impact = project_occurrence_fork_impact(
        &second,
        &second_results,
        edit_request(&second.current(), FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    assert_eq!(first_impact.body_lineage, second_impact.body_lineage);
    assert_eq!(first_impact.feature_lineage, second_impact.feature_lineage);
    assert_eq!(
        first_impact.affected_fork_body_ids,
        second_impact.affected_fork_body_ids
    );
    assert_eq!(
        first_impact.affected_fork_feature_ids,
        second_impact.affected_fork_feature_ids
    );
    assert_eq!(
        first_impact.unchanged_sibling_occurrences,
        second_impact.unchanged_sibling_occurrences
    );
    assert_eq!(
        first_impact.collection_dependencies,
        second_impact.collection_dependencies
    );
    assert_eq!(first_impact.drawing_views, second_impact.drawing_views);
    assert_eq!(first_impact.exports, second_impact.exports);
    assert_eq!(
        first_impact
            .subshape_lineage
            .iter()
            .map(|lineage| (
                lineage.source_profile_feature_id,
                lineage.source_producer_feature_id,
                lineage.fork_profile_feature_id,
                lineage.fork_producer_feature_id,
                &lineage.semantic_role,
                &lineage.source_element_id,
                &lineage.expected_type,
            ))
            .collect::<Vec<_>>(),
        second_impact
            .subshape_lineage
            .iter()
            .map(|lineage| (
                lineage.source_profile_feature_id,
                lineage.source_producer_feature_id,
                lineage.fork_profile_feature_id,
                lineage.fork_producer_feature_id,
                &lineage.semantic_role,
                &lineage.source_element_id,
                &lineage.expected_type,
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first_impact
            .mate_references
            .iter()
            .map(|reference| (
                reference.mate_id,
                reference.occurrence_id,
                reference.source_producer_feature_id,
                reference.fork_producer_feature_id,
            ))
            .collect::<Vec<_>>(),
        second_impact
            .mate_references
            .iter()
            .map(|reference| (
                reference.mate_id,
                reference.occurrence_id,
                reference.source_producer_feature_id,
                reference.fork_producer_feature_id,
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(first_impact.selected_occurrence_id, FIRST);
    assert_eq!(first_impact.source_definition_id, DEFINITION);
    assert_eq!(first_impact.fork_definition_id, DefinitionId(3));
    assert_eq!(
        first_impact.body_lineage,
        vec![OccurrenceForkBodyLineage {
            source_definition_id: DEFINITION,
            source_body_id: BodyId(1),
            fork_definition_id: DefinitionId(3),
            fork_body_id: BodyId(1),
        }]
    );
    assert_eq!(
        first_impact.feature_lineage,
        vec![
            OccurrenceForkFeatureLineage {
                source_feature_id: PROFILE,
                fork_feature_id: FeatureId(12),
            },
            OccurrenceForkFeatureLineage {
                source_feature_id: EXTRUSION,
                fork_feature_id: FeatureId(13),
            },
        ]
    );
    assert_eq!(first_impact.affected_fork_body_ids, vec![BodyId(1)]);
    assert_eq!(first_impact.affected_fork_feature_ids, vec![FeatureId(13)]);
    assert_eq!(first_impact.unchanged_source_body_ids, vec![BodyId(1)]);
    assert_eq!(
        first_impact.unchanged_definition_ids,
        vec![DEFINITION, OTHER_DEFINITION]
    );
    assert_eq!(
        first_impact
            .unchanged_sibling_occurrences
            .iter()
            .map(|occurrence| occurrence.occurrence_id)
            .collect::<Vec<_>>(),
        vec![SECOND]
    );
    assert!(!first_impact.subshape_lineage.is_empty());
    assert!(first_impact.subshape_lineage.iter().all(|lineage| {
        lineage.source_definition_id == DEFINITION
            && lineage.fork_definition_id == DefinitionId(3)
            && lineage.source_producer_feature_id == EXTRUSION
            && lineage.fork_producer_feature_id == FeatureId(13)
            && lineage.source_lineage_digest != lineage.fork_lineage_digest
    }));
    assert_eq!(first_impact.exact_jobs.len(), 1);
    assert_eq!(first_impact.exact_jobs[0].definition_id, DefinitionId(3));
    assert_eq!(first_impact.exact_jobs[0].body_id, BodyId(1));
    assert_eq!(
        first_impact.exact_jobs[0].producer_feature_id,
        FeatureId(13)
    );
    assert_eq!(
        first_impact.exact_jobs[0].last_valid_result_fingerprint,
        "last-valid"
    );
    assert_eq!(first_impact.mate_references.len(), 1);
    assert_eq!(first_impact.mate_references[0].mate_id, MATE);
    assert_eq!(first_impact.mate_references[0].occurrence_id, FIRST);
    assert_eq!(first_impact.collection_dependencies.len(), 1);
    assert_eq!(
        first_impact.collection_dependencies[0].collection_id,
        COLLECTION
    );
    assert_eq!(
        first_impact.collection_dependencies[0].occurrence_ids_before,
        vec![FIRST, SECOND, OTHER]
    );
    assert_eq!(
        first_impact.collection_dependencies[0].occurrence_ids_after,
        vec![FIRST, SECOND, OTHER]
    );
    assert_eq!(
        first_impact
            .drawing_views
            .iter()
            .map(|view| view.view)
            .collect::<Vec<_>>(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ]
    );
    assert_eq!(
        first_impact
            .exports
            .iter()
            .map(|export| (
                export.format,
                export.eligibility,
                export.occurrence_paths.clone(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                SharedChangeExportFormat::Step,
                SharedChangeExportEligibility::PendingExactRecompute,
                vec![ketchup_core::document::InstancePath::root(FIRST)],
            ),
            (
                SharedChangeExportFormat::Stl,
                SharedChangeExportEligibility::PendingExactRecompute,
                vec![ketchup_core::document::InstancePath::root(FIRST)],
            ),
        ]
    );

    let source_definition = first.current().definition(DEFINITION).unwrap().clone();
    let candidate = first.preview_batch(first_impact.proposal.batch()).unwrap();
    assert_eq!(candidate.definition(DEFINITION), Some(&source_definition));
    assert_eq!(
        candidate.occurrence(FIRST).unwrap().definition_id(),
        DefinitionId(3)
    );
    assert_eq!(
        candidate.occurrence(SECOND).unwrap().definition_id(),
        DEFINITION
    );
    assert!(matches!(
        candidate.feature(FeatureId(13)).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 15.0
    ));
    assert_ne!(first_impact.candidate_digest, first_impact.source_digest);
    assert_eq!(stamp(&first), first_before);
    assert_eq!(stamp(&second), second_before);
    assert_eq!(first_results.contents_stamp(), first_results_before);
    assert_eq!(persistence::save(&first.current()), saved_before);

    let reopened = persistence::load(&saved_before)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_impact = project_occurrence_fork_impact(
        &reopened,
        &first_results,
        edit_request(&reopened.current(), FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(reopened_impact, first_impact);
}

#[test]
fn independent_atomic_fork_verifier_covers_parity_cancel_outputs_and_round_trip() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source, "source-last-valid");
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();
    let saved_before = persistence::save(&source);
    let source_package = Arc::clone(
        exact_results
            .get_body(&source, DEFINITION, BodyId(1))
            .unwrap()
            .unwrap(),
    );

    let manual = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let assistant = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(
        manual.proposal.command_digest(),
        assistant.proposal.command_digest()
    );
    assert_eq!(manual.candidate_digest, assistant.candidate_digest);
    assert_eq!(manual.body_lineage, assistant.body_lineage);
    assert_eq!(manual.feature_lineage, assistant.feature_lineage);
    assert_eq!(manual.subshape_lineage, assistant.subshape_lineage);
    assert_eq!(manual.exact_jobs, assistant.exact_jobs);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);

    let candidate = document.preview_batch(manual.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        manual.fork_definition_id,
        "fork-last-valid",
    )));
    let receipt = commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &manual,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    )
    .unwrap();

    let committed = document.current();
    let current_source = exact_results
        .get_body(&committed, DEFINITION, BodyId(1))
        .unwrap()
        .unwrap();
    let current_fork = exact_results
        .get_body(&committed, receipt.fork_definition_id, BodyId(1))
        .unwrap()
        .unwrap();
    assert_eq!(current_source.bounds_mm(), source_package.bounds_mm());
    assert_eq!(current_source.vertices(), source_package.vertices());
    assert_eq!(current_source.triangles(), source_package.triangles());
    assert_ne!(current_source.definition_id(), current_fork.definition_id());
    assert_ne!(
        current_source.producer_feature_id(),
        current_fork.producer_feature_id()
    );
    assert_ne!(current_source.bounds_mm(), current_fork.bounds_mm());
    assert_ne!(current_source.vertices(), current_fork.vertices());
    assert_eq!(current_source.triangles(), current_fork.triangles());
    assert_ne!(current_source.references(), current_fork.references());
    assert!(
        current_source
            .references()
            .iter()
            .all(|reference| reference.definition_id == DEFINITION)
    );
    assert!(
        current_fork
            .references()
            .iter()
            .all(|reference| reference.definition_id == receipt.fork_definition_id)
    );
    assert_eq!(exact_results.body_values(&committed).unwrap().len(), 2);

    let committed_digest = committed.canonical_digest();
    let committed_results = exact_results.contents_stamp();
    assert_eq!(document.undo().unwrap().canonical_digest(), before.digest);
    assert_eq!(document.visible_undo_steps(), before.undo);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert_eq!(exact_results.contents_stamp(), committed_results);

    let saved_committed = persistence::save(&document.current());
    let reopened = persistence::load(&saved_committed).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(reopened_snapshot.canonical_digest(), committed_digest);
    assert_eq!(persistence::save(&reopened_snapshot), saved_committed);
    assert_eq!(
        reopened_snapshot.occurrence(FIRST).unwrap().definition_id(),
        receipt.fork_definition_id
    );
    assert_eq!(
        reopened_snapshot
            .occurrence(SECOND)
            .unwrap()
            .definition_id(),
        DEFINITION
    );
    assert_eq!(
        reopened_snapshot
            .definition(receipt.fork_definition_id)
            .unwrap()
            .feature_ids(),
        receipt
            .feature_lineage
            .iter()
            .map(|lineage| lineage.fork_feature_id)
            .collect::<Vec<_>>()
            .as_slice()
    );
    assert_eq!(
        exact_results
            .get_body(&reopened_snapshot, DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "source-last-valid"
    );
    assert_eq!(
        exact_results
            .get_body(&reopened_snapshot, receipt.fork_definition_id, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "fork-last-valid"
    );
}

#[test]
fn independent_fork_verifier_rejects_stale_and_tampered_dependency_reviews() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source, "source-verifier");
    let impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-verifier",
    )));
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();
    let saved_before = persistence::save(&source);

    let mut tampered_collection = impact.clone();
    tampered_collection.collection_dependencies.clear();
    let mut evaluations = 0;
    assert!(matches!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &tampered_collection,
            |_| -> Result<Arc<ExactBodyPackage>, String> {
                evaluations += 1;
                Ok(Arc::clone(&evaluated))
            },
        ),
        Err(OccurrenceForkPropagationError::InvalidImpact(reason))
            if reason.contains("collection dependency evidence")
    ));
    assert_eq!(evaluations, 0);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);

    let mut tampered_drawings = impact.clone();
    tampered_drawings.drawing_views.clear();
    assert!(matches!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &tampered_drawings,
            |_| -> Result<Arc<ExactBodyPackage>, String> { unreachable!() },
        ),
        Err(OccurrenceForkPropagationError::InvalidImpact(reason))
            if reason.contains("drawing dependency evidence")
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);

    let mut tampered_digest = impact.clone();
    tampered_digest.candidate_digest.push('0');
    assert!(matches!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &tampered_digest,
            |_| -> Result<Arc<ExactBodyPackage>, String> { unreachable!() },
        ),
        Err(OccurrenceForkPropagationError::InvalidImpact(reason))
            if reason.contains("candidate digest")
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);

    let mut stale_document = seed(false);
    let stale_source = stale_document.current();
    let mut stale_results = registry(&stale_source, "stale-source");
    let stale_impact = project_occurrence_fork_impact(
        &stale_document,
        &stale_results,
        edit_request(&stale_source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    stale_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OTHER,
                visible: false,
            },
        ]))
        .unwrap();
    let stale_before = stamp(&stale_document);
    let stale_results_before = stale_results.contents_stamp();
    let stale_saved_before = persistence::save(&stale_document.current());
    assert_eq!(
        commit_occurrence_fork_change(
            &mut stale_document,
            &mut stale_results,
            &stale_impact,
            |_| -> Result<Arc<ExactBodyPackage>, String> { unreachable!() },
        ),
        Err(OccurrenceForkPropagationError::Stale)
    );
    assert_eq!(stamp(&stale_document), stale_before);
    assert_eq!(stale_results.contents_stamp(), stale_results_before);
    assert_eq!(
        persistence::save(&stale_document.current()),
        stale_saved_before
    );
}

#[test]
fn reviewed_occurrence_fork_commits_once_and_refreshes_only_the_selected_branch() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source, "last-valid");
    let before = stamp(&document);
    let source_definition = source.definition(DEFINITION).unwrap().clone();
    let sibling = source.occurrence(SECOND).unwrap().clone();
    let unrelated_definition = source.definition(OTHER_DEFINITION).unwrap().clone();
    let selected_transform = source.occurrence(FIRST).unwrap().transform();
    let impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-exact",
    )));
    let mut evaluations = 0;

    let receipt = commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &impact,
        |request| -> Result<Arc<ExactBodyPackage>, String> {
            evaluations += 1;
            assert_eq!(request.definition_id, DefinitionId(3));
            assert_eq!(request.producer_feature_id(), FeatureId(13));
            Ok(Arc::clone(&evaluated))
        },
    )
    .unwrap();

    assert_eq!(evaluations, 1);
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    assert_eq!(receipt.revision_id, document.current().revision_id());
    assert_eq!(receipt.canonical_digest, impact.candidate_digest);
    assert_eq!(receipt.selected_occurrence.occurrence_id, FIRST);
    assert_eq!(receipt.selected_occurrence.transform, selected_transform);
    assert_eq!(receipt.selected_occurrence.result_fingerprint, "fork-exact");
    assert_eq!(receipt.source_definition_id, DEFINITION);
    assert_eq!(receipt.fork_definition_id, DefinitionId(3));
    assert_eq!(receipt.body_lineage, impact.body_lineage);
    assert_eq!(receipt.feature_lineage, impact.feature_lineage);
    assert_eq!(receipt.subshape_lineage, impact.subshape_lineage);
    assert_eq!(receipt.unaffected_sibling_occurrence_ids, vec![SECOND]);

    let committed = document.current();
    assert_eq!(
        committed.occurrence(FIRST).unwrap().definition_id(),
        DefinitionId(3)
    );
    assert_eq!(
        committed.occurrence(FIRST).unwrap().transform(),
        selected_transform
    );
    assert_eq!(committed.occurrence(SECOND), Some(&sibling));
    assert_eq!(committed.definition(DEFINITION), Some(&source_definition));
    assert_eq!(
        committed.definition(OTHER_DEFINITION),
        Some(&unrelated_definition)
    );
    assert_eq!(
        exact_results
            .get_body(&committed, DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "last-valid"
    );
    assert_eq!(
        exact_results
            .get_body(&committed, DefinitionId(3), BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "fork-exact"
    );
    assert_eq!(exact_results.body_values(&committed).unwrap().len(), 2);

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.digest);
    assert_eq!(document.visible_undo_steps(), before.undo);
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        receipt.canonical_digest
    );
    assert!(
        exact_results
            .get_body(&document.current(), DefinitionId(3), BodyId(1))
            .unwrap()
            .is_some()
    );
}

#[test]
fn occurrence_fork_refreshes_only_selected_planar_axial_dependencies_and_outputs() {
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Positioned assembly".into(),
                transform: Transform::from_translation(7.0, 11.0, 13.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: FIRST,
                transform: Transform::from_translation(-7.0, -11.0, -13.0).unwrap(),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: FIRST,
                parent: Some(GROUP),
            },
        ]))
        .unwrap();
    let axial_evidence = exact_package(&document.current(), "axial-evidence");
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved(
                    FIRST,
                    axial_evidence
                        .reference(ExactFaceRole::East)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    SECOND,
                    axial_evidence
                        .reference(ExactFaceRole::East)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateKind::ConcentricAxial { reversed: false },
            )),
        ]))
        .unwrap();
    let source = document.current();
    let source_selected = source.occurrence(FIRST).unwrap().clone();
    let source_world_transform = source.world_transform_for_occurrence(FIRST).unwrap();
    let source_collection = source.collection(COLLECTION).unwrap().clone();
    let source_drawing_source = source.drawing_sheet(SHEET).unwrap().source().clone();
    let source_sibling = source.occurrence(SECOND).unwrap().clone();
    let source_mates =
        [MATE, AXIAL_MATE].map(|mate_id| source.assembly_mate(mate_id).unwrap().clone());
    let mut exact_results = registry(&source, "source-current");
    let impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(
        impact
            .mate_references
            .iter()
            .map(|reference| reference.mate_id)
            .collect::<Vec<_>>(),
        vec![MATE, AXIAL_MATE]
    );
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-current",
    )));
    let before = stamp(&document);

    let receipt = commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    )
    .unwrap();

    assert_eq!(receipt.rebound_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(receipt.drawings.len(), 1);
    assert!(receipt.drawings[0].is_current(&document.current()));
    assert_eq!(
        receipt.drawings[0]
            .views
            .iter()
            .map(|view| view.kind)
            .collect::<Vec<_>>(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ]
    );
    assert_eq!(
        receipt
            .exports
            .iter()
            .map(|export| (export.format, export.eligibility))
            .collect::<Vec<_>>(),
        vec![
            (
                SharedChangeExportFormat::Step,
                SharedChangeExportEligibility::CurrentExact,
            ),
            (
                SharedChangeExportFormat::Stl,
                SharedChangeExportEligibility::CurrentExact,
            ),
        ]
    );
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    let committed = document.current();
    let committed_selected = committed.occurrence(FIRST).unwrap();
    assert_eq!(committed_selected.id(), source_selected.id());
    assert_eq!(committed_selected.name(), source_selected.name());
    assert_eq!(committed_selected.transform(), source_selected.transform());
    assert_eq!(committed_selected.parent(), source_selected.parent());
    assert_eq!(
        committed.world_transform_for_occurrence(FIRST),
        Some(source_world_transform)
    );
    assert_eq!(committed.collection(COLLECTION), Some(&source_collection));
    assert_eq!(
        committed.drawing_sheet(SHEET).unwrap().source(),
        &source_drawing_source
    );
    assert_eq!(committed.occurrence(SECOND), Some(&source_sibling));
    for (mate_id, source_mate) in [MATE, AXIAL_MATE].into_iter().zip(&source_mates) {
        let committed_mate = committed.assembly_mate(mate_id).unwrap();
        assert_eq!(committed_mate.endpoint_b(), source_mate.endpoint_b());
        assert_eq!(
            committed_mate.endpoint_a().reference().definition_id,
            receipt.fork_definition_id
        );
        assert_eq!(
            committed_mate.endpoint_a().reference().result_fingerprint,
            "fork-current"
        );
    }
    assert_eq!(
        exact_results
            .get_body(&document.current(), DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "source-current"
    );

    let committed_digest = committed.canonical_digest();
    let committed_mates =
        [MATE, AXIAL_MATE].map(|mate_id| committed.assembly_mate(mate_id).unwrap().clone());
    let committed_drawing = receipt.drawings[0].clone();
    let committed_package = Arc::clone(
        exact_results
            .get_body(&committed, receipt.fork_definition_id, BodyId(1))
            .unwrap()
            .unwrap(),
    );
    let committed_stl = exact_model_stl_export(
        &committed,
        &[(
            committed_package.as_ref(),
            committed.occurrence(FIRST).unwrap().transform(),
        )],
    )
    .unwrap();
    let saved = persistence::save(&committed);
    let reopened = persistence::load(&saved).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_results = ExactResultRegistry::carried_forward(&reopened_snapshot, &exact_results);
    assert_eq!(reopened_snapshot.canonical_digest(), committed_digest);
    assert_eq!(
        reopened_snapshot.world_transform_for_occurrence(FIRST),
        Some(source_world_transform)
    );
    assert_eq!(
        reopened_snapshot.collection(COLLECTION),
        Some(&source_collection)
    );
    assert_eq!(
        reopened_snapshot.drawing_sheet(SHEET).unwrap().source(),
        &source_drawing_source
    );
    assert_eq!(
        [MATE, AXIAL_MATE].map(|mate_id| reopened_snapshot.assembly_mate(mate_id).unwrap().clone()),
        committed_mates
    );
    assert_eq!(
        project_orthographic_drawing(
            &reopened_snapshot,
            &reopened_results,
            reopened_snapshot.drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        committed_drawing
    );
    assert_eq!(
        exact_model_stl_export(
            &reopened_snapshot,
            &[(
                reopened_results
                    .get_body(&reopened_snapshot, receipt.fork_definition_id, BodyId(1))
                    .unwrap()
                    .unwrap()
                    .as_ref(),
                reopened_snapshot.occurrence(FIRST).unwrap().transform(),
            )],
        )
        .unwrap(),
        committed_stl
    );

    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        source.canonical_digest()
    );
    assert_eq!(document.current().occurrence(SECOND), Some(&source_sibling));
    assert_eq!(
        [MATE, AXIAL_MATE].map(|mate_id| document
            .current()
            .assembly_mate(mate_id)
            .unwrap()
            .clone()),
        source_mates
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert_eq!(
        [MATE, AXIAL_MATE].map(|mate_id| document
            .current()
            .assembly_mate(mate_id)
            .unwrap()
            .clone()),
        committed_mates
    );
}

#[test]
fn conflicting_local_dependency_preserves_canonical_history_and_transforms() {
    let mut document = seed(false);
    let evidence = exact_package(&document.current(), "conflicting-dependencies");
    let top = evidence.reference(ExactFaceRole::Top).unwrap().clone();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteDrawingSheet { id: SHEET },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SECOND,
                grounded: false,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved(FIRST, top.clone()),
                AssemblyMateEndpoint::resolved(SECOND, top.clone()),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(42),
                AssemblyMateEndpoint::resolved(FIRST, top.clone()),
                AssemblyMateEndpoint::resolved(SECOND, top),
                AssemblyMateKind::Distance { distance_mm: 10.0 },
            )),
        ]))
        .unwrap();
    let source = document.current();
    let mut exact_results = registry(&source, "source-conflict");
    let source_transforms = [FIRST, SECOND].map(|id| source.occurrence(id).unwrap().transform());
    let impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-conflict",
    )));
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();

    let failed = commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    );

    assert!(
        matches!(failed, Err(OccurrenceForkPropagationError::Dependency(_))),
        "{failed:?}"
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(
        [FIRST, SECOND].map(|id| document.current().occurrence(id).unwrap().transform()),
        source_transforms
    );
}

#[test]
fn underconstrained_local_dependency_is_rejected_before_fork_preview() {
    let mut document = seed(false);
    let source = document.current();
    let source_transforms = [FIRST, SECOND].map(|id| source.occurrence(id).unwrap().transform());
    let source_mates = source.assembly_mates().cloned().collect::<Vec<_>>();
    let exact_results = registry(&source, "source-underconstrained");
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();
    let saved_before = persistence::save(&source);

    let failed = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceGrounded {
            id: SECOND,
            grounded: false,
        },
    ]));

    assert!(matches!(
        failed,
        Err(CanonicalError::Drawing(DrawingError::SourceNotRigid))
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);
    assert_eq!(
        [FIRST, SECOND].map(|id| document.current().occurrence(id).unwrap().transform()),
        source_transforms
    );
    assert_eq!(
        document
            .current()
            .assembly_mates()
            .cloned()
            .collect::<Vec<_>>(),
        source_mates
    );
}

#[test]
fn unsupported_local_export_preserves_last_valid_views_and_products() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source, "source-unsupported");
    let source_package = Arc::clone(
        exact_results
            .get_body(&source, DEFINITION, BodyId(1))
            .unwrap()
            .unwrap(),
    );
    let source_drawing = project_orthographic_drawing(
        &source,
        &exact_results,
        source.drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    let source_stl = exact_model_stl_export(
        &source,
        &[(
            source_package.as_ref(),
            source.occurrence(FIRST).unwrap().transform(),
        )],
    )
    .unwrap();
    let mut impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    for export in &mut impact.exports {
        export.occurrence_paths = vec![ketchup_core::document::InstancePath::root(OTHER)];
    }
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-unsupported",
    )));
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();

    let failed = commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    );

    assert!(
        matches!(failed, Err(OccurrenceForkPropagationError::Dependency(_))),
        "{failed:?}"
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &exact_results,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        source_drawing
    );
    assert_eq!(
        exact_model_stl_export(
            &document.current(),
            &[(
                source_package.as_ref(),
                document.current().occurrence(FIRST).unwrap().transform(),
            )],
        )
        .unwrap(),
        source_stl
    );
}

#[test]
fn failed_followup_fork_preserves_existing_source_and_fork_outputs() {
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: THIRD,
                definition_id: DEFINITION,
                name: "Remaining source reuse".into(),
                transform: Transform::from_translation(40.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let source = document.current();
    let mut exact_results = registry(&source, "source-output");
    let first_impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let first_candidate = document
        .preview_batch(first_impact.proposal.batch())
        .unwrap();
    let first_evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &first_candidate,
        first_impact.fork_definition_id,
        "first-fork-output",
    )));
    commit_occurrence_fork_change(
        &mut document,
        &mut exact_results,
        &first_impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&first_evaluated)) },
    )
    .unwrap();

    let committed = document.current();
    assert_eq!(
        committed.occurrence(SECOND).unwrap().definition_id(),
        DEFINITION
    );
    assert_eq!(
        committed.occurrence(THIRD).unwrap().definition_id(),
        DEFINITION
    );
    let source_output = Arc::clone(
        exact_results
            .get_body(&committed, DEFINITION, BodyId(1))
            .unwrap()
            .unwrap(),
    );
    let first_fork_output = Arc::clone(
        exact_results
            .get_body(&committed, first_impact.fork_definition_id, BodyId(1))
            .unwrap()
            .unwrap(),
    );
    let followup = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&committed, SECOND),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();
    let saved_before = persistence::save(&committed);

    assert_eq!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &followup,
            |_| -> Result<Arc<ExactBodyPackage>, String> { Err("followup worker failed".into()) },
        ),
        Err(OccurrenceForkPropagationError::Evaluation(
            "followup worker failed".into()
        ))
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);
    assert_eq!(
        exact_results
            .get_body(&document.current(), DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .as_ref(),
        source_output.as_ref()
    );
    assert_eq!(
        exact_results
            .get_body(
                &document.current(),
                first_impact.fork_definition_id,
                BodyId(1),
            )
            .unwrap()
            .unwrap()
            .as_ref(),
        first_fork_output.as_ref()
    );
}

#[test]
fn occurrence_fork_evaluation_and_publication_fail_without_partial_state() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source, "last-valid");
    let impact = project_occurrence_fork_impact(
        &document,
        &exact_results,
        edit_request(&source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let before = stamp(&document);
    let results_before = exact_results.contents_stamp();
    let saved_before = persistence::save(&source);
    let mut evaluations = 0;

    assert_eq!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &impact,
            |_| -> Result<Arc<ExactBodyPackage>, String> {
                evaluations += 1;
                Err("worker failed".to_owned())
            },
        ),
        Err(OccurrenceForkPropagationError::Evaluation(
            "worker failed".to_owned()
        ))
    );
    assert_eq!(evaluations, 1);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);

    let stale_package = Arc::new(ExactBodyPackage::from(exact_package(&source, "stale")));
    assert!(matches!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &impact,
            |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&stale_package)) },
        ),
        Err(OccurrenceForkPropagationError::ExactPublication(_))
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);

    let mut invalid = impact.clone();
    invalid.affected_fork_body_ids.push(BodyId(1));
    let mut invalid_evaluations = 0;
    assert!(matches!(
        commit_occurrence_fork_change(
            &mut document,
            &mut exact_results,
            &invalid,
            |_| -> Result<Arc<ExactBodyPackage>, String> {
                invalid_evaluations += 1;
                unreachable!()
            },
        ),
        Err(OccurrenceForkPropagationError::InvalidImpact(_))
    ));
    assert_eq!(invalid_evaluations, 0);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), results_before);
}

#[test]
fn dependency_closed_suffix_projection_is_mapped_only_to_the_fork() {
    const CUT_PROFILE: FeatureId = FeatureId(12);
    const POCKET: FeatureId = FeatureId(13);
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".into(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::from_decimal("2").unwrap(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut results = registry(&snapshot, "last-valid-pocket");
    let before = stamp(&document);
    let results_before = results.contents_stamp();
    let source_definition = snapshot.definition(DEFINITION).unwrap().clone();
    let request = OccurrenceForkChangeRequest::body_history_mutation(
        &snapshot,
        FIRST,
        "Selected reuse unique",
        BodyHistoryMutationRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            mutation: BodyHistoryMutation::SuppressFrom(CUT_PROFILE),
        },
    );

    let impact = project_occurrence_fork_impact(
        &document,
        &results,
        request,
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(impact.fork_definition_id, DefinitionId(3));
    assert_eq!(
        impact.feature_lineage,
        vec![
            OccurrenceForkFeatureLineage {
                source_feature_id: PROFILE,
                fork_feature_id: FeatureId(14),
            },
            OccurrenceForkFeatureLineage {
                source_feature_id: EXTRUSION,
                fork_feature_id: FeatureId(15),
            },
            OccurrenceForkFeatureLineage {
                source_feature_id: CUT_PROFILE,
                fork_feature_id: FeatureId(16),
            },
            OccurrenceForkFeatureLineage {
                source_feature_id: POCKET,
                fork_feature_id: FeatureId(17),
            },
        ]
    );
    assert_eq!(
        impact.affected_fork_feature_ids,
        vec![FeatureId(16), FeatureId(17)]
    );
    assert_eq!(impact.exact_jobs[0].producer_feature_id, FeatureId(15));
    assert_eq!(
        impact.exact_jobs[0].last_valid_result_fingerprint,
        "last-valid-pocket"
    );
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    assert_eq!(candidate.definition(DEFINITION), Some(&source_definition));
    assert!(
        candidate
            .suppressed_feature_ids(DEFINITION, BodyId(1))
            .is_none_or(|suppressed| suppressed.is_empty())
    );
    assert_eq!(
        candidate
            .suppressed_feature_ids(DefinitionId(3), BodyId(1))
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![FeatureId(16), FeatureId(17)]
    );
    assert_eq!(
        candidate.occurrence(SECOND).unwrap().definition_id(),
        DEFINITION
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);

    let evaluated = Arc::new(ExactBodyPackage::from(exact_package_for(
        &candidate,
        impact.fork_definition_id,
        "fork-suppressed",
    )));
    let mut evaluations = 0;
    let receipt = commit_occurrence_fork_change(
        &mut document,
        &mut results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> {
            evaluations += 1;
            Ok(Arc::clone(&evaluated))
        },
    )
    .unwrap();
    assert_eq!(evaluations, 1);
    assert_eq!(
        receipt.selected_occurrence.result_fingerprint,
        "fork-suppressed"
    );
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert!(
        document
            .current()
            .suppressed_feature_ids(DEFINITION, BodyId(1))
            .is_none_or(|suppressed| suppressed.is_empty())
    );
    assert_eq!(
        document
            .current()
            .suppressed_feature_ids(DefinitionId(3), BodyId(1))
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![FeatureId(16), FeatureId(17)]
    );
    assert_eq!(
        results
            .get_body(&document.current(), DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "last-valid-pocket"
    );
    assert_eq!(
        results
            .get_body(&document.current(), DefinitionId(3), BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "fork-suppressed"
    );
}

#[test]
fn invalid_fork_requests_fail_closed_without_history_or_exact_mutation() {
    let mut document = seed(false);
    let snapshot = document.current();
    let results = registry(&snapshot, "last-valid");
    let before = stamp(&document);
    let results_before = results.contents_stamp();

    assert_eq!(
        project_occurrence_fork_impact(
            &document,
            &ExactResultRegistry::default(),
            edit_request(&snapshot, FIRST),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Failed(BodyId(1)))
    );
    let cross_definition = OccurrenceForkChangeRequest::exact_parameter_edit(
        &snapshot,
        FIRST,
        "Invalid fork",
        BodyParameterEditRequest {
            definition_id: OTHER_DEFINITION,
            body_id: BodyId(1),
            edits: vec![ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
                dimension: Dimension::from_decimal("15").unwrap(),
            }],
        },
    );
    assert_eq!(
        project_occurrence_fork_impact(
            &document,
            &results,
            cross_definition,
            ProposalPrincipal::LocalAssistant,
        ),
        Err(OccurrenceForkImpactError::CrossDefinition(
            DEFINITION,
            OTHER_DEFINITION,
        ))
    );
    let empty_name = OccurrenceForkChangeRequest::exact_parameter_edit(
        &snapshot,
        FIRST,
        "",
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
                dimension: Dimension::from_decimal("15").unwrap(),
            }],
        },
    );
    assert!(matches!(
        project_occurrence_fork_impact(
            &document,
            &results,
            empty_name,
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Unsupported(_))
    ));
    let duplicate = ExactParameterEdit {
        target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
        dimension: Dimension::from_decimal("15").unwrap(),
    };
    let duplicate_target = OccurrenceForkChangeRequest::exact_parameter_edit(
        &snapshot,
        FIRST,
        "Invalid duplicate target",
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![duplicate.clone(), duplicate],
        },
    );
    assert!(matches!(
        project_occurrence_fork_impact(
            &document,
            &results,
            duplicate_target,
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Unsupported(reason)) if reason.contains("duplicate")
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);

    let stale_request = edit_request(&snapshot, FIRST);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OTHER,
                visible: false,
            },
        ]))
        .unwrap();
    let changed = stamp(&document);
    assert_eq!(
        project_occurrence_fork_impact(
            &document,
            &results,
            stale_request,
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Stale)
    );
    assert_eq!(stamp(&document), changed);
}

#[test]
fn duplicate_identity_and_cyclic_sources_are_rejected_before_fork_preview() {
    let mut duplicate = seed(false);
    let duplicate_before = stamp(&duplicate);
    let duplicate_saved = persistence::save(&duplicate.current());
    let duplicate_error = match duplicate.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Duplicate definition".into(),
        },
    ])) {
        Ok(_) => panic!("duplicate definition identity was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        duplicate_error,
        CanonicalError::DefinitionAlreadyExists(DEFINITION)
    );
    assert_eq!(stamp(&duplicate), duplicate_before);
    assert_eq!(persistence::save(&duplicate.current()), duplicate_saved);

    const CYCLIC_DEFINITION: DefinitionId = DefinitionId(70);
    const FIRST_SHELL: FeatureId = FeatureId(300);
    const SECOND_SHELL: FeatureId = FeatureId(301);
    let mut cyclic = DocumentStore::new();
    let cyclic_before = stamp(&cyclic);
    let cyclic_saved = persistence::save(&cyclic.current());
    let shell = |target| FeatureKind::Shell {
        target,
        removed_faces: vec![StableFaceRole::new("test.face").unwrap()],
        thickness: Dimension::from_decimal("1").unwrap(),
    };
    let cycle_error = match cyclic.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: CYCLIC_DEFINITION,
            name: "Cycle".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FIRST_SHELL,
            definition_id: CYCLIC_DEFINITION,
            name: "First".into(),
            kind: shell(SECOND_SHELL),
        },
        CanonicalCommand::CreateFeature {
            id: SECOND_SHELL,
            definition_id: CYCLIC_DEFINITION,
            name: "Second".into(),
            kind: shell(FIRST_SHELL),
        },
    ])) {
        Ok(_) => panic!("cyclic feature graph was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        cycle_error,
        CanonicalError::FeatureDependencyCycle(FIRST_SHELL)
    );
    assert_eq!(stamp(&cyclic), cyclic_before);
    assert_eq!(persistence::save(&cyclic.current()), cyclic_saved);
}

#[test]
fn hidden_single_use_lost_and_ambiguous_inputs_are_rejected_observationally() {
    let mut hidden = seed(false);
    hidden
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: FIRST,
                visible: false,
            },
        ]))
        .unwrap();
    let hidden_snapshot = hidden.current();
    let hidden_results = registry(&hidden_snapshot, "hidden");
    let hidden_before = stamp(&hidden);
    assert_eq!(
        project_occurrence_fork_impact(
            &hidden,
            &hidden_results,
            edit_request(&hidden_snapshot, FIRST),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Hidden(FIRST))
    );
    assert_eq!(stamp(&hidden), hidden_before);

    let mut single = seed(false);
    single
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::DeleteDrawingSheet { id: SHEET },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![FIRST, OTHER],
            },
            CanonicalCommand::DeleteOccurrence { id: SECOND },
        ]))
        .unwrap();
    let single_snapshot = single.current();
    let single_results = registry(&single_snapshot, "single");
    let single_before = stamp(&single);
    assert_eq!(
        project_occurrence_fork_impact(
            &single,
            &single_results,
            edit_request(&single_snapshot, FIRST),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::DefinitionNotReused(DEFINITION))
    );
    assert_eq!(stamp(&single), single_before);

    let mut lost = seed(false);
    let mate = lost.current().assembly_mate(MATE).unwrap().clone();
    lost.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::UpdateDrawingSheet(
            DrawingSheet::new(SHEET, "Reused part", DrawingSource::Definition(DEFINITION)).unwrap(),
        ),
        CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
            MATE,
            AssemblyMateEndpoint::lost(FIRST, mate.endpoint_a().reference().clone()),
            mate.endpoint_b().clone(),
            mate.kind(),
        )),
    ]))
    .unwrap();
    let lost_snapshot = lost.current();
    let lost_results = registry(&lost_snapshot, "lost");
    let lost_before = stamp(&lost);
    assert_eq!(
        project_occurrence_fork_impact(
            &lost,
            &lost_results,
            edit_request(&lost_snapshot, FIRST),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceForkImpactError::Lost(MATE))
    );
    assert_eq!(stamp(&lost), lost_before);

    let ambiguous_document = seed(false);
    let ambiguous_snapshot = ambiguous_document.current();
    let package = exact_package(&ambiguous_snapshot, "ambiguous-a");
    let mut alternate = package.clone();
    alternate.identity.backend = "alternate".into();
    for reference in &mut alternate.references {
        reference.backend = alternate.identity.backend.clone();
    }
    let ambiguous_results = ExactResultRegistry::accept(
        &ambiguous_snapshot,
        [
            Arc::new(ExactBodyPackage::from(package)),
            Arc::new(ExactBodyPackage::from(alternate)),
        ],
    )
    .unwrap();
    let ambiguous_before = stamp(&ambiguous_document);
    assert_eq!(
        project_occurrence_fork_impact(
            &ambiguous_document,
            &ambiguous_results,
            edit_request(&ambiguous_snapshot, FIRST),
            ProposalPrincipal::LocalAssistant,
        ),
        Err(OccurrenceForkImpactError::Ambiguous(BodyId(1)))
    );
    assert_eq!(stamp(&ambiguous_document), ambiguous_before);
}
