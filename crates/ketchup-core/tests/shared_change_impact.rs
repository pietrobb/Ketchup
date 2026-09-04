use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, PlanarFaceAttachment,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CanonicalError, CollectionId, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, GroupId, OccurrenceId, ProposalCommitError,
    ProposalPrincipal, StableFaceRole, Transform,
};
use ketchup_core::drawing::{
    DrawingSheet, DrawingSheetId, DrawingSource, OrthographicDrawing, OrthographicViewKind,
    project_orthographic_drawing,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactPlanarFaceAttachmentInput,
    ExactRenderPackage, ExactResultRegistry, ExactStlExport,
    build_box_render_package_with_attachments, canonical_reference_lineage_digest,
    exact_model_stl_export,
};
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationRequest, BodyParameterEditRequest, ExactParameterEdit,
    ExactParameterEditTarget,
};
use ketchup_core::persistence;
use ketchup_core::shared_change::{
    OCCURRENCE_EDIT_IMPACT_SCHEMA_V1, OccurrenceDrawingDependencyAction, OccurrenceEdit,
    OccurrenceEditImpactError, OccurrenceEditRequest, SharedChangeExportEligibility,
    SharedChangeExportFormat, SharedChangeImpactError, SharedChangePropagationError,
    SharedDefinitionChangeRequest, commit_shared_definition_change, project_occurrence_edit_impact,
    project_shared_change_impact,
};
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const OTHER_DEFINITION: DefinitionId = DefinitionId(2);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const FIRST: OccurrenceId = OccurrenceId(20);
const SECOND: OccurrenceId = OccurrenceId(21);
const OTHER: OccurrenceId = OccurrenceId(30);
const MATE: AssemblyMateId = AssemblyMateId(40);
const AXIAL_MATE: AssemblyMateId = AssemblyMateId(41);
const SHEET: DrawingSheetId = DrawingSheetId(50);
const COLLECTION: CollectionId = CollectionId(70);

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

#[test]
fn occurrence_edit_contract_binds_provenance_and_preserves_world_space_by_default() {
    let document = seed(false);
    let snapshot = document.current();
    let delete = OccurrenceEditRequest::delete(&snapshot, FIRST);
    let reparent = OccurrenceEditRequest::reparent(&snapshot, FIRST, Some(GroupId(60)));

    assert_eq!(
        OCCURRENCE_EDIT_IMPACT_SCHEMA_V1,
        "ketchup.occurrence-edit-impact.v1"
    );
    assert_eq!(delete.source_revision, snapshot.revision_id());
    assert_eq!(delete.source_digest, snapshot.canonical_digest());
    assert_eq!(delete.target_occurrence_id, FIRST);
    assert_eq!(delete.edit, OccurrenceEdit::Delete);
    assert_eq!(reparent.source_revision, snapshot.revision_id());
    assert_eq!(reparent.source_digest, snapshot.canonical_digest());
    assert_eq!(reparent.target_occurrence_id, FIRST);
    assert_eq!(
        reparent.edit,
        OccurrenceEdit::Reparent {
            parent: Some(GroupId(60)),
            preserve_world_transform: true,
        }
    );
}

#[derive(Clone, Debug, PartialEq)]
struct DerivedOutputStamp {
    package: ExactBodyPackage,
    drawing: OrthographicDrawing,
    stl: ExactStlExport,
}

fn derived_output_stamp(
    snapshot: &ketchup_core::document::Snapshot,
    exact_results: &ExactResultRegistry,
) -> DerivedOutputStamp {
    let package = exact_results
        .get_body(snapshot, DEFINITION, BodyId(1))
        .unwrap()
        .unwrap();
    DerivedOutputStamp {
        package: package.as_ref().clone(),
        drawing: project_orthographic_drawing(
            snapshot,
            exact_results,
            snapshot.drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        stl: exact_model_stl_export(snapshot, &[(package.as_ref(), Transform::identity())])
            .unwrap(),
    }
}

fn exact_package(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> ExactRenderPackage {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_body(snapshot, DEFINITION, BodyId(1)).unwrap();
    let evidence = |role: ExactFaceRole| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:{fingerprint}"),
        )
    };
    let attachments = [
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
    ];
    build_box_render_package_with_attachments(
        &request,
        format!("exact-input:{fingerprint}"),
        fingerprint.to_owned(),
        "occt".into(),
        "r0".into(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ]
        .map(evidence),
        &attachments,
    )
    .unwrap()
}

fn planar_endpoint(
    package: &ExactRenderPackage,
    occurrence_id: OccurrenceId,
    role: ExactFaceRole,
) -> AssemblyMateEndpoint {
    let reference = package.reference(role).unwrap();
    AssemblyMateEndpoint::resolved_planar_face(
        occurrence_id,
        package.planar_face_attachment(reference).unwrap().clone(),
    )
}

fn seed(reverse_occurrences: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let first = CanonicalCommand::CreateOccurrence {
        id: FIRST,
        definition_id: DEFINITION,
        name: "Visible reuse".into(),
        transform: Transform::identity(),
        parent: None,
        tag: None,
        visible: true,
    };
    let second = CanonicalCommand::CreateOccurrence {
        id: SECOND,
        definition_id: DEFINITION,
        name: "Hidden reuse".into(),
        transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: false,
    };
    let occurrences = if reverse_occurrences {
        vec![second, first]
    } else {
        vec![first, second]
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
    commands.extend(occurrences);
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

    let evidence = exact_package(&document.current(), "reference");
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                planar_endpoint(&evidence, FIRST, ExactFaceRole::Top),
                planar_endpoint(&evidence, SECOND, ExactFaceRole::Bottom),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateDrawingSheet(
                DrawingSheet::new(SHEET, "Shared part", DrawingSource::Definition(DEFINITION))
                    .unwrap(),
            ),
        ]))
        .unwrap();
    document
}

