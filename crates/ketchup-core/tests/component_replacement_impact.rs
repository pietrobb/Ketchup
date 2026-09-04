use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, PlanarFaceAttachment,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CollectionId, CommandBatch, DefinitionId, Dimension, DocumentId,
    DocumentStore, FeatureId, FeatureKind, GroupId, OccurrenceId, Transform,
};
use ketchup_core::drawing::{
    DrawingSheet, DrawingSheetId, DrawingSource, OrthographicViewKind, project_orthographic_drawing,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactPlanarFaceAttachmentInput,
    ExactRenderPackage, ExactResultRegistry, build_box_render_package_with_attachments,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::shared_change::{
    ComponentReplacementBodyCorrespondence, ComponentReplacementCommitError,
    ComponentReplacementFeatureCorrespondence, ComponentReplacementImpactError,
    ComponentReplacementImpactRequest, SharedChangeExportEligibility, SharedChangeExportFormat,
    commit_component_replacement, project_component_replacement_impact,
};
use std::sync::Arc;

const SOURCE: DefinitionId = DefinitionId(1);
const TARGET: DefinitionId = DefinitionId(2);
const SOURCE_PROFILE: FeatureId = FeatureId(10);
const SOURCE_EXTRUSION: FeatureId = FeatureId(11);
const TARGET_PROFILE: FeatureId = FeatureId(20);
const TARGET_EXTRUSION: FeatureId = FeatureId(21);
const SELECTED: OccurrenceId = OccurrenceId(100);
const SIBLING: OccurrenceId = OccurrenceId(101);
const TARGET_OCCURRENCE: OccurrenceId = OccurrenceId(200);
const MATE: AssemblyMateId = AssemblyMateId(300);
const AXIAL_MATE: AssemblyMateId = AssemblyMateId(301);
const SHEET: DrawingSheetId = DrawingSheetId(400);
const COLLECTION: CollectionId = CollectionId(500);
const GROUP: GroupId = GroupId(600);

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
            format!("geometry:{definition_id:?}:{role:?}:{fingerprint}"),
        )
    };
    build_box_render_package_with_attachments(
        &request,
        format!("exact-input:{definition_id:?}:{fingerprint}"),
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
        &[
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::Top,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [0.0, 0.0, 1.0],
            },
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::Bottom,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [0.0, 0.0, -1.0],
            },
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::East,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [1.0, 0.0, 0.0],
            },
        ],
    )
    .unwrap()
}

fn registry(snapshot: &ketchup_core::document::Snapshot) -> ExactResultRegistry {
    ExactResultRegistry::accept(
        snapshot,
        [
            Arc::new(ExactBodyPackage::from(exact_package(
                snapshot,
                SOURCE,
                "source-current",
            ))),
            Arc::new(ExactBodyPackage::from(exact_package(
                snapshot,
                TARGET,
                "target-current",
            ))),
        ],
    )
    .unwrap()
}

