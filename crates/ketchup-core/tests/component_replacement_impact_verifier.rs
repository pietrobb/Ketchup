use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, AssemblySolveStatus,
    AssemblySolverPolicy, solve_rigid_assembly,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, OccurrenceId, ProposalPrincipal, StableFaceRole, Transform,
};
use ketchup_core::drawing::{
    DrawingSheet, DrawingSheetId, DrawingSource, OrthographicViewKind, project_orthographic_drawing,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactRenderPackage,
    ExactResultRegistry, build_box_render_package, canonical_reference_lineage_digest,
    exact_model_stl_export,
};
use ketchup_core::persistence;
use ketchup_core::shared_change::{
    ComponentReplacementCommitError, ComponentReplacementImpactError,
    ComponentReplacementImpactRequest, SharedChangeExportEligibility, SharedChangeExportFormat,
    commit_component_replacement, project_component_replacement_impact,
    project_component_replacement_impact_for_principal,
};
use std::sync::Arc;

const SOURCE: DefinitionId = DefinitionId(41);
const TARGET: DefinitionId = DefinitionId(42);
const SOURCE_PROFILE: FeatureId = FeatureId(410);
const SOURCE_EXTRUSION: FeatureId = FeatureId(411);
const TARGET_PROFILE: FeatureId = FeatureId(420);
const TARGET_EXTRUSION: FeatureId = FeatureId(421);
const SELECTED: OccurrenceId = OccurrenceId(4100);
const SIBLING: OccurrenceId = OccurrenceId(4101);
const TARGET_OCCURRENCE: OccurrenceId = OccurrenceId(4200);
const MATE: AssemblyMateId = AssemblyMateId(4300);
const AXIAL_MATE: AssemblyMateId = AssemblyMateId(4301);
const SHEET: DrawingSheetId = DrawingSheetId(4400);
const ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
];

#[derive(Debug, Eq, PartialEq)]
struct StoreStamp {
    revision: u64,
    digest: String,
    revisions: usize,
    undo: usize,
    redo: usize,
    bytes: Vec<u8>,
}

fn store_stamp(document: &DocumentStore) -> StoreStamp {
    StoreStamp {
        revision: document.current().revision_id(),
        digest: document.current().canonical_digest(),
        revisions: document.revision_count(),
        undo: document.visible_undo_steps(),
        redo: document.visible_redo_steps(),
        bytes: persistence::save(&document.current()),
    }
}

fn package<const N: usize>(
    snapshot: &ketchup_core::document::Snapshot,
    definition_id: DefinitionId,
    fingerprint: &str,
    roles: [ExactFaceRole; N],
) -> ExactRenderPackage {
    let request =
        ExactFeatureChainRequest::from_snapshot_for_body(snapshot, definition_id, BodyId(1))
            .unwrap();
    let evidence = roles.map(|role| {
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
    });
    build_box_render_package(
        &request,
        format!("exact-input:{definition_id:?}:{fingerprint}"),
        fingerprint.to_owned(),
        "occt".into(),
        "r0".into(),
        request.expected_bounds_mm(),
        evidence,
    )
    .unwrap()
}

fn registry(snapshot: &ketchup_core::document::Snapshot, reverse: bool) -> ExactResultRegistry {
    let source = Arc::new(ExactBodyPackage::from(package(
        snapshot,
        SOURCE,
        "source-current",
        ROLES,
    )));
    let target = Arc::new(ExactBodyPackage::from(package(
        snapshot,
        TARGET,
        "target-current",
        ROLES,
    )));
    if reverse {
        ExactResultRegistry::accept(snapshot, [target, source]).unwrap()
    } else {
        ExactResultRegistry::accept(snapshot, [source, target]).unwrap()
    }
}