fn seed_rigid_dependencies() -> DocumentStore {
    let mut document = seed(false);
    let evidence = exact_package(&document.current(), "rigid-dependencies");
    let east = evidence.reference(ExactFaceRole::East).unwrap().clone();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SECOND,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SECOND,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved_planar_face(
                    FIRST,
                    PlanarFaceAttachment::new(east.clone(), [0.0; 3], [1.0, 0.0, 0.0]).unwrap(),
                ),
                AssemblyMateEndpoint::resolved_planar_face(
                    SECOND,
                    PlanarFaceAttachment::new(east, [0.0; 3], [1.0, 0.0, 0.0]).unwrap(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 20.0,
                    reversed: true,
                },
            )),
            CanonicalCommand::UpdateDrawingSheet(
                DrawingSheet::new(
                    SHEET,
                    "Rigid shared assembly",
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

#[test]
fn dependency_aware_delete_is_one_reviewed_atomic_proposal() {
    let mut document = seed_rigid_dependencies();
    document
        .apply_batch(&CommandBatch::new(vec![
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
    let source = document.current();
    let before = stamp(&document);
    let unrelated_before = source.occurrence(OTHER).unwrap().clone();

    let impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::delete(&source, FIRST),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(stamp(&document), before);
    assert!(impact.is_review_only());
    assert_eq!(impact.schema, OCCURRENCE_EDIT_IMPACT_SCHEMA_V1);
    assert_eq!(impact.source_revision, source.revision_id());
    assert_eq!(impact.source_digest, source.canonical_digest());
    assert_ne!(impact.candidate_digest, impact.source_digest);
    assert_eq!(impact.target_occurrence_id, FIRST);
    assert_eq!(impact.target_instance_path.root_occurrence(), FIRST);
    assert_eq!(impact.edit, OccurrenceEdit::Delete);
    assert_eq!(impact.parent_before, None);
    assert_eq!(impact.parent_after, None);
    assert_eq!(impact.local_transform_before, Transform::identity());
    assert_eq!(impact.local_transform_after, None);
    assert_eq!(impact.world_transform_before, Transform::identity());
    assert_eq!(impact.world_transform_after, None);
    assert_eq!(impact.incident_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(impact.collection_dependencies.len(), 1);
    assert_eq!(impact.collection_dependencies[0].collection_id, COLLECTION);
    assert_eq!(
        impact.collection_dependencies[0].occurrence_ids_before,
        vec![FIRST, SECOND, OTHER]
    );
    assert_eq!(
        impact.collection_dependencies[0].occurrence_ids_after,
        vec![SECOND, OTHER]
    );
    assert_eq!(impact.drawing_dependencies.len(), 1);
    assert_eq!(impact.drawing_dependencies[0].sheet_id, SHEET);
    assert_eq!(
        impact.drawing_dependencies[0].action,
        OccurrenceDrawingDependencyAction::UpdateRigidAssembly {
            occurrence_ids: vec![SECOND],
        }
    );

    let commands = impact.proposal.batch().commands();
    assert_eq!(commands.len(), 5);
    assert!(matches!(
        commands[0],
        CanonicalCommand::DeleteAssemblyMate { id: MATE }
    ));
    assert!(matches!(
        commands[1],
        CanonicalCommand::DeleteAssemblyMate { id: AXIAL_MATE }
    ));
    assert!(matches!(
        &commands[2],
        CanonicalCommand::SetCollectionOccurrences { id: COLLECTION, occurrence_ids }
            if occurrence_ids == &vec![SECOND, OTHER]
    ));
    assert!(matches!(
        &commands[3],
        CanonicalCommand::UpdateDrawingSheet(sheet)
            if sheet.id() == SHEET
                && sheet.source() == &DrawingSource::RigidAssembly {
                    occurrence_ids: vec![SECOND],
                }
    ));
    assert!(matches!(
        commands[4],
        CanonicalCommand::DeleteOccurrence { id: FIRST }
    ));

    document.commit_proposal(&impact.proposal).unwrap();
    let committed = document.current();
    assert_eq!(committed.canonical_digest(), impact.candidate_digest);
    assert!(committed.occurrence(FIRST).is_none());
    assert_eq!(committed.occurrence(OTHER), Some(&unrelated_before));
    assert!(committed.assembly_mate(MATE).is_none());
    assert!(committed.assembly_mate(AXIAL_MATE).is_none());
    assert_eq!(
        committed
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![SECOND, OTHER]
    );
    assert_eq!(
        committed.drawing_sheet(SHEET).unwrap().source(),
        &DrawingSource::RigidAssembly {
            occurrence_ids: vec![SECOND],
        }
    );
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
}

#[test]
fn dependency_aware_delete_verifier_covers_cancel_stale_persistence_and_undo_redo() {
    let mut document = seed_rigid_dependencies();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Persistent selection".into(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![FIRST, SECOND, OTHER],
            },
        ]))
        .unwrap();
    let source = document.current();
    let cancelled_before = stamp(&document);
    let bytes_before_cancel = persistence::save(&source);
    let stale_request = OccurrenceEditRequest::delete(&source, FIRST);
    let cancelled = project_occurrence_edit_impact(
        &document,
        stale_request.clone(),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    drop(cancelled);
    assert_eq!(stamp(&document), cancelled_before);
    assert_eq!(persistence::save(&document.current()), bytes_before_cancel);

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
        project_occurrence_edit_impact(&document, stale_request, ProposalPrincipal::LocalAssistant,),
        Err(OccurrenceEditImpactError::Stale)
    );
    assert_eq!(stamp(&document), changed);

    let stale_impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::delete(&document.current(), FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OTHER,
                visible: true,
            },
        ]))
        .unwrap();
    let before_stale_commit = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale_impact.proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale_commit);

    let impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::delete(&document.current(), FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let before_commit = stamp(&document);
    document.commit_proposal(&impact.proposal).unwrap();
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_eq!(committed_digest, impact.candidate_digest);
    assert_eq!(document.revision_count(), before_commit.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before_commit.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);

    let reopened = persistence::load(&persistence::save(&committed))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened = reopened.current();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(reopened.occurrence(FIRST).is_none());
    assert!(reopened.assembly_mate(MATE).is_none());
    assert!(reopened.assembly_mate(AXIAL_MATE).is_none());
    assert_eq!(
        reopened
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![SECOND, OTHER]
    );
    assert_eq!(
        reopened.drawing_sheet(SHEET).unwrap().source(),
        &DrawingSource::RigidAssembly {
            occurrence_ids: vec![SECOND],
        }
    );

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before_commit.digest);
    assert_eq!(document.visible_undo_steps(), before_commit.undo);
    assert_eq!(document.visible_redo_steps(), 1);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert_eq!(document.visible_undo_steps(), before_commit.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
}