fn seed(reverse_occurrences: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let selected = CanonicalCommand::CreateOccurrence {
        id: SELECTED,
        definition_id: SOURCE,
        name: "Selected source".into(),
        transform: Transform::from_translation(5.0, 6.0, 7.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    };
    let sibling = CanonicalCommand::CreateOccurrence {
        id: SIBLING,
        definition_id: SOURCE,
        name: "Source sibling".into(),
        transform: Transform::from_translation(25.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    };
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: SOURCE,
            name: "Source component".into(),
        },
        CanonicalCommand::CreateDefinition {
            id: TARGET,
            name: "Compatible target".into(),
        },
        CanonicalCommand::CreateFeature {
            id: SOURCE_PROFILE,
            definition_id: SOURCE,
            name: "Source profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: SOURCE_EXTRUSION,
            definition_id: SOURCE,
            name: "Source extrusion".into(),
            kind: FeatureKind::Extrusion {
                profile: SOURCE_PROFILE,
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
        CanonicalCommand::CreateFeature {
            id: TARGET_PROFILE,
            definition_id: TARGET,
            name: "Target profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [14.0, 0.0], [14.0, 8.0], [0.0, 8.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: TARGET_EXTRUSION,
            definition_id: TARGET,
            name: "Target extrusion".into(),
            kind: FeatureKind::Extrusion {
                profile: TARGET_PROFILE,
                height: Dimension::from_decimal("18").unwrap(),
            },
        },
    ];
    commands.extend(if reverse_occurrences {
        vec![sibling, selected]
    } else {
        vec![selected, sibling]
    });
    commands.push(CanonicalCommand::CreateOccurrence {
        id: TARGET_OCCURRENCE,
        definition_id: TARGET,
        name: "Existing target occurrence".into(),
        transform: Transform::from_translation(50.0, 6.0, 7.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let evidence = exact_package(&document.current(), SOURCE, "mate-evidence");
    let target_evidence = exact_package(&document.current(), TARGET, "mate-target");
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::resolved_planar_face(
                    SELECTED,
                    PlanarFaceAttachment::new(
                        evidence.reference(ExactFaceRole::Top).unwrap().clone(),
                        [0.0; 3],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateEndpoint::resolved_planar_face(
                    TARGET_OCCURRENCE,
                    PlanarFaceAttachment::new(
                        target_evidence
                            .reference(ExactFaceRole::Bottom)
                            .unwrap()
                            .clone(),
                        [0.0; 3],
                        [0.0, 0.0, -1.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved_planar_face(
                    SELECTED,
                    PlanarFaceAttachment::new(
                        evidence.reference(ExactFaceRole::East).unwrap().clone(),
                        [0.0; 3],
                        [1.0, 0.0, 0.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateEndpoint::resolved_planar_face(
                    TARGET_OCCURRENCE,
                    PlanarFaceAttachment::new(
                        target_evidence
                            .reference(ExactFaceRole::East)
                            .unwrap()
                            .clone(),
                        [0.0; 3],
                        [1.0, 0.0, 0.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 45.0,
                    reversed: true,
                },
            )),
            CanonicalCommand::SetOccurrenceGrounded {
                id: SELECTED,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SIBLING,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: TARGET_OCCURRENCE,
                grounded: true,
            },
            CanonicalCommand::CreateDrawingSheet(
                DrawingSheet::new(
                    SHEET,
                    "Replacement assembly",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![SELECTED, SIBLING, TARGET_OCCURRENCE],
                    },
                )
                .unwrap(),
            ),
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Replacement selection".into(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![SELECTED, SIBLING, TARGET_OCCURRENCE],
            },
        ]))
        .unwrap();
    document
}

#[test]
fn replacement_impact_is_deterministic_complete_and_non_mutating() {
    let first = seed(false);
    let second = seed(true);
    let first_results = registry(&first.current());
    let second_results = registry(&second.current());
    let first_before = stamp(&first);
    let second_before = stamp(&second);
    let first_results_before = first_results.contents_stamp();
    let saved_before = persistence::save(&first.current());

    let first_impact = project_component_replacement_impact(
        &first,
        &first_results,
        ComponentReplacementImpactRequest::new(&first.current(), SELECTED, TARGET),
    )
    .unwrap();
    let second_impact = project_component_replacement_impact(
        &second,
        &second_results,
        ComponentReplacementImpactRequest::new(&second.current(), SELECTED, TARGET),
    )
    .unwrap();

    assert_eq!(
        first_impact.body_correspondence,
        second_impact.body_correspondence
    );
    assert_eq!(
        first_impact.feature_correspondence,
        second_impact.feature_correspondence
    );
    assert_eq!(
        first_impact
            .subshape_correspondence
            .iter()
            .map(|mapping| (
                mapping.source_profile_feature_id,
                mapping.source_producer_feature_id,
                mapping.target_profile_feature_id,
                mapping.target_producer_feature_id,
                &mapping.semantic_role,
                &mapping.source_element_id,
                &mapping.expected_type,
            ))
            .collect::<Vec<_>>(),
        second_impact
            .subshape_correspondence
            .iter()
            .map(|mapping| (
                mapping.source_profile_feature_id,
                mapping.source_producer_feature_id,
                mapping.target_profile_feature_id,
                mapping.target_producer_feature_id,
                &mapping.semantic_role,
                &mapping.source_element_id,
                &mapping.expected_type,
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first_impact.unchanged_source_occurrences,
        second_impact.unchanged_source_occurrences
    );
    assert_eq!(
        first_impact.unchanged_target_occurrences,
        second_impact.unchanged_target_occurrences
    );
    assert_eq!(
        first_impact.collection_dependencies,
        second_impact.collection_dependencies
    );
    assert_eq!(first_impact.drawing_views, second_impact.drawing_views);
    assert_eq!(first_impact.exports, second_impact.exports);
    assert_eq!(first_impact.selected_occurrence_id, SELECTED);
    assert_eq!(
        first_impact.selected_instance_path,
        ketchup_core::document::InstancePath::root(SELECTED)
    );
    assert_eq!(
        first_impact.selected_transform,
        first.current().occurrence(SELECTED).unwrap().transform()
    );
    assert_eq!(first_impact.source_definition_id, SOURCE);
    assert_eq!(first_impact.target_definition_id, TARGET);
    assert_eq!(
        first_impact.body_correspondence,
        vec![ComponentReplacementBodyCorrespondence {
            source_body_id: BodyId(1),
            target_body_id: BodyId(1),
        }]
    );
    assert_eq!(
        first_impact.feature_correspondence,
        vec![
            ComponentReplacementFeatureCorrespondence {
                source_feature_id: SOURCE_PROFILE,
                target_feature_id: TARGET_PROFILE,
            },
            ComponentReplacementFeatureCorrespondence {
                source_feature_id: SOURCE_EXTRUSION,
                target_feature_id: TARGET_EXTRUSION,
            },
        ]
    );
    assert_eq!(first_impact.subshape_correspondence.len(), 3);
    assert!(first_impact.subshape_correspondence.iter().all(|mapping| {
        mapping.source_profile_feature_id == SOURCE_PROFILE
            && mapping.source_producer_feature_id == SOURCE_EXTRUSION
            && mapping.target_profile_feature_id == TARGET_PROFILE
            && mapping.target_producer_feature_id == TARGET_EXTRUSION
            && mapping.source_lineage_digest != mapping.target_lineage_digest
    }));
    assert_eq!(
        first_impact
            .unchanged_source_occurrences
            .iter()
            .map(|occurrence| occurrence.occurrence_id)
            .collect::<Vec<_>>(),
        vec![SIBLING]
    );
    assert_eq!(
        first_impact
            .unchanged_target_occurrences
            .iter()
            .map(|occurrence| occurrence.occurrence_id)
            .collect::<Vec<_>>(),
        vec![TARGET_OCCURRENCE]
    );
    assert_eq!(first_impact.unchanged_definition_ids, vec![SOURCE, TARGET]);
    assert_eq!(first_impact.exact_jobs.len(), 1);
    assert_eq!(first_impact.exact_jobs[0].definition_id, TARGET);
    assert_eq!(first_impact.exact_jobs[0].body_id, BodyId(1));
    assert_eq!(
        first_impact.exact_jobs[0].producer_feature_id,
        TARGET_EXTRUSION
    );
    assert_eq!(
        first_impact.exact_jobs[0].last_valid_result_fingerprint,
        "target-current"
    );
    assert_eq!(
        first_impact
            .mate_references
            .iter()
            .map(|reference| reference.mate_id)
            .collect::<Vec<_>>(),
        vec![MATE, AXIAL_MATE]
    );
    assert_eq!(first_impact.collection_dependencies.len(), 1);
    assert_eq!(
        first_impact.collection_dependencies[0].collection_id,
        COLLECTION
    );
    assert_eq!(
        first_impact.collection_dependencies[0].occurrence_ids_before,
        vec![SELECTED, SIBLING, TARGET_OCCURRENCE]
    );
    assert_eq!(
        first_impact.collection_dependencies[0].occurrence_ids_after,
        vec![SELECTED, SIBLING, TARGET_OCCURRENCE]
    );
    assert_eq!(
        first_impact
            .drawing_views
            .iter()
            .map(|impact| impact.view)
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
            .map(|impact| (impact.format, impact.eligibility))
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
    assert_eq!(stamp(&first), first_before);
    assert_eq!(stamp(&second), second_before);
    assert_eq!(first_results.contents_stamp(), first_results_before);
    assert_eq!(persistence::save(&first.current()), saved_before);
}

#[test]
fn replacement_impact_rejects_identity_and_evidence_failures_without_mutation() {
    let mut document = seed(false);
    let results = registry(&document.current());
    let before = stamp(&document);
    let results_before = results.contents_stamp();

    assert_eq!(
        project_component_replacement_impact(
            &document,
            &results,
            ComponentReplacementImpactRequest::new(&document.current(), SELECTED, SOURCE),
        ),
        Err(ComponentReplacementImpactError::SelfReplacement(SOURCE))
    );
    let mut duplicate =
        ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET);
    duplicate.target_definition_ids.push(TARGET);
    assert_eq!(
        project_component_replacement_impact(&document, &results, duplicate),
        Err(ComponentReplacementImpactError::DuplicateTarget)
    );
    let mut cross_document =
        ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET);
    cross_document.target_document_id = DocumentId(document.current().document_id().0 + 1);
    assert!(matches!(
        project_component_replacement_impact(&document, &results, cross_document),
        Err(ComponentReplacementImpactError::CrossDocument(_, _))
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);

    let stale = ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SIBLING,
                visible: false,
            },
        ]))
        .unwrap();
    assert_eq!(
        project_component_replacement_impact(&document, &results, stale),
        Err(ComponentReplacementImpactError::Stale)
    );
}

#[test]
fn replacement_impact_fails_closed_for_hidden_failed_lost_and_incompatible_inputs() {
    let mut hidden = seed(false);
    hidden
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SELECTED,
                visible: false,
            },
        ]))
        .unwrap();
    let hidden_results = registry(&hidden.current());
    let hidden_before = stamp(&hidden);
    assert_eq!(
        project_component_replacement_impact(
            &hidden,
            &hidden_results,
            ComponentReplacementImpactRequest::new(&hidden.current(), SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Hidden(SELECTED))
    );
    assert_eq!(stamp(&hidden), hidden_before);

    let failed = seed(false);
    let failed_snapshot = failed.current();
    let source_only = ExactResultRegistry::accept(
        &failed_snapshot,
        [Arc::new(ExactBodyPackage::from(exact_package(
            &failed_snapshot,
            SOURCE,
            "source-only",
        )))],
    )
    .unwrap();
    let failed_before = stamp(&failed);
    assert_eq!(
        project_component_replacement_impact(
            &failed,
            &source_only,
            ComponentReplacementImpactRequest::new(&failed_snapshot, SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Failed(TARGET, BodyId(1)))
    );
    assert_eq!(stamp(&failed), failed_before);

    let mut lost = seed(false);
    let lost_snapshot = lost.current();
    let lost_package = exact_package(&lost_snapshot, SOURCE, "lost-reference");
    let existing_mate = lost_snapshot.assembly_mate(MATE).unwrap();
    lost.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::DeleteDrawingSheet { id: SHEET },
        CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
            MATE,
            AssemblyMateEndpoint::lost(
                SELECTED,
                lost_package.reference(ExactFaceRole::Top).unwrap().clone(),
            ),
            existing_mate.endpoint_b().clone(),
            existing_mate.kind(),
        )),
    ]))
    .unwrap();
    let lost_results = registry(&lost.current());
    let lost_before = stamp(&lost);
    assert_eq!(
        project_component_replacement_impact(
            &lost,
            &lost_results,
            ComponentReplacementImpactRequest::new(&lost.current(), SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Lost(MATE))
    );
    assert_eq!(stamp(&lost), lost_before);

    let mut incompatible = seed(false);
    incompatible
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(22),
            definition_id: TARGET,
            name: "Unmatched target feature".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
            },
        }]))
        .unwrap();
    let incompatible_results = registry(&incompatible.current());
    let incompatible_before = stamp(&incompatible);
    assert!(matches!(
        project_component_replacement_impact(
            &incompatible,
            &incompatible_results,
            ComponentReplacementImpactRequest::new(&incompatible.current(), SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Incompatible(reason))
            if reason.contains("feature counts")
    ));
    assert_eq!(stamp(&incompatible), incompatible_before);
}