fn seed() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: SOURCE,
                name: "Verifier source".into(),
            },
            CanonicalCommand::CreateDefinition {
                id: TARGET,
                name: "Verifier target".into(),
            },
            CanonicalCommand::CreateFeature {
                id: SOURCE_PROFILE,
                definition_id: SOURCE,
                name: "Source profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 6.0], [0.0, 6.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: SOURCE_EXTRUSION,
                definition_id: SOURCE,
                name: "Source extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: SOURCE_PROFILE,
                    height: Dimension::from_decimal("7").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: TARGET_PROFILE,
                definition_id: TARGET,
                name: "Target profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 9.0], [0.0, 9.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: TARGET_EXTRUSION,
                definition_id: TARGET,
                name: "Target extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: TARGET_PROFILE,
                    height: Dimension::from_decimal("11").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: TARGET_OCCURRENCE,
                definition_id: TARGET,
                name: "Existing target".into(),
                transform: Transform::from_translation(30.0, 2.0, 3.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SIBLING,
                definition_id: SOURCE,
                name: "Unchanged sibling".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SELECTED,
                definition_id: SOURCE,
                name: "Selected source".into(),
                transform: Transform::from_translation(1.0, 2.0, 3.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let source_package = package(&snapshot, SOURCE, "mate-source", ROLES);
    let target_package = package(&snapshot, TARGET, "mate-target", ROLES);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::resolved(
                    SELECTED,
                    source_package
                        .reference(ExactFaceRole::Top)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    TARGET_OCCURRENCE,
                    target_package
                        .reference(ExactFaceRole::Bottom)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved(
                    SELECTED,
                    source_package
                        .reference(ExactFaceRole::East)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    TARGET_OCCURRENCE,
                    target_package
                        .reference(ExactFaceRole::East)
                        .unwrap()
                        .clone(),
                ),
                AssemblyMateKind::ConcentricAxial { reversed: false },
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
                    "Verifier replacement",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![SELECTED, SIBLING, TARGET_OCCURRENCE],
                    },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    document
}

#[test]
fn save_open_replay_and_registry_permutations_are_identical_and_read_only() {
    let document = seed();
    let direct_results = registry(&document.current(), false);
    let reversed_results = registry(&document.current(), true);
    let before = store_stamp(&document);
    let direct_stamp = direct_results.contents_stamp();
    let reversed_stamp = reversed_results.contents_stamp();
    let request = ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET);

    let inspected =
        project_component_replacement_impact(&document, &direct_results, request.clone()).unwrap();
    let previewed =
        project_component_replacement_impact(&document, &reversed_results, request).unwrap();
    assert_eq!(inspected, previewed);
    assert_eq!(inspected.selected_occurrence_id, SELECTED);
    assert_eq!(
        inspected.selected_transform,
        Transform::from_translation(1.0, 2.0, 3.0).unwrap()
    );
    assert_eq!(inspected.mate_references.len(), 2);
    assert_eq!(inspected.drawing_views.len(), 3);
    assert_eq!(inspected.exports.len(), 2);

    drop(previewed);
    assert_eq!(store_stamp(&document), before);
    assert_eq!(direct_results.contents_stamp(), direct_stamp);
    assert_eq!(reversed_results.contents_stamp(), reversed_stamp);

    let reopened = persistence::load(&before.bytes)
        .unwrap()
        .into_editable()
        .unwrap_or_else(|_| panic!("current replacement fixture must reopen as editable"));
    let reopened_results = registry(&reopened.current(), true);
    let reopened_before = store_stamp(&reopened);
    let replayed = project_component_replacement_impact(
        &reopened,
        &reopened_results,
        ComponentReplacementImpactRequest::new(&reopened.current(), SELECTED, TARGET),
    )
    .unwrap();
    assert_eq!(replayed, inspected);
    assert_eq!(store_stamp(&reopened), reopened_before);
}

#[test]
fn ambiguity_topology_identity_and_unsupported_inputs_have_explicit_diagnostics() {
    let document = seed();
    let snapshot = document.current();
    let before = store_stamp(&document);

    let source = Arc::new(ExactBodyPackage::from(package(
        &snapshot,
        SOURCE,
        "source-current",
        ROLES,
    )));
    let target = package(&snapshot, TARGET, "target-current", ROLES);
    let mut alternate = target.clone();
    alternate.identity.backend.push_str("-alternate");
    for reference in &mut alternate.references {
        reference.backend = alternate.identity.backend.clone();
    }
    let ambiguous = ExactResultRegistry::accept(
        &snapshot,
        [
            Arc::clone(&source),
            Arc::new(ExactBodyPackage::from(target)),
            Arc::new(ExactBodyPackage::from(alternate)),
        ],
    )
    .unwrap();
    let error = project_component_replacement_impact(
        &document,
        &ambiguous,
        ComponentReplacementImpactRequest::new(&snapshot, SELECTED, TARGET),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ComponentReplacementImpactError::Ambiguous(TARGET, BodyId(1))
    );
    assert_eq!(
        error.to_string(),
        "definition 42 body 1 has ambiguous exact results"
    );

    let current = registry(&snapshot, false);
    let missing_occurrence = project_component_replacement_impact(
        &document,
        &current,
        ComponentReplacementImpactRequest::new(&snapshot, OccurrenceId(999_001), TARGET),
    )
    .unwrap_err();
    assert_eq!(
        missing_occurrence,
        ComponentReplacementImpactError::OccurrenceNotFound(OccurrenceId(999_001))
    );
    let missing_target = project_component_replacement_impact(
        &document,
        &current,
        ComponentReplacementImpactRequest::new(&snapshot, SELECTED, DefinitionId(999_002)),
    )
    .unwrap_err();
    assert_eq!(
        missing_target,
        ComponentReplacementImpactError::TargetDefinitionNotFound(DefinitionId(999_002))
    );
    let mut no_target = ComponentReplacementImpactRequest::new(&snapshot, SELECTED, TARGET);
    no_target.target_definition_ids.clear();
    assert_eq!(
        project_component_replacement_impact(&document, &current, no_target),
        Err(ComponentReplacementImpactError::DuplicateTarget)
    );
    assert_eq!(store_stamp(&document), before);

    let mut topology_mismatch = seed();
    topology_mismatch
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(422),
            definition_id: TARGET,
            name: "Unmatched target feature".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
            },
        }]))
        .unwrap();
    let topology_results = registry(&topology_mismatch.current(), false);
    let topology_before = store_stamp(&topology_mismatch);
    let error = project_component_replacement_impact(
        &topology_mismatch,
        &topology_results,
        ComponentReplacementImpactRequest::new(&topology_mismatch.current(), SELECTED, TARGET),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ComponentReplacementImpactError::Incompatible(_)
    ));
    assert_eq!(
        error.to_string(),
        "source and target definitions have different feature counts"
    );
    assert_eq!(store_stamp(&topology_mismatch), topology_before);
}