#[test]
fn dependency_aware_delete_removes_a_drawing_with_an_empty_reduced_source() {
    let mut document = seed(false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::UpdateDrawingSheet(
                DrawingSheet::new(
                    SHEET,
                    "Single rigid source",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![FIRST],
                    },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    let source = document.current();

    let impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::delete(&source, FIRST),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    assert!(impact.incident_mate_ids.is_empty());
    assert_eq!(impact.drawing_dependencies.len(), 1);
    assert_eq!(impact.drawing_dependencies[0].sheet_id, SHEET);
    assert_eq!(
        impact.drawing_dependencies[0].action,
        OccurrenceDrawingDependencyAction::DeleteSheet
    );
    assert!(matches!(
        impact.proposal.batch().commands()[0],
        CanonicalCommand::DeleteDrawingSheet { id: SHEET }
    ));
    assert!(matches!(
        impact.proposal.batch().commands()[1],
        CanonicalCommand::DeleteOccurrence { id: FIRST }
    ));

    document.commit_proposal(&impact.proposal).unwrap();
    assert!(document.current().drawing_sheet(SHEET).is_none());
    assert!(document.current().occurrence(FIRST).is_none());
    assert!(document.current().occurrence(SECOND).is_some());
}

#[test]
fn dependency_aware_reparent_preserves_world_transform_mates_and_collections() {
    let parent = GroupId(60);
    let nested_parent = GroupId(61);
    let mut document = seed_rigid_dependencies();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: parent,
                name: "Rotated assembly".into(),
                transform: Transform::from_matrix([
                    0.0, -1.0, 0.0, 100.0, 1.0, 0.0, 0.0, 10.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: nested_parent,
                name: "Nested assembly".into(),
                transform: Transform::from_translation(-20.0, 40.0, 0.0).unwrap(),
                parent: Some(parent),
            },
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Preserved selection".into(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![FIRST, SECOND, OTHER],
            },
        ]))
        .unwrap();
    let source = document.current();
    let before = stamp(&document);
    let mate_before = source.assembly_mate(MATE).unwrap().clone();
    let axial_mate_before = source.assembly_mate(AXIAL_MATE).unwrap().clone();
    let drawing_before = source.drawing_sheet(SHEET).unwrap().clone();
    let unrelated_before = source.occurrence(OTHER).unwrap().clone();
    let world_before = source.world_transform_for_occurrence(FIRST).unwrap();
    let expected_local = Transform::from_matrix([
        0.0, 1.0, 0.0, 10.0, -1.0, 0.0, 0.0, 60.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();

    let impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::reparent(&source, FIRST, Some(nested_parent)),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(stamp(&document), before);
    assert!(impact.is_review_only());
    assert_eq!(impact.parent_before, None);
    assert_eq!(impact.parent_after, Some(nested_parent));
    assert_eq!(impact.local_transform_before, Transform::identity());
    assert_eq!(impact.local_transform_after, Some(expected_local));
    assert_eq!(impact.world_transform_before, world_before);
    assert_eq!(impact.world_transform_after, Some(world_before));
    assert_eq!(impact.incident_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(impact.collection_dependencies.len(), 1);
    assert_eq!(
        impact.collection_dependencies[0].occurrence_ids_before,
        vec![FIRST, SECOND, OTHER]
    );
    assert_eq!(
        impact.collection_dependencies[0].occurrence_ids_after,
        vec![FIRST, SECOND, OTHER]
    );
    assert!(impact.drawing_dependencies.is_empty());
    assert_eq!(impact.proposal.batch().commands().len(), 2);
    assert!(matches!(
        impact.proposal.batch().commands()[0],
        CanonicalCommand::SetOccurrenceTransform {
            id: FIRST,
            transform,
        } if transform == expected_local
    ));
    assert!(matches!(
        impact.proposal.batch().commands()[1],
        CanonicalCommand::SetOccurrenceParent {
            id: FIRST,
            parent: Some(id),
        } if id == nested_parent
    ));

    document.commit_proposal(&impact.proposal).unwrap();
    let committed = document.current();
    assert_eq!(committed.canonical_digest(), impact.candidate_digest);
    assert_eq!(
        committed.occurrence(FIRST).unwrap().parent(),
        Some(nested_parent)
    );
    assert_eq!(
        committed.occurrence(FIRST).unwrap().transform(),
        expected_local
    );
    assert_eq!(
        committed.world_transform_for_occurrence(FIRST),
        Some(world_before)
    );
    assert_eq!(committed.assembly_mate(MATE), Some(&mate_before));
    assert_eq!(
        committed.assembly_mate(AXIAL_MATE),
        Some(&axial_mate_before)
    );
    assert_eq!(committed.drawing_sheet(SHEET), Some(&drawing_before));
    assert_eq!(committed.occurrence(OTHER), Some(&unrelated_before));
    assert_eq!(
        committed
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND, OTHER]
    );
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
}

#[test]
fn dependency_aware_reparent_verifier_covers_failures_persistence_and_undo_redo() {
    let parent = GroupId(62);
    let nested_parent = GroupId(63);
    let singular_parent = GroupId(64);
    let missing_parent = GroupId(999);
    let mut document = seed_rigid_dependencies();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: parent,
                name: "Verifier root".into(),
                transform: Transform::from_matrix([
                    0.0, -1.0, 0.0, 75.0, 1.0, 0.0, 0.0, -25.0, 0.0, 0.0, 1.0, 5.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: nested_parent,
                name: "Verifier nested".into(),
                transform: Transform::from_translation(15.0, 20.0, -5.0).unwrap(),
                parent: Some(parent),
            },
            CanonicalCommand::CreateGroup {
                id: singular_parent,
                name: "Non-invertible parent".into(),
                transform: Transform::from_matrix([
                    0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Verifier selection".into(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: COLLECTION,
                occurrence_ids: vec![FIRST, SECOND, OTHER],
            },
        ]))
        .unwrap();

    let before_cycle = stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetGroupParent {
            id: parent,
            parent: Some(nested_parent),
        }])),
        Err(CanonicalError::GroupCycle(id)) if id == parent
    ));
    assert_eq!(stamp(&document), before_cycle);

    let source = document.current();
    let before_invalid = stamp(&document);
    assert_eq!(
        project_occurrence_edit_impact(
            &document,
            OccurrenceEditRequest::reparent(&source, FIRST, Some(missing_parent)),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceEditImpactError::ParentNotFound(missing_parent))
    );
    assert_eq!(stamp(&document), before_invalid);
    assert_eq!(
        project_occurrence_edit_impact(
            &document,
            OccurrenceEditRequest::reparent(&source, FIRST, Some(singular_parent)),
            ProposalPrincipal::ManualClient,
        ),
        Err(OccurrenceEditImpactError::NonInvertibleParent(
            singular_parent
        ))
    );
    assert_eq!(stamp(&document), before_invalid);

    let stale_request = OccurrenceEditRequest::reparent(&source, FIRST, Some(nested_parent));
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OTHER,
                visible: false,
            },
        ]))
        .unwrap();
    let before_stale_request = stamp(&document);
    assert_eq!(
        project_occurrence_edit_impact(&document, stale_request, ProposalPrincipal::LocalAssistant,),
        Err(OccurrenceEditImpactError::Stale)
    );
    assert_eq!(stamp(&document), before_stale_request);

    let stale_impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::reparent(&document.current(), FIRST, Some(nested_parent)),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: parent,
                transform: Transform::from_translation(1.0, 2.0, 3.0).unwrap(),
            },
        ]))
        .unwrap();
    let before_stale_commit = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale_impact.proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale_commit);

    let source = document.current();
    let world_before = source.world_transform_for_occurrence(FIRST).unwrap();
    let mate_before = source.assembly_mate(MATE).unwrap().clone();
    let axial_mate_before = source.assembly_mate(AXIAL_MATE).unwrap().clone();
    let drawing_before = source.drawing_sheet(SHEET).unwrap().clone();
    let before_commit = stamp(&document);
    let impact = project_occurrence_edit_impact(
        &document,
        OccurrenceEditRequest::reparent(&source, FIRST, Some(nested_parent)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    document.commit_proposal(&impact.proposal).unwrap();
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_eq!(committed_digest, impact.candidate_digest);
    assert_eq!(document.revision_count(), before_commit.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before_commit.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);

    let reopened = persistence::load(&persistence::save(&committed))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened = reopened.current();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert_eq!(
        reopened.occurrence(FIRST).unwrap().parent(),
        Some(nested_parent)
    );
    assert_eq!(
        reopened.world_transform_for_occurrence(FIRST),
        Some(world_before)
    );
    assert_eq!(reopened.assembly_mate(MATE), Some(&mate_before));
    assert_eq!(reopened.assembly_mate(AXIAL_MATE), Some(&axial_mate_before));
    assert_eq!(reopened.drawing_sheet(SHEET), Some(&drawing_before));
    assert_eq!(
        reopened
            .collection(COLLECTION)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND, OTHER]
    );

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before_commit.digest);
    assert_eq!(document.visible_undo_steps(), before_commit.undo);
    assert_eq!(document.visible_redo_steps(), 1);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert_eq!(document.visible_undo_steps(), before_commit.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
}