#[test]
fn replacement_commit_is_one_reviewed_batch_one_undo_and_reuses_current_target_exact() {
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::DeleteAssemblyMate { id: AXIAL_MATE },
        ]))
        .unwrap();
    let source = document.current();
    let source_definition = source.definition(SOURCE).unwrap().clone();
    let target_definition = source.definition(TARGET).unwrap().clone();
    let sibling = source.occurrence(SIBLING).unwrap().clone();
    let target_occurrence = source.occurrence(TARGET_OCCURRENCE).unwrap().clone();
    let before = stamp(&document);
    let mut results = registry(&source);
    let impact = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&source, SELECTED, TARGET),
    )
    .unwrap();

    let proposal = impact.proposal.as_ref().unwrap();
    assert_eq!(proposal.batch().commands().len(), 1);
    assert!(matches!(
        proposal.batch().commands(),
        [CanonicalCommand::RepointOccurrence {
            id: SELECTED,
            definition_id: TARGET,
        }]
    ));
    let receipt = commit_component_replacement(&mut document, &mut results, &impact).unwrap();
    let replaced = document.current();

    assert_eq!(receipt.revision_id, before.revision + 1);
    assert_eq!(receipt.selected_occurrence_id, SELECTED);
    assert_eq!(
        receipt.selected_instance_path,
        impact.selected_instance_path
    );
    assert_eq!(receipt.selected_transform, impact.selected_transform);
    assert_eq!(
        receipt.reused_target_results,
        vec![(BodyId(1), "target-current".into())]
    );
    assert_eq!(
        replaced.occurrence(SELECTED).unwrap().definition_id(),
        TARGET
    );
    assert_eq!(
        replaced.occurrence(SELECTED).unwrap().transform(),
        impact.selected_transform
    );
    assert_eq!(replaced.definition(SOURCE), Some(&source_definition));
    assert_eq!(replaced.definition(TARGET), Some(&target_definition));
    assert_eq!(replaced.occurrence(SIBLING), Some(&sibling));
    assert_eq!(
        replaced.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence)
    );
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    assert_eq!(
        results
            .get_body(&replaced, TARGET, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "target-current"
    );

    let undone = document.undo().unwrap();
    assert_eq!(undone.canonical_digest(), impact.source_digest);
    assert_eq!(undone.occurrence(SELECTED).unwrap().definition_id(), SOURCE);
    assert_eq!(document.visible_redo_steps(), 1);
    let redone = document.redo().unwrap();
    assert_eq!(
        redone.canonical_digest(),
        impact.candidate_digest.as_deref().unwrap()
    );
    assert_eq!(redone.occurrence(SELECTED).unwrap().definition_id(), TARGET);
}