#[test]
fn unsupported_mate_fails_closed_without_canonical_or_registry_mutation() {
    let mut document = seed();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyMateKind {
                id: MATE,
                kind: AssemblyMateKind::Distance { distance_mm: 4.0 },
            },
        ]))
        .unwrap();
    let results = registry(&document.current(), false);
    let before = store_stamp(&document);
    let results_before = results.contents_stamp();

    let error = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&document.current(), SELECTED, TARGET),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ComponentReplacementImpactError::Unsupported(_)
    ));
    assert_eq!(
        error.to_string(),
        "assembly mate 4300 is not planar or axial"
    );
    assert_eq!(store_stamp(&document), before);
    assert_eq!(results.contents_stamp(), results_before);
}

fn dependency_free_seed() -> DocumentStore {
    let mut document = seed();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteAssemblyMate { id: MATE },
            CanonicalCommand::DeleteAssemblyMate { id: AXIAL_MATE },
        ]))
        .unwrap();
    document
}

fn result_fingerprints(
    snapshot: &ketchup_core::document::Snapshot,
    results: &ExactResultRegistry,
) -> Vec<(DefinitionId, BodyId, String)> {
    results
        .body_values(snapshot)
        .unwrap()
        .into_iter()
        .map(|(key, package)| {
            (
                key.definition_id,
                key.body_id,
                package.result_key().result_fingerprint.clone(),
            )
        })
        .collect()
}