fn edit_request(snapshot: &ketchup_core::document::Snapshot) -> SharedDefinitionChangeRequest {
    SharedDefinitionChangeRequest::exact_parameter_edit(
        snapshot,
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

fn registry(snapshot: &ketchup_core::document::Snapshot) -> ExactResultRegistry {
    ExactResultRegistry::accept(
        snapshot,
        [Arc::new(ExactBodyPackage::from(exact_package(
            snapshot,
            "last-valid",
        )))],
    )
    .unwrap()
}

fn history_package(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> ExactRenderPackage {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_body(snapshot, DEFINITION, BodyId(1)).unwrap();
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
    let attachments = [
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
    ];
    let package = if request.pocket_depth_bits.is_some() {
        build_box_render_package_with_attachments(
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
            &attachments,
        )
    } else {
        build_box_render_package_with_attachments(
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
            &attachments,
        )
    };
    package.unwrap()
}

fn history_registry(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> ExactResultRegistry {
    ExactResultRegistry::accept(
        snapshot,
        [Arc::new(ExactBodyPackage::from(history_package(
            snapshot,
            fingerprint,
        )))],
    )
    .unwrap()
}

#[test]
fn reused_definition_impact_is_complete_stable_and_non_mutating() {
    let first = seed(false);
    let second = seed(true);
    let first_registry = registry(&first.current());
    let second_registry = registry(&second.current());
    let first_before = stamp(&first);
    let second_before = stamp(&second);
    let first_registry_before = first_registry.contents_stamp();

    let first_impact = project_shared_change_impact(
        &first,
        &first_registry,
        edit_request(&first.current()),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let second_impact = project_shared_change_impact(
        &second,
        &second_registry,
        edit_request(&second.current()),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();

    assert_eq!(first_impact.occurrences, second_impact.occurrences);
    assert_eq!(
        first_impact.affected_body_ids,
        second_impact.affected_body_ids
    );
    assert_eq!(
        first_impact.affected_feature_ids,
        second_impact.affected_feature_ids
    );
    assert_eq!(
        first_impact.unchanged_body_ids,
        second_impact.unchanged_body_ids
    );
    assert_eq!(
        first_impact.unchanged_definition_ids,
        second_impact.unchanged_definition_ids
    );
    assert_eq!(
        first_impact
            .mate_references
            .iter()
            .map(|reference| (reference.mate_id, reference.occurrence_id))
            .collect::<Vec<_>>(),
        second_impact
            .mate_references
            .iter()
            .map(|reference| (reference.mate_id, reference.occurrence_id))
            .collect::<Vec<_>>()
    );
    assert_eq!(first_impact.drawing_views, second_impact.drawing_views);
    assert_eq!(first_impact.exports, second_impact.exports);
    assert_eq!(
        first_impact.proposal.command_digest(),
        second_impact.proposal.command_digest()
    );
    assert_eq!(first_impact.definition_id, DEFINITION);
    assert_eq!(first_impact.affected_body_ids, vec![BodyId(1)]);
    assert_eq!(first_impact.affected_feature_ids, vec![EXTRUSION]);
    assert!(first_impact.unchanged_body_ids.is_empty());
    assert_eq!(
        first_impact.unchanged_definition_ids,
        vec![OTHER_DEFINITION]
    );
    assert_eq!(
        first_impact
            .occurrences
            .iter()
            .map(|impact| (impact.occurrence_id, impact.visible))
            .collect::<Vec<_>>(),
        vec![(FIRST, true), (SECOND, false)]
    );
    assert_eq!(first_impact.exact_jobs.len(), 1);
    assert_eq!(first_impact.exact_jobs[0].definition_id, DEFINITION);
    assert_eq!(first_impact.exact_jobs[0].body_id, BodyId(1));
    assert_eq!(first_impact.exact_jobs[0].producer_feature_id, EXTRUSION);
    assert_eq!(
        first_impact.exact_jobs[0].last_valid_result_fingerprint,
        "last-valid"
    );
    assert_eq!(first_impact.mate_references.len(), 2);
    assert!(
        first_impact
            .mate_references
            .iter()
            .all(|reference| reference.mate_id == MATE
                && reference.definition_id == DEFINITION
                && reference.producer_feature_id == EXTRUSION)
    );
    assert_eq!(
        first_impact.drawing_views,
        vec![
            ketchup_core::shared_change::SharedChangeDrawingViewImpact {
                sheet_id: SHEET,
                view: OrthographicViewKind::Front,
            },
            ketchup_core::shared_change::SharedChangeDrawingViewImpact {
                sheet_id: SHEET,
                view: OrthographicViewKind::Top,
            },
            ketchup_core::shared_change::SharedChangeDrawingViewImpact {
                sheet_id: SHEET,
                view: OrthographicViewKind::Right,
            },
        ]
    );
    assert_eq!(
        first_impact
            .exports
            .iter()
            .map(|export| (
                export.format,
                export.eligibility,
                export.occurrence_paths.len()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                SharedChangeExportFormat::Step,
                SharedChangeExportEligibility::PendingExactRecompute,
                1,
            ),
            (
                SharedChangeExportFormat::Stl,
                SharedChangeExportEligibility::PendingExactRecompute,
                1,
            ),
        ]
    );
    assert_ne!(first_impact.candidate_digest, first_impact.source_digest);
    assert_eq!(stamp(&first), first_before);
    assert_eq!(stamp(&second), second_before);
    assert_eq!(first_registry.contents_stamp(), first_registry_before);
}

#[test]
fn stale_failed_ambiguous_and_unsupported_inputs_fail_without_mutation() {
    let mut document = seed(false);
    let snapshot = document.current();
    let current_registry = registry(&snapshot);
    let request = edit_request(&snapshot);
    let before = stamp(&document);

    assert_eq!(
        project_shared_change_impact(
            &document,
            &ExactResultRegistry::default(),
            request.clone(),
            ProposalPrincipal::ManualClient,
        ),
        Err(SharedChangeImpactError::Failed(BodyId(1)))
    );
    assert_eq!(stamp(&document), before);

    let package = exact_package(&snapshot, "ambiguous-a");
    let mut alternate = package.clone();
    alternate.identity.backend = "alternate".into();
    for reference in &mut alternate.references {
        reference.backend = alternate.identity.backend.clone();
    }
    alternate.planar_face_attachments = alternate
        .planar_face_attachments
        .iter()
        .map(|attachment| {
            let reference = alternate
                .references
                .iter()
                .find(|reference| reference.lineage_digest == attachment.reference().lineage_digest)
                .unwrap()
                .clone();
            PlanarFaceAttachment::new(
                reference,
                attachment.local_origin_mm(),
                attachment.local_unit_normal(),
            )
            .unwrap()
        })
        .collect();
    let ambiguous = ExactResultRegistry::accept(
        &snapshot,
        [
            Arc::new(ExactBodyPackage::from(package)),
            Arc::new(ExactBodyPackage::from(alternate)),
        ],
    )
    .unwrap();
    assert_eq!(
        project_shared_change_impact(
            &document,
            &ambiguous,
            request.clone(),
            ProposalPrincipal::ManualClient,
        ),
        Err(SharedChangeImpactError::Ambiguous(BodyId(1)))
    );
    assert_eq!(stamp(&document), before);

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
        project_shared_change_impact(
            &document,
            &current_registry,
            request,
            ProposalPrincipal::ManualClient,
        ),
        Err(SharedChangeImpactError::Stale)
    );
    assert_eq!(stamp(&document), changed);

    let current = document.current();
    let current_registry = registry(&current);
    let unsupported = SharedDefinitionChangeRequest::exact_parameter_edit(
        &current,
        BodyParameterEditRequest {
            definition_id: OTHER_DEFINITION,
            body_id: BodyId(1),
            edits: vec![ExactParameterEdit {
                target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
                dimension: Dimension::from_decimal("20").unwrap(),
            }],
        },
    );
    assert!(matches!(
        project_shared_change_impact(
            &document,
            &current_registry,
            unsupported,
            ProposalPrincipal::LocalAssistant,
        ),
        Err(SharedChangeImpactError::Unsupported(_))
    ));
    assert_eq!(stamp(&document), changed);
}

#[test]
fn preview_cancel_and_save_open_preserve_identity_and_last_valid_outputs() {
    let document = seed(false);
    let snapshot = document.current();
    let exact_results = registry(&snapshot);
    let before = stamp(&document);
    let registry_before = exact_results.contents_stamp();
    let bytes_before = persistence::save(&snapshot);
    let drawing_before = project_orthographic_drawing(
        &snapshot,
        &exact_results,
        snapshot.drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    let package = exact_results
        .get_body(&snapshot, DEFINITION, BodyId(1))
        .unwrap()
        .unwrap();
    let stl_before =
        exact_model_stl_export(&snapshot, &[(package.as_ref(), Transform::identity())]).unwrap();

    let impact = project_shared_change_impact(
        &document,
        &exact_results,
        edit_request(&snapshot),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(impact.source_revision, snapshot.revision_id());
    assert_eq!(impact.source_digest, snapshot.canonical_digest());
    assert_eq!(impact.definition_id, DEFINITION);
    assert_eq!(impact.exact_jobs[0].body_id, BodyId(1));
    assert_eq!(impact.exact_jobs[0].producer_feature_id, EXTRUSION);
    assert!(
        impact
            .occurrences
            .iter()
            .all(|value| value.instance_path.root_occurrence() == value.occurrence_id)
    );
    assert!(
        impact
            .mate_references
            .iter()
            .all(|value| !value.lineage_digest.is_empty())
    );

    let impact_before_open = impact.clone();
    drop(impact);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), registry_before);
    assert_eq!(persistence::save(&document.current()), bytes_before);
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &exact_results,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        drawing_before
    );
    assert_eq!(
        exact_model_stl_export(
            &document.current(),
            &[(package.as_ref(), Transform::identity())],
        )
        .unwrap(),
        stl_before
    );

    let reopened = persistence::load(&bytes_before)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_before = stamp(&reopened);
    let reopened_impact = project_shared_change_impact(
        &reopened,
        &exact_results,
        edit_request(&reopened.current()),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(reopened_impact, impact_before_open);
    assert_eq!(reopened_impact.source_revision, snapshot.revision_id());
    assert_eq!(reopened_impact.source_digest, snapshot.canonical_digest());
    assert_eq!(reopened_impact.definition_id, DEFINITION);
    assert_eq!(reopened_impact.affected_body_ids, vec![BodyId(1)]);
    assert_eq!(
        reopened_impact
            .occurrences
            .iter()
            .map(|value| (&value.instance_path, value.visible))
            .collect::<Vec<_>>(),
        vec![
            (&ketchup_core::document::InstancePath::root(FIRST), true),
            (&ketchup_core::document::InstancePath::root(SECOND), false),
        ]
    );
    assert_eq!(stamp(&reopened), reopened_before);
    assert_eq!(exact_results.contents_stamp(), registry_before);
}

#[test]
fn hidden_lost_and_ambiguous_mate_inputs_are_observational() {
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
    let hidden_results = registry(&hidden_snapshot);
    let hidden_before = stamp(&hidden);
    let hidden_results_before = hidden_results.contents_stamp();
    let hidden_impact = project_shared_change_impact(
        &hidden,
        &hidden_results,
        edit_request(&hidden_snapshot),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    assert_eq!(hidden_impact.occurrences.len(), 2);
    assert!(hidden_impact.occurrences.iter().all(|value| !value.visible));
    assert!(hidden_impact.exports.is_empty());
    assert_eq!(stamp(&hidden), hidden_before);
    assert_eq!(hidden_results.contents_stamp(), hidden_results_before);

    let mut invalid = seed(false);
    let references = exact_package(&invalid.current(), "mate-health");
    let top = references.reference(ExactFaceRole::Top).unwrap().clone();
    invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::lost(FIRST, top.clone()),
                planar_endpoint(&references, SECOND, ExactFaceRole::Bottom),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
        ]))
        .unwrap();
    let lost_snapshot = invalid.current();
    let lost_results = registry(&lost_snapshot);
    let lost_before = stamp(&invalid);
    let lost_results_before = lost_results.contents_stamp();
    let lost_outputs_before = derived_output_stamp(&lost_snapshot, &lost_results);
    assert_eq!(
        project_shared_change_impact(
            &invalid,
            &lost_results,
            edit_request(&lost_snapshot),
            ProposalPrincipal::ManualClient,
        ),
        Err(SharedChangeImpactError::Lost(MATE))
    );
    assert_eq!(stamp(&invalid), lost_before);
    assert_eq!(lost_results.contents_stamp(), lost_results_before);
    assert_eq!(
        derived_output_stamp(&invalid.current(), &lost_results),
        lost_outputs_before
    );

    invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::ambiguous(FIRST, top, 2),
                planar_endpoint(&references, SECOND, ExactFaceRole::Bottom),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
        ]))
        .unwrap();
    let ambiguous_snapshot = invalid.current();
    let ambiguous_results = registry(&ambiguous_snapshot);
    let ambiguous_before = stamp(&invalid);
    let ambiguous_results_before = ambiguous_results.contents_stamp();
    let ambiguous_outputs_before = derived_output_stamp(&ambiguous_snapshot, &ambiguous_results);
    assert_eq!(
        project_shared_change_impact(
            &invalid,
            &ambiguous_results,
            edit_request(&ambiguous_snapshot),
            ProposalPrincipal::LocalAssistant,
        ),
        Err(SharedChangeImpactError::Ambiguous(BodyId(1)))
    );
    assert_eq!(stamp(&invalid), ambiguous_before);
    assert_eq!(ambiguous_results.contents_stamp(), ambiguous_results_before);
    assert_eq!(
        derived_output_stamp(&invalid.current(), &ambiguous_results),
        ambiguous_outputs_before
    );
}