#[test]
fn replacement_commit_rebinds_dependencies_and_rejects_tampered_correspondence() {
    let mut dependent = seed(false);
    let mut dependent_results = registry(&dependent.current());
    let dependent_impact = project_component_replacement_impact(
        &dependent,
        &dependent_results,
        ComponentReplacementImpactRequest::new(&dependent.current(), SELECTED, TARGET),
    )
    .unwrap();
    let dependent_before = stamp(&dependent);
    let source_mates = [MATE, AXIAL_MATE]
        .map(|mate_id| dependent.current().assembly_mate(mate_id).unwrap().clone());
    assert!(matches!(
        dependent_impact
            .proposal
            .as_ref()
            .unwrap()
            .batch()
            .commands(),
        [
            CanonicalCommand::RepointOccurrence { .. },
            CanonicalCommand::RebindAssemblyMate(_),
            CanonicalCommand::RebindAssemblyMate(_)
        ]
    ));
    let receipt =
        commit_component_replacement(&mut dependent, &mut dependent_results, &dependent_impact)
            .unwrap();
    assert_eq!(receipt.rebound_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(receipt.drawings.len(), 1);
    assert!(receipt.drawings[0].is_current(&dependent.current()));
    assert_eq!(receipt.exports, dependent_impact.exports);
    assert_eq!(stamp(&dependent).revision, dependent_before.revision + 1);
    let committed = dependent.current();
    for (mate_id, source_mate) in [MATE, AXIAL_MATE].into_iter().zip(&source_mates) {
        let rebound = committed.assembly_mate(mate_id).unwrap();
        assert_eq!(rebound.endpoint_b(), source_mate.endpoint_b());
        assert_eq!(rebound.endpoint_a().occurrence_id(), SELECTED);
        assert_eq!(rebound.endpoint_a().reference().definition_id, TARGET);
        assert_eq!(
            rebound.endpoint_a().health(),
            ketchup_core::assembly::AssemblyReferenceHealth::Resolved
        );
    }

    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::DeleteAssemblyMate { id: AXIAL_MATE },
        ]))
        .unwrap();
    let mut results = registry(&document.current());
    let mut tampered = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET),
    )
    .unwrap();
    tampered.feature_correspondence.pop();
    let before = stamp(&document);
    let results_before = results.contents_stamp();
    assert_eq!(
        commit_component_replacement(&mut document, &mut results, &tampered),
        Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement impact no longer matches the complete current correspondence"
                .into(),
        ))
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);
}