#[test]
fn confirm_cancel_undo_redo_save_open_and_manual_ai_paths_share_one_atomic_contract() {
    let mut manual_document = dependency_free_seed();
    let manual_source = manual_document.current();
    let mut assistant_document = persistence::load(&persistence::save(&manual_source))
        .unwrap()
        .into_editable()
        .unwrap_or_else(|_| panic!("replacement parity fixture must reopen as editable"));
    let assistant_source = assistant_document.current();
    assert_eq!(manual_source.document_id(), assistant_source.document_id());
    assert_eq!(manual_source.revision_id(), assistant_source.revision_id());
    assert_eq!(
        manual_source.canonical_digest(),
        assistant_source.canonical_digest()
    );

    let source_definition = manual_source.definition(SOURCE).unwrap().clone();
    let target_definition = manual_source.definition(TARGET).unwrap().clone();
    let selected_before = manual_source.occurrence(SELECTED).unwrap().clone();
    let sibling_before = manual_source.occurrence(SIBLING).unwrap().clone();
    let target_occurrence_before = manual_source.occurrence(TARGET_OCCURRENCE).unwrap().clone();
    let manual_before = store_stamp(&manual_document);
    let assistant_before = store_stamp(&assistant_document);
    let mut manual_results = registry(&manual_source, false);
    let mut assistant_results = registry(&assistant_source, true);
    let source_results = result_fingerprints(&manual_source, &manual_results);

    let manual_impact = project_component_replacement_impact(
        &manual_document,
        &manual_results,
        ComponentReplacementImpactRequest::new(&manual_source, SELECTED, TARGET),
    )
    .unwrap();
    let assistant_impact = project_component_replacement_impact_for_principal(
        &assistant_document,
        &assistant_results,
        ComponentReplacementImpactRequest::new(&assistant_source, SELECTED, TARGET),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let manual_proposal = manual_impact.proposal.as_ref().unwrap();
    let assistant_proposal = assistant_impact.proposal.as_ref().unwrap();
    assert_eq!(manual_proposal.principal(), ProposalPrincipal::ManualClient);
    assert_eq!(
        assistant_proposal.principal(),
        ProposalPrincipal::LocalAssistant
    );
    assert_eq!(manual_proposal.batch(), assistant_proposal.batch());
    assert_eq!(
        manual_proposal.command_digest(),
        assistant_proposal.command_digest()
    );
    assert_eq!(
        manual_proposal.intended_result_digest(),
        assistant_proposal.intended_result_digest()
    );
    let mut manual_contract = manual_impact.clone();
    let mut assistant_contract = assistant_impact.clone();
    manual_contract.proposal = None;
    assistant_contract.proposal = None;
    assert_eq!(manual_contract, assistant_contract);

    let cancelled = project_component_replacement_impact(
        &manual_document,
        &manual_results,
        ComponentReplacementImpactRequest::new(&manual_source, SELECTED, TARGET),
    )
    .unwrap();
    let cancelled_preview = manual_document
        .preview_batch(cancelled.proposal.as_ref().unwrap().batch())
        .unwrap();
    assert_eq!(
        cancelled_preview.canonical_digest(),
        cancelled.candidate_digest.as_deref().unwrap()
    );
    drop(cancelled);
    assert_eq!(store_stamp(&manual_document), manual_before);
    assert_eq!(store_stamp(&assistant_document), assistant_before);
    assert_eq!(
        result_fingerprints(&manual_document.current(), &manual_results),
        source_results
    );

    let manual_receipt =
        commit_component_replacement(&mut manual_document, &mut manual_results, &manual_impact)
            .unwrap();
    let assistant_receipt = commit_component_replacement(
        &mut assistant_document,
        &mut assistant_results,
        &assistant_impact,
    )
    .unwrap();
    assert_eq!(manual_receipt, assistant_receipt);

    let committed = manual_document.current();
    assert_eq!(manual_receipt.revision_id, manual_before.revision + 1);
    assert_eq!(
        manual_document.revision_count(),
        manual_before.revisions + 1
    );
    assert_eq!(manual_document.visible_undo_steps(), manual_before.undo + 1);
    assert_eq!(manual_document.visible_redo_steps(), 0);
    assert_eq!(committed.definition(SOURCE), Some(&source_definition));
    assert_eq!(committed.definition(TARGET), Some(&target_definition));
    assert_eq!(committed.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        committed.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    let selected_after = committed.occurrence(SELECTED).unwrap();
    assert_eq!(selected_after.id(), selected_before.id());
    assert_eq!(selected_after.definition_id(), TARGET);
    assert_eq!(selected_after.name(), selected_before.name());
    assert_eq!(selected_after.transform(), selected_before.transform());
    assert_eq!(selected_after.parent(), selected_before.parent());
    assert_eq!(selected_after.tag(), selected_before.tag());
    assert_eq!(selected_after.visible(), selected_before.visible());

    let scene = committed.scene_query();
    let selected_scene = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_id == SELECTED)
        .unwrap();
    assert_eq!(selected_scene.definition_id, TARGET);
    assert_eq!(selected_scene.transform, selected_before.transform());
    assert_eq!(
        scene
            .iter()
            .find(|occurrence| occurrence.occurrence_id == SIBLING)
            .unwrap()
            .definition_id,
        SOURCE
    );
    assert_eq!(
        scene
            .iter()
            .find(|occurrence| occurrence.occurrence_id == TARGET_OCCURRENCE)
            .unwrap()
            .definition_id,
        TARGET
    );
    assert!(manual_results.is_bound_to(&committed));
    assert_eq!(
        result_fingerprints(&committed, &manual_results),
        source_results
    );
    assert_eq!(
        manual_results
            .get_render(&committed, SOURCE)
            .unwrap()
            .result_key()
            .result_fingerprint,
        "source-current"
    );
    assert_eq!(
        manual_results
            .get_render(&committed, TARGET)
            .unwrap()
            .result_key()
            .result_fingerprint,
        "target-current"
    );

    let committed_digest = committed.canonical_digest();
    let bytes = persistence::save(&committed);
    let reopened = persistence::load(&bytes).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(reopened_snapshot.canonical_digest(), committed_digest);
    assert_eq!(
        reopened_snapshot
            .occurrence(SELECTED)
            .unwrap()
            .definition_id(),
        TARGET
    );
    assert_eq!(reopened_snapshot.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        reopened_snapshot.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    let reopened_results =
        ExactResultRegistry::carried_forward(&reopened_snapshot, &manual_results);
    assert!(reopened_results.is_bound_to(&reopened_snapshot));
    assert_eq!(
        result_fingerprints(&reopened_snapshot, &reopened_results),
        source_results
    );

    let undone = manual_document.undo().unwrap();
    assert_eq!(undone.canonical_digest(), manual_before.digest);
    assert_eq!(undone.occurrence(SELECTED), Some(&selected_before));
    let undo_results = ExactResultRegistry::carried_forward(&undone, &manual_results);
    assert!(undo_results.is_bound_to(&undone));
    assert_eq!(result_fingerprints(&undone, &undo_results), source_results);
    assert_eq!(manual_document.visible_redo_steps(), 1);

    let redone = manual_document.redo().unwrap();
    assert_eq!(redone.canonical_digest(), committed_digest);
    assert_eq!(redone.occurrence(SELECTED).unwrap().definition_id(), TARGET);
    let redo_results = ExactResultRegistry::carried_forward(&redone, &undo_results);
    assert!(redo_results.is_bound_to(&redone));
    assert_eq!(result_fingerprints(&redone, &redo_results), source_results);
}

#[test]
fn planar_axial_rebind_replays_drawing_export_undo_redo_and_save_open_atomically() {
    let mut document = seed();
    let source = document.current();
    let source_definition = source.definition(SOURCE).unwrap().clone();
    let target_definition = source.definition(TARGET).unwrap().clone();
    let selected_before = source.occurrence(SELECTED).unwrap().clone();
    let sibling_before = source.occurrence(SIBLING).unwrap().clone();
    let target_occurrence_before = source.occurrence(TARGET_OCCURRENCE).unwrap().clone();
    let source_mates =
        [MATE, AXIAL_MATE].map(|mate_id| source.assembly_mate(mate_id).unwrap().clone());
    let mut results = registry(&source, false);
    let impact = project_component_replacement_impact(
        &document,
        &results,
        ComponentReplacementImpactRequest::new(&source, SELECTED, TARGET),
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
    assert!(matches!(
        impact.proposal.as_ref().unwrap().batch().commands(),
        [
            CanonicalCommand::RepointOccurrence { .. },
            CanonicalCommand::RebindAssemblyMate(_),
            CanonicalCommand::RebindAssemblyMate(_)
        ]
    ));
    let before = store_stamp(&document);

    let receipt = commit_component_replacement(&mut document, &mut results, &impact).unwrap();
    let committed = document.current();
    assert_eq!(receipt.rebound_mate_ids, vec![MATE, AXIAL_MATE]);
    assert_eq!(receipt.drawings.len(), 1);
    assert!(receipt.drawings[0].is_current(&committed));
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
    assert_eq!(committed.definition(SOURCE), Some(&source_definition));
    assert_eq!(committed.definition(TARGET), Some(&target_definition));
    assert_eq!(committed.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        committed.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    let selected_after = committed.occurrence(SELECTED).unwrap();
    assert_eq!(selected_after.id(), selected_before.id());
    assert_eq!(selected_after.definition_id(), TARGET);
    assert_eq!(selected_after.transform(), selected_before.transform());
    for (mate_id, source_mate) in [MATE, AXIAL_MATE].into_iter().zip(&source_mates) {
        let rebound = committed.assembly_mate(mate_id).unwrap();
        assert_eq!(rebound.endpoint_b(), source_mate.endpoint_b());
        assert_eq!(rebound.endpoint_a().occurrence_id(), SELECTED);
        assert_eq!(rebound.endpoint_a().reference().definition_id, TARGET);
        assert!(impact.mate_references.iter().any(|reference| {
            reference.mate_id == mate_id
                && reference.target_lineage_digest
                    == rebound.endpoint_a().reference().lineage_digest
        }));
    }
    let solved = solve_rigid_assembly(&committed, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(solved.status(), AssemblySolveStatus::FullyConstrained);
    assert!(solved.conflicting_mate_ids().is_empty());
    assert!(solved.maximum_residual().is_finite());

    let committed_drawing = project_orthographic_drawing(
        &committed,
        &results,
        committed.drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(committed_drawing, receipt.drawings[0]);
    let target_package = results
        .get_body(&committed, TARGET, BodyId(1))
        .unwrap()
        .unwrap();
    let committed_stl = exact_model_stl_export(
        &committed,
        &[(target_package.as_ref(), selected_after.transform())],
    )
    .unwrap();

    let committed_digest = committed.canonical_digest();
    let committed_mates =
        [MATE, AXIAL_MATE].map(|mate_id| committed.assembly_mate(mate_id).unwrap().clone());
    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_results = ExactResultRegistry::carried_forward(&reopened_snapshot, &results);
    assert_eq!(reopened_snapshot.canonical_digest(), committed_digest);
    assert_eq!(reopened_snapshot.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        reopened_snapshot.occurrence(TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
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
                    .get_body(&reopened_snapshot, TARGET, BodyId(1))
                    .unwrap()
                    .unwrap()
                    .as_ref(),
                reopened_snapshot.occurrence(SELECTED).unwrap().transform(),
            )],
        )
        .unwrap(),
        committed_stl
    );

    let undone = document.undo().unwrap();
    assert_eq!(undone.canonical_digest(), before.digest);
    assert_eq!(undone.occurrence(SELECTED), Some(&selected_before));
    assert_eq!(
        [MATE, AXIAL_MATE].map(|mate_id| undone.assembly_mate(mate_id).unwrap().clone()),
        source_mates
    );
    let redone = document.redo().unwrap();
    assert_eq!(redone.canonical_digest(), committed_digest);
    assert_eq!(redone.occurrence(SIBLING), Some(&sibling_before));
    assert_eq!(
        [MATE, AXIAL_MATE].map(|mate_id| redone.assembly_mate(mate_id).unwrap().clone()),
        committed_mates
    );
}

#[test]
fn under_over_constrained_and_invalid_export_paths_preserve_last_valid_state() {
    let mut under_constrained = seed();
    under_constrained
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteDrawingSheet { id: SHEET },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SELECTED,
                grounded: false,
            },
            CanonicalCommand::DeleteAssemblyMate { id: AXIAL_MATE },
        ]))
        .unwrap();
    let under_results = registry(&under_constrained.current(), false);
    let under_before = store_stamp(&under_constrained);
    let under_results_before = under_results.contents_stamp();
    let under_error = project_component_replacement_impact(
        &under_constrained,
        &under_results,
        ComponentReplacementImpactRequest::new(&under_constrained.current(), SELECTED, TARGET),
    )
    .unwrap_err();
    assert!(matches!(
        under_error,
        ComponentReplacementImpactError::Unsupported(reason)
            if reason.contains("UnderConstrained")
    ));
    assert_eq!(store_stamp(&under_constrained), under_before);
    assert_eq!(under_results.contents_stamp(), under_results_before);

    let mut over_constrained = seed();
    over_constrained
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: TARGET_OCCURRENCE,
                transform: Transform::from_translation(30.0, 2.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let over_results = registry(&over_constrained.current(), false);
    let over_before = store_stamp(&over_constrained);
    let over_results_before = over_results.contents_stamp();
    let over_drawing = project_orthographic_drawing(
        &over_constrained.current(),
        &over_results,
        over_constrained.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    let over_error = project_component_replacement_impact(
        &over_constrained,
        &over_results,
        ComponentReplacementImpactRequest::new(&over_constrained.current(), SELECTED, TARGET),
    )
    .unwrap_err();
    assert!(matches!(
        over_error,
        ComponentReplacementImpactError::Unsupported(reason)
            if reason.contains("OverConstrained")
    ));
    assert_eq!(store_stamp(&over_constrained), over_before);
    assert_eq!(over_results.contents_stamp(), over_results_before);
    assert_eq!(
        project_orthographic_drawing(
            &over_constrained.current(),
            &over_results,
            over_constrained.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        over_drawing
    );

    let export_document = seed();
    let export_snapshot = export_document.current();
    let source_package = Arc::new(ExactBodyPackage::from(package(
        &export_snapshot,
        SOURCE,
        "source-last-valid",
        ROLES,
    )));
    let mut invalid_target = package(
        &export_snapshot,
        TARGET,
        "target-last-valid-invalid-mesh",
        ROLES,
    );
    invalid_target.triangles[0].vertex_indices = [0, 0, 0];
    let invalid_results = ExactResultRegistry::accept(
        &export_snapshot,
        [
            source_package,
            Arc::new(ExactBodyPackage::from(invalid_target)),
        ],
    )
    .unwrap();
    let export_before = store_stamp(&export_document);
    let invalid_results_before = invalid_results.contents_stamp();
    let export_error = project_component_replacement_impact(
        &export_document,
        &invalid_results,
        ComponentReplacementImpactRequest::new(&export_snapshot, SELECTED, TARGET),
    )
    .unwrap_err();
    assert!(
        matches!(
            export_error,
            ComponentReplacementImpactError::Unsupported(ref reason)
                if reason.contains("invalid facet")
        ),
        "unexpected export failure: {export_error:?}"
    );
    assert_eq!(store_stamp(&export_document), export_before);
    assert_eq!(invalid_results.contents_stamp(), invalid_results_before);
    assert_eq!(
        invalid_results
            .get_body(&export_snapshot, SOURCE, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "source-last-valid"
    );
    assert_eq!(
        invalid_results
            .get_body(&export_snapshot, TARGET, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "target-last-valid-invalid-mesh"
    );
}

#[test]
fn stale_failed_lost_and_cyclic_inputs_preserve_canonical_history_and_exact_outputs() {
    let mut stale_document = dependency_free_seed();
    let mut stale_results = registry(&stale_document.current(), false);
    let stale_impact = project_component_replacement_impact(
        &stale_document,
        &stale_results,
        ComponentReplacementImpactRequest::new(&stale_document.current(), SELECTED, TARGET),
    )
    .unwrap();
    stale_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SIBLING,
                visible: false,
            },
        ]))
        .unwrap();
    let stale_before = store_stamp(&stale_document);
    let stale_results_before = stale_results.contents_stamp();
    assert_eq!(
        commit_component_replacement(&mut stale_document, &mut stale_results, &stale_impact,),
        Err(ComponentReplacementCommitError::Stale)
    );
    assert_eq!(store_stamp(&stale_document), stale_before);
    assert_eq!(stale_results.contents_stamp(), stale_results_before);

    let failed_document = dependency_free_seed();
    let failed_snapshot = failed_document.current();
    let failed_results = ExactResultRegistry::accept(
        &failed_snapshot,
        [Arc::new(ExactBodyPackage::from(package(
            &failed_snapshot,
            SOURCE,
            "source-last-valid",
            ROLES,
        )))],
    )
    .unwrap();
    let failed_before = store_stamp(&failed_document);
    let failed_results_before = failed_results.contents_stamp();
    assert_eq!(
        project_component_replacement_impact(
            &failed_document,
            &failed_results,
            ComponentReplacementImpactRequest::new(&failed_snapshot, SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Failed(TARGET, BodyId(1)))
    );
    assert_eq!(store_stamp(&failed_document), failed_before);
    assert_eq!(failed_results.contents_stamp(), failed_results_before);
    assert_eq!(
        failed_results
            .get_body(&failed_snapshot, SOURCE, BodyId(1))
            .unwrap()
            .unwrap()
            .result_key()
            .result_fingerprint,
        "source-last-valid"
    );

    let mut lost_document = seed();
    let lost_snapshot = lost_document.current();
    let lost_source = package(&lost_snapshot, SOURCE, "lost-source", ROLES);
    let mate = lost_snapshot.assembly_mate(MATE).unwrap();
    lost_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteDrawingSheet { id: SHEET },
            CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::lost(
                    SELECTED,
                    lost_source.reference(ExactFaceRole::Top).unwrap().clone(),
                ),
                mate.endpoint_b().clone(),
                mate.kind(),
            )),
        ]))
        .unwrap();
    let lost_results = registry(&lost_document.current(), false);
    let lost_before = store_stamp(&lost_document);
    let lost_results_before = lost_results.contents_stamp();
    assert_eq!(
        project_component_replacement_impact(
            &lost_document,
            &lost_results,
            ComponentReplacementImpactRequest::new(&lost_document.current(), SELECTED, TARGET),
        ),
        Err(ComponentReplacementImpactError::Lost(MATE))
    );
    assert_eq!(store_stamp(&lost_document), lost_before);
    assert_eq!(lost_results.contents_stamp(), lost_results_before);

    const CYCLIC_DEFINITION: DefinitionId = DefinitionId(700);
    const FIRST_SHELL: FeatureId = FeatureId(701);
    const SECOND_SHELL: FeatureId = FeatureId(702);
    let mut cyclic_document = DocumentStore::new();
    let cyclic_before = store_stamp(&cyclic_document);
    let shell = |target| FeatureKind::Shell {
        target,
        removed_faces: vec![StableFaceRole::new("replacement.cycle").unwrap()],
        thickness: Dimension::from_decimal("1").unwrap(),
    };
    let cycle_error = match cyclic_document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: CYCLIC_DEFINITION,
            name: "Rejected cycle".into(),
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
        Ok(_) => panic!("cyclic replacement source was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        cycle_error,
        CanonicalError::FeatureDependencyCycle(FIRST_SHELL)
    );
    assert_eq!(store_stamp(&cyclic_document), cyclic_before);
}