#[test]
fn suffix_suppress_resume_and_invalid_dependency_inputs_are_bounded() {
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
    let active = document.current();
    let active_results = history_registry(&active, "last-valid-pocket");
    let active_before = stamp(&document);
    let active_results_before = active_results.contents_stamp();
    let suppress = SharedDefinitionChangeRequest::body_history_mutation(
        &active,
        BodyHistoryMutationRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            mutation: BodyHistoryMutation::SuppressFrom(CUT_PROFILE),
        },
    );
    let suppress_impact = project_shared_change_impact(
        &document,
        &active_results,
        suppress,
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(
        suppress_impact.affected_feature_ids,
        vec![CUT_PROFILE, POCKET]
    );
    assert_eq!(suppress_impact.exact_jobs.len(), 1);
    assert_eq!(suppress_impact.exact_jobs[0].producer_feature_id, EXTRUSION);
    assert_eq!(
        suppress_impact.exact_jobs[0].last_valid_result_fingerprint,
        "last-valid-pocket"
    );
    assert_eq!(stamp(&document), active_before);
    assert_eq!(active_results.contents_stamp(), active_results_before);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                suppressed_feature_ids: vec![CUT_PROFILE, POCKET],
            },
        ]))
        .unwrap();
    let suppressed = document.current();
    let suppressed_results = history_registry(&suppressed, "last-valid-base");
    let suppressed_before = stamp(&document);
    let suppressed_results_before = suppressed_results.contents_stamp();
    let resume = SharedDefinitionChangeRequest::body_history_mutation(
        &suppressed,
        BodyHistoryMutationRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            mutation: BodyHistoryMutation::Resume,
        },
    );
    let resume_impact = project_shared_change_impact(
        &document,
        &suppressed_results,
        resume,
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    assert_eq!(
        resume_impact.affected_feature_ids,
        vec![CUT_PROFILE, POCKET]
    );
    assert_eq!(resume_impact.exact_jobs[0].producer_feature_id, POCKET);
    assert_eq!(
        resume_impact.exact_jobs[0].last_valid_result_fingerprint,
        "last-valid-base"
    );
    assert_eq!(stamp(&document), suppressed_before);
    assert_eq!(
        suppressed_results.contents_stamp(),
        suppressed_results_before
    );

    let mut not_reused = seed(false);
    not_reused
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::DeleteOccurrence { id: SECOND },
        ]))
        .unwrap();
    let not_reused_snapshot = not_reused.current();
    let not_reused_results = registry(&not_reused_snapshot);
    let not_reused_before = stamp(&not_reused);
    assert_eq!(
        project_shared_change_impact(
            &not_reused,
            &not_reused_results,
            edit_request(&not_reused_snapshot),
            ProposalPrincipal::ManualClient,
        ),
        Err(SharedChangeImpactError::DefinitionNotReused(DEFINITION))
    );
    assert_eq!(stamp(&not_reused), not_reused_before);

    const CYCLIC_DEFINITION: DefinitionId = DefinitionId(70);
    const FIRST_SHELL: FeatureId = FeatureId(300);
    const SECOND_SHELL: FeatureId = FeatureId(301);
    let mut cyclic = DocumentStore::new();
    let cyclic_before = stamp(&cyclic);
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
        Ok(_) => panic!("cyclic dependency was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        cycle_error,
        CanonicalError::FeatureDependencyCycle(FIRST_SHELL)
    );
    assert_eq!(stamp(&cyclic), cyclic_before);
}