#[test]
fn replacement_preserves_parented_world_placement_collections_and_drawings_atomically() {
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
                id: SELECTED,
                transform: Transform::from_translation(-2.0, -5.0, -6.0).unwrap(),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: SELECTED,
                parent: Some(GROUP),
            },
        ]))
        .unwrap();
    let source = document.current();
    let selected_before = source.occurrence(SELECTED).unwrap().clone();
    let world_before = source.world_transform_for_occurrence(SELECTED).unwrap();
    let collection_before = source.collection(COLLECTION).unwrap().clone();
    let drawing_before = source.drawing_sheet(SHEET).unwrap().clone();
    let sibling_before = source.occurrence(SIBLING).unwrap().clone();
    let target_occurrence_before = source.occurrence(TARGET_OCCURRENCE).unwrap().clone();
    let source_mates =
        [MATE, AXIAL_MATE].map(|mate_id| source.assembly_mate(mate_id).unwrap().clone());
    let before = stamp(&document);
    let mut results = registry(&source);

    let impact = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&source, SELECTED, TARGET),
    )
    .unwrap();

    assert_eq!(impact.selected_transform, world_before);
    assert_eq!(impact.collection_dependencies.len(), 1);
    assert_eq!(
        impact.collection_dependencies[0].occurrence_ids_before,
        impact.collection_dependencies[0].occurrence_ids_after
    );
    assert!(matches!(
        impact.proposal.as_ref().unwrap().batch().commands(),
        [
            CanonicalCommand::RepointOccurrence { .. },
            CanonicalCommand::RebindAssemblyMate(_),
            CanonicalCommand::RebindAssemblyMate(_)
        ]
    ));

    let receipt = commit_component_replacement(&mut document, &mut results, &impact).unwrap();
    let committed = document.current();
    let selected_after = committed.occurrence(SELECTED).unwrap();
    assert_eq!(receipt.rebound_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(selected_after.id(), selected_before.id());
    assert_eq!(selected_after.definition_id(), TARGET);
    assert_eq!(selected_after.transform(), selected_before.transform());
    assert_eq!(selected_after.parent(), selected_before.parent());
    assert_eq!(
        committed.world_transform_for_occurrence(SELECTED),
        Some(world_before)
    );
    assert_eq!(committed.collection(COLLECTION), Some(&collection_before));
    assert_eq!(committed.drawing_sheet(SHEET), Some(&drawing_before));
    assert_eq!(committed.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        committed.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    for (mate_id, source_mate) in [MATE, AXIAL_MATE].into_iter().zip(&source_mates) {
        let rebound = committed.assembly_mate(mate_id).unwrap();
        assert_eq!(rebound.endpoint_b(), source_mate.endpoint_b());
        assert_eq!(rebound.endpoint_a().occurrence_id(), SELECTED);
        assert_eq!(rebound.endpoint_a().reference().definition_id, TARGET);
    }
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    assert_eq!(receipt.drawings.len(), 1);
    assert!(receipt.drawings[0].is_current(&committed));
}

#[test]
fn conflicting_replacement_dependency_preserves_canonical_registry_and_last_valid_drawing() {
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: TARGET_OCCURRENCE,
                transform: Transform::from_translation(50.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let results = registry(&document.current());
    let before = stamp(&document);
    let results_before = results.contents_stamp();
    let saved_before = persistence::save(&document.current());
    let drawing_before = project_orthographic_drawing(
        &document.current(),
        &results,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();

    let failed = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET),
    );

    assert!(matches!(
        failed,
        Err(ComponentReplacementImpactError::Unsupported(reason))
            if reason.contains("not fully constrained")
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);
    assert_eq!(persistence::save(&document.current()), saved_before);
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &results,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        drawing_before
    );
}