#[test]
fn reviewed_shared_definition_change_commits_once_and_refreshes_every_reuse() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source);
    let before = stamp(&document);
    let before_registry_stamp = exact_results.contents_stamp();
    let unrelated_definition = source.definition(OTHER_DEFINITION).unwrap().clone();
    let source_occurrences = source
        .scene_query()
        .into_iter()
        .map(|occurrence| (occurrence.instance_path, occurrence.transform))
        .collect::<Vec<_>>();
    let impact = project_shared_change_impact(
        &document,
        &exact_results,
        edit_request(&source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &candidate,
        "shared-change",
    )));
    let expected_fingerprint = evaluated.result_key().result_fingerprint;
    let mut evaluations = 0;

    let receipt = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &impact,
        |request| -> Result<Arc<ExactBodyPackage>, String> {
            evaluations += 1;
            assert_eq!(request.definition_id, DEFINITION);
            assert_eq!(request.producer_feature_id(), EXTRUSION);
            Ok(Arc::clone(&evaluated))
        },
    )
    .unwrap();

    assert_eq!(evaluations, 1);
    assert_eq!(document.revision_count(), before.revisions + 1);
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    assert_eq!(
        document.current().canonical_digest(),
        impact.candidate_digest
    );
    assert_ne!(exact_results.contents_stamp(), before_registry_stamp);
    assert_eq!(receipt.revision_id, document.current().revision_id());
    assert_eq!(receipt.definition_id, DEFINITION);
    assert_eq!(receipt.body_id, BodyId(1));
    assert_eq!(receipt.affected_feature_ids, vec![EXTRUSION]);
    assert_eq!(receipt.unchanged_definition_ids, vec![OTHER_DEFINITION]);
    assert_eq!(receipt.rebound_mate_ids, vec![MATE]);
    assert_eq!(receipt.drawings.len(), 1);
    assert!(receipt.drawings[0].is_current(&document.current()));
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
    assert_eq!(receipt.occurrences.len(), 2);
    assert!(
        receipt
            .occurrences
            .iter()
            .all(
                |occurrence| occurrence.result_fingerprint == expected_fingerprint
                    && !occurrence.subshape_lineage_digests.is_empty()
            )
    );
    assert_eq!(
        receipt
            .occurrences
            .iter()
            .map(|occurrence| (occurrence.instance_path.clone(), occurrence.transform))
            .collect::<Vec<_>>(),
        source_occurrences
            .iter()
            .filter(|(path, _)| { matches!(path.root_occurrence(), FIRST | SECOND) })
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        document.current().definition(OTHER_DEFINITION),
        Some(&unrelated_definition)
    );
    assert_eq!(
        document
            .current()
            .scene_query()
            .into_iter()
            .map(|occurrence| (occurrence.instance_path, occurrence.transform))
            .collect::<Vec<_>>(),
        source_occurrences
    );
    assert_eq!(
        exact_results
            .get_body(&document.current(), DEFINITION, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        expected_fingerprint
    );
    let committed = document.current();
    let rebound_mate = committed.assembly_mate(MATE).unwrap();
    assert!(
        [rebound_mate.endpoint_a(), rebound_mate.endpoint_b()]
            .into_iter()
            .all(|endpoint| endpoint.reference().result_fingerprint == expected_fingerprint)
    );

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before.digest);
    assert_eq!(document.visible_undo_steps(), before.undo);
}

#[test]
fn shared_definition_evaluation_and_publication_fail_atomically() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source);
    let impact = project_shared_change_impact(
        &document,
        &exact_results,
        edit_request(&source),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let before = stamp(&document);
    let registry_before = exact_results.contents_stamp();
    let outputs_before = derived_output_stamp(&source, &exact_results);
    let mut evaluations = 0;

    let failed = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> {
            evaluations += 1;
            Err("worker failed".to_owned())
        },
    );
    assert_eq!(
        failed,
        Err(SharedChangePropagationError::Evaluation(
            "worker failed".to_owned()
        ))
    );
    assert_eq!(evaluations, 1);
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), registry_before);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        outputs_before
    );

    let stale_package = Arc::new(ExactBodyPackage::from(exact_package(&source, "stale")));
    let failed = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&stale_package)) },
    );
    assert!(matches!(
        failed,
        Err(SharedChangePropagationError::ExactPublication(_))
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), registry_before);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        outputs_before
    );
}

#[test]
fn over_constrained_dependency_refuses_the_whole_shared_change() {
    let mut document = seed(false);
    let evidence = exact_package(&document.current(), "conflicting-mates");
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(41),
                planar_endpoint(&evidence, FIRST, ExactFaceRole::Top),
                planar_endpoint(&evidence, SECOND, ExactFaceRole::Top),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(42),
                planar_endpoint(&evidence, FIRST, ExactFaceRole::Top),
                planar_endpoint(&evidence, SECOND, ExactFaceRole::Top),
                AssemblyMateKind::Distance { distance_mm: 10.0 },
            )),
        ]))
        .unwrap();
    let source = document.current();
    let mut exact_results = registry(&source);
    let impact = project_shared_change_impact(
        &document,
        &exact_results,
        edit_request(&source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let before = stamp(&document);
    let registry_before = exact_results.contents_stamp();
    let outputs_before = derived_output_stamp(&source, &exact_results);
    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &candidate,
        "dependency-failure",
    )));

    let failed = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    );

    assert!(
        matches!(failed, Err(SharedChangePropagationError::Dependency(_))),
        "{failed:?}"
    );
    assert_eq!(stamp(&document), before);
    assert_eq!(exact_results.contents_stamp(), registry_before);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        outputs_before
    );
}

#[test]
fn propagation_verifier_covers_manual_ai_redo_save_open_and_shared_exact_outputs() {
    let mut document = seed(false);
    let source = document.current();
    let source_stamp = stamp(&document);
    let source_results = registry(&source);
    let source_outputs = derived_output_stamp(&source, &source_results);
    let manual = project_shared_change_impact(
        &document,
        &source_results,
        edit_request(&source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let assistant = project_shared_change_impact(
        &document,
        &source_results,
        edit_request(&source),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();

    assert_eq!(manual.candidate_digest, assistant.candidate_digest);
    assert_eq!(manual.definition_id, assistant.definition_id);
    assert_eq!(manual.affected_body_ids, assistant.affected_body_ids);
    assert_eq!(manual.affected_feature_ids, assistant.affected_feature_ids);
    assert_eq!(manual.occurrences, assistant.occurrences);
    assert_eq!(manual.exact_jobs, assistant.exact_jobs);
    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(
        manual.proposal.command_digest(),
        assistant.proposal.command_digest()
    );
    assert_eq!(
        manual.proposal.intended_result_digest(),
        assistant.proposal.intended_result_digest()
    );
    assert_ne!(manual.proposal.principal(), assistant.proposal.principal());
    assert_eq!(stamp(&document), source_stamp);

    let candidate = document.preview_batch(manual.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &candidate,
        "verified-shared-change",
    )));
    let mut exact_results = source_results.clone();
    let mut evaluations = 0;
    let receipt = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &manual,
        |request| -> Result<Arc<ExactBodyPackage>, String> {
            evaluations += 1;
            assert_eq!(request.definition_id, DEFINITION);
            assert_eq!(request.producer_feature_id(), EXTRUSION);
            Ok(Arc::clone(&evaluated))
        },
    )
    .unwrap();

    assert_eq!(evaluations, 1);
    assert_eq!(manual.exact_jobs.len(), 1);
    assert_eq!(document.visible_undo_steps(), source_stamp.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    assert_eq!(receipt.occurrences.len(), 2);
    assert_eq!(
        receipt
            .occurrences
            .iter()
            .map(|occurrence| occurrence.instance_path.clone())
            .collect::<Vec<_>>(),
        vec![
            ketchup_core::document::InstancePath::root(FIRST),
            ketchup_core::document::InstancePath::root(SECOND),
        ]
    );
    assert!(receipt.occurrences.windows(2).all(|occurrences| {
        occurrences[0].result_fingerprint == occurrences[1].result_fingerprint
            && occurrences[0].subshape_lineage_digests == occurrences[1].subshape_lineage_digests
    }));
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let committed_outputs = derived_output_stamp(&committed, &exact_results);
    assert_eq!(
        committed_outputs.package.bounds_mm(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 15.0]]
    );
    assert!(!committed_outputs.package.vertices().is_empty());
    assert!(!committed_outputs.package.triangles().is_empty());
    assert!(!committed_outputs.package.references().is_empty());

    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().canonical_digest(), committed_digest);
    let reopened_results = ExactResultRegistry::accept(
        &reopened.current(),
        [Arc::new(committed_outputs.package.clone())],
    )
    .unwrap();
    assert_eq!(
        derived_output_stamp(&reopened.current(), &reopened_results),
        committed_outputs
    );

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), source_stamp.digest);
    assert_eq!(document.visible_undo_steps(), source_stamp.undo);
    assert_eq!(document.visible_redo_steps(), 1);
    assert_eq!(
        derived_output_stamp(&document.current(), &source_results),
        source_outputs
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert_eq!(document.visible_undo_steps(), source_stamp.undo + 1);
    assert_eq!(document.visible_redo_steps(), 0);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        committed_outputs
    );
}

#[test]
fn duplicate_invalid_and_stale_propagation_requests_preserve_history_and_outputs() {
    let mut document = seed(false);
    let source = document.current();
    let mut exact_results = registry(&source);
    let before = stamp(&document);
    let outputs_before = derived_output_stamp(&source, &exact_results);
    let duplicate = ExactParameterEdit {
        target: ExactParameterEditTarget::FeatureDimension(EXTRUSION),
        dimension: Dimension::from_decimal("15").unwrap(),
    };
    let duplicate_request = SharedDefinitionChangeRequest::exact_parameter_edit(
        &source,
        BodyParameterEditRequest {
            definition_id: DEFINITION,
            body_id: BodyId(1),
            edits: vec![duplicate.clone(), duplicate],
        },
    );

    assert!(matches!(
        project_shared_change_impact(
            &document,
            &exact_results,
            duplicate_request,
            ProposalPrincipal::LocalAssistant,
        ),
        Err(SharedChangeImpactError::Unsupported(reason)) if reason.contains("duplicate")
    ));
    assert_eq!(stamp(&document), before);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        outputs_before
    );

    let impact = project_shared_change_impact(
        &document,
        &exact_results,
        edit_request(&source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let mut invalid = impact.clone();
    invalid.affected_body_ids.push(BodyId(1));
    let mut evaluations = 0;
    assert!(matches!(
        commit_shared_definition_change(
            &mut document,
            &mut exact_results,
            &invalid,
            |_| -> Result<Arc<ExactBodyPackage>, String> {
                evaluations += 1;
                panic!("invalid impact reached the evaluator")
            },
        ),
        Err(SharedChangePropagationError::InvalidImpact(_))
    ));
    assert_eq!(evaluations, 0);
    assert_eq!(stamp(&document), before);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        outputs_before
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OTHER,
                visible: false,
            },
        ]))
        .unwrap();
    exact_results = ExactResultRegistry::carried_forward(&document.current(), &exact_results);
    let changed = stamp(&document);
    let changed_outputs = derived_output_stamp(&document.current(), &exact_results);
    assert_eq!(
        commit_shared_definition_change(
            &mut document,
            &mut exact_results,
            &impact,
            |_| -> Result<Arc<ExactBodyPackage>, String> {
                panic!("stale impact reached the evaluator")
            },
        ),
        Err(SharedChangePropagationError::Stale)
    );
    assert_eq!(stamp(&document), changed);
    assert_eq!(
        derived_output_stamp(&document.current(), &exact_results),
        changed_outputs
    );
}

#[test]
fn dependent_rebind_verifier_covers_planar_mates_drawing_exports_undo_and_save_open() {
    let mut document = seed_rigid_dependencies();
    let source = document.current();
    let source_stamp = stamp(&document);
    let source_results = registry(&source);
    let source_outputs = derived_output_stamp(&source, &source_results);
    let source_transforms = [FIRST, SECOND].map(|id| source.occurrence(id).unwrap().transform());
    let impact = project_shared_change_impact(
        &document,
        &source_results,
        edit_request(&source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    assert_eq!(
        impact
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
    assert_eq!(impact.mate_references.len(), 4);
    assert_eq!(
        impact
            .exports
            .iter()
            .map(|export| (export.format, export.occurrence_paths.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                SharedChangeExportFormat::Step,
                vec![
                    ketchup_core::document::InstancePath::root(FIRST),
                    ketchup_core::document::InstancePath::root(SECOND),
                ],
            ),
            (
                SharedChangeExportFormat::Stl,
                vec![
                    ketchup_core::document::InstancePath::root(FIRST),
                    ketchup_core::document::InstancePath::root(SECOND),
                ],
            ),
        ]
    );

    let candidate = document.preview_batch(impact.proposal.batch()).unwrap();
    let evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &candidate,
        "dependent-rebind-verifier",
    )));
    let mut exact_results = source_results.clone();
    let receipt = commit_shared_definition_change(
        &mut document,
        &mut exact_results,
        &impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&evaluated)) },
    )
    .unwrap();

    assert_eq!(receipt.rebound_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(
        receipt
            .drawings
            .iter()
            .flat_map(|drawing| drawing.views.iter().map(|view| view.kind))
            .collect::<Vec<_>>(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ]
    );
    assert!(
        receipt
            .drawings
            .iter()
            .all(|drawing| drawing.is_current(&document.current()))
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
    let committed = document.current();
    assert_eq!(
        [FIRST, SECOND].map(|id| committed.occurrence(id).unwrap().transform()),
        source_transforms
    );
    for mate_id in [MATE, AXIAL_MATE] {
        let mate = committed.assembly_mate(mate_id).unwrap();
        assert!(
            [mate.endpoint_a(), mate.endpoint_b()]
                .into_iter()
                .all(|endpoint| endpoint.reference().result_fingerprint
                    == evaluated.result_key().result_fingerprint)
        );
    }
    let committed_package = exact_results
        .get_body(&committed, DEFINITION, BodyId(1))
        .unwrap()
        .unwrap();
    let committed_stl = exact_model_stl_export(
        &committed,
        &receipt
            .occurrences
            .iter()
            .map(|occurrence| (committed_package.as_ref(), occurrence.transform))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let committed_drawing = receipt.drawings[0].clone();
    let committed_digest = committed.canonical_digest();

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), source_stamp.digest);
    assert_eq!(
        derived_output_stamp(&document.current(), &source_results),
        source_outputs
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &exact_results,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        committed_drawing
    );

    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_results =
        ExactResultRegistry::accept(&reopened.current(), [Arc::clone(committed_package)]).unwrap();
    assert_eq!(reopened.current().canonical_digest(), committed_digest);
    assert_eq!(
        project_orthographic_drawing(
            &reopened.current(),
            &reopened_results,
            reopened.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        committed_drawing
    );
    assert_eq!(
        exact_model_stl_export(
            &reopened.current(),
            &receipt
                .occurrences
                .iter()
                .map(|occurrence| (committed_package.as_ref(), occurrence.transform))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
        committed_stl
    );
}

#[test]
fn dependent_rebind_failures_preserve_last_valid_transforms_views_and_exports() {
    let under_constrained = seed_rigid_dependencies();
    let under_source = under_constrained.current();
    let under_results = registry(&under_source);
    let under_before = stamp(&under_constrained);
    let under_outputs = derived_output_stamp(&under_source, &under_results);
    let under_transforms =
        [FIRST, SECOND].map(|id| under_source.occurrence(id).unwrap().transform());
    let under_impact = project_shared_change_impact(
        &under_constrained,
        &under_results,
        edit_request(&under_source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let mut under_commands = under_impact.proposal.batch().commands().to_vec();
    under_commands.push(CanonicalCommand::SetOccurrenceGrounded {
        id: SECOND,
        grounded: false,
    });
    let under_failed = under_constrained.prepare_proposal(CommandBatch::new(under_commands));
    assert!(under_failed.is_err(), "{under_failed:?}");
    assert_eq!(stamp(&under_constrained), under_before);
    assert_eq!(
        [FIRST, SECOND].map(|id| under_constrained
            .current()
            .occurrence(id)
            .unwrap()
            .transform()),
        under_transforms
    );
    assert_eq!(
        derived_output_stamp(&under_constrained.current(), &under_results),
        under_outputs
    );

    let mut hidden = seed(false);
    let hidden_source = hidden.current();
    let mut hidden_results = registry(&hidden_source);
    let hidden_before = stamp(&hidden);
    let hidden_registry_before = hidden_results.contents_stamp();
    let hidden_outputs = derived_output_stamp(&hidden_source, &hidden_results);
    let hidden_transforms =
        [FIRST, SECOND].map(|id| hidden_source.occurrence(id).unwrap().transform());
    let mut hidden_impact = project_shared_change_impact(
        &hidden,
        &hidden_results,
        edit_request(&hidden_source),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    for export in &mut hidden_impact.exports {
        export
            .occurrence_paths
            .push(ketchup_core::document::InstancePath::root(SECOND));
    }
    let hidden_candidate = hidden
        .preview_batch(hidden_impact.proposal.batch())
        .unwrap();
    let hidden_evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &hidden_candidate,
        "hidden-export-path",
    )));
    let hidden_failed = commit_shared_definition_change(
        &mut hidden,
        &mut hidden_results,
        &hidden_impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&hidden_evaluated)) },
    );
    assert!(
        matches!(
            hidden_failed,
            Err(SharedChangePropagationError::Dependency(ref reason))
                if reason.contains("not visible and current")
        ),
        "{hidden_failed:?}"
    );
    assert_eq!(stamp(&hidden), hidden_before);
    assert_eq!(hidden_results.contents_stamp(), hidden_registry_before);
    assert_eq!(
        [FIRST, SECOND].map(|id| hidden.current().occurrence(id).unwrap().transform()),
        hidden_transforms
    );
    assert_eq!(
        derived_output_stamp(&hidden.current(), &hidden_results),
        hidden_outputs
    );

    let mut unsupported = seed_rigid_dependencies();
    let unsupported_source = unsupported.current();
    let mut unsupported_results = registry(&unsupported_source);
    let unsupported_before = stamp(&unsupported);
    let unsupported_registry_before = unsupported_results.contents_stamp();
    let unsupported_outputs = derived_output_stamp(&unsupported_source, &unsupported_results);
    let unsupported_transforms =
        [FIRST, SECOND].map(|id| unsupported_source.occurrence(id).unwrap().transform());
    let mut unsupported_impact = project_shared_change_impact(
        &unsupported,
        &unsupported_results,
        edit_request(&unsupported_source),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    for export in &mut unsupported_impact.exports {
        export.occurrence_paths = vec![ketchup_core::document::InstancePath::root(OTHER)];
    }
    let unsupported_candidate = unsupported
        .preview_batch(unsupported_impact.proposal.batch())
        .unwrap();
    let unsupported_evaluated = Arc::new(ExactBodyPackage::from(history_package(
        &unsupported_candidate,
        "unsupported-export-path",
    )));
    let unsupported_failed = commit_shared_definition_change(
        &mut unsupported,
        &mut unsupported_results,
        &unsupported_impact,
        |_| -> Result<Arc<ExactBodyPackage>, String> { Ok(Arc::clone(&unsupported_evaluated)) },
    );
    assert!(
        matches!(
            unsupported_failed,
            Err(SharedChangePropagationError::Dependency(ref reason))
                if reason.contains("has no current exact body")
        ),
        "{unsupported_failed:?}"
    );
    assert_eq!(stamp(&unsupported), unsupported_before);
    assert_eq!(
        unsupported_results.contents_stamp(),
        unsupported_registry_before
    );
    assert_eq!(
        [FIRST, SECOND].map(|id| unsupported.current().occurrence(id).unwrap().transform()),
        unsupported_transforms
    );
    assert_eq!(
        derived_output_stamp(&unsupported.current(), &unsupported_results),
        unsupported_outputs
    );
}
