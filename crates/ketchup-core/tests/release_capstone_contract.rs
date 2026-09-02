#![cfg(feature = "named-product-fixtures")]

use ketchup_core::document::DocumentId;
use ketchup_core::release_capstone::{
    CapstoneCapability, CapstoneObservationalPath, CapstoneOutputKind, CapstoneRefusalPath,
    CapstoneStage, CapstoneStageEvidence, ReleaseCapstoneContract, ReleaseCapstoneContractError,
};

fn complete_evidence(contract: &ReleaseCapstoneContract) -> Vec<CapstoneStageEvidence> {
    let revisions = [0, 3, 6, 7, 8, 9, 9];
    contract
        .stages
        .iter()
        .enumerate()
        .map(|(index, requirement)| {
            let digest = if requirement.stage == CapstoneStage::Reopened {
                format!("digest-{:?}", CapstoneStage::ComponentReplaced)
            } else {
                format!("digest-{:?}", requirement.stage)
            };
            CapstoneStageEvidence {
                stage: requirement.stage,
                document_id: DocumentId(42),
                source_revision: revisions[index],
                source_digest: digest,
                lineage: requirement.lineage.clone(),
                output_fingerprints: requirement
                    .required_outputs
                    .iter()
                    .map(|kind| {
                        let output_stage = if requirement.stage == CapstoneStage::Reopened {
                            CapstoneStage::ComponentReplaced
                        } else {
                            requirement.stage
                        };
                        (*kind, format!("fingerprint-{output_stage:?}-{kind:?}"))
                    })
                    .collect(),
            }
        })
        .collect()
}

#[test]
fn mechanical_release_capstone_contract_is_bounded_deterministic_and_complete() {
    let first = ReleaseCapstoneContract::mechanical_plate_fixture();
    let second = ReleaseCapstoneContract::mechanical_plate_fixture();

    first.validate().unwrap();
    assert_eq!(first, second);
    assert_eq!(first.contract_fingerprint(), second.contract_fingerprint());
    assert_eq!(first.fixture_name, "bounded-mechanical-plate-and-fasteners");
    assert_eq!(first.dimensions.plate_length_mm, 120);
    assert_eq!(first.dimensions.plate_width_mm, 80);
    assert_eq!(first.dimensions.plate_height_mm, 8);
    assert_eq!(first.dimensions.plate_pocket_diameter_mm, 20);
    assert_eq!(first.dimensions.fastener_diameter_mm, 12);
    assert_eq!(first.dimensions.fastener_height_mm, 30);
    assert_eq!(
        first.plate_feature_ids.all(),
        [110, 111, 112, 113, 114, 115].map(ketchup_core::document::FeatureId)
    );
    assert_eq!(
        first.shared_feature_ids.all(),
        [210, 211, 212].map(ketchup_core::document::FeatureId)
    );
    assert_eq!(
        first.replacement_feature_ids.all(),
        [310, 311, 312].map(ketchup_core::document::FeatureId)
    );
    assert_eq!(
        first.unique_feature_ids.all(),
        [313, 314, 315].map(ketchup_core::document::FeatureId)
    );
    assert_eq!(
        first
            .stages
            .iter()
            .map(|stage| stage.stage)
            .collect::<Vec<_>>(),
        CapstoneStage::ALL
    );
    assert_eq!(
        first.required_capabilities,
        vec![
            CapstoneCapability::PrincipalWorkplane,
            CapstoneCapability::ConstrainedSketch,
            CapstoneCapability::Pad,
            CapstoneCapability::Pocket,
            CapstoneCapability::SharedDefinitionEdit,
            CapstoneCapability::MakeUnique,
            CapstoneCapability::RigidOccurrencePlacement,
            CapstoneCapability::PlanarMate,
            CapstoneCapability::AxialMate,
            CapstoneCapability::FrontTopRightDrawing,
            CapstoneCapability::CompatibleComponentReplacement,
            CapstoneCapability::ExactRenderPick,
            CapstoneCapability::StepExport,
            CapstoneCapability::StlExport,
            CapstoneCapability::UndoRedo,
            CapstoneCapability::SaveOpen,
            CapstoneCapability::ManualAiParity,
        ]
    );
    assert_eq!(
        first.observational_paths,
        vec![
            CapstoneObservationalPath::Inspect,
            CapstoneObservationalPath::Preview,
            CapstoneObservationalPath::Cancel,
            CapstoneObservationalPath::Escape,
        ]
    );
    assert_eq!(
        first.refusal_paths,
        vec![
            CapstoneRefusalPath::Missing,
            CapstoneRefusalPath::Hidden,
            CapstoneRefusalPath::Stale,
            CapstoneRefusalPath::Failed,
            CapstoneRefusalPath::Ambiguous,
            CapstoneRefusalPath::Lost,
            CapstoneRefusalPath::Cyclic,
            CapstoneRefusalPath::CrossDocument,
            CapstoneRefusalPath::Unsupported,
        ]
    );

    let part_stage = &first.stages[1];
    assert_eq!(
        part_stage.required_outputs,
        vec![
            CapstoneOutputKind::ExactBrep,
            CapstoneOutputKind::RenderMesh,
            CapstoneOutputKind::PickMap,
        ]
    );
    for stage in &first.stages[2..] {
        assert_eq!(stage.required_outputs.len(), 8);
    }
    assert!(first.non_commit_paths_preserve_canonical_history);
    assert!(first.non_commit_paths_preserve_last_valid_outputs);
    assert!(first.stages[6].must_match_prior_canonical_state);
    assert!(!first.stages[6].must_advance_revision);
}

#[test]
fn contract_identity_sets_are_permutation_stable_but_lifecycle_order_is_not() {
    let canonical = ReleaseCapstoneContract::mechanical_plate_fixture();
    let mut permuted = canonical.clone();
    permuted.required_capabilities.reverse();
    permuted.observational_paths.reverse();
    permuted.refusal_paths.reverse();
    for requirement in &mut permuted.stages {
        requirement.lineage.definition_ids.reverse();
        requirement.lineage.body_ids.reverse();
        requirement.lineage.feature_ids.reverse();
        requirement.lineage.occurrence_ids.reverse();
        requirement.required_outputs.reverse();
    }

    permuted.validate().unwrap();
    assert_eq!(
        permuted.contract_fingerprint(),
        canonical.contract_fingerprint()
    );

    let mut duplicate_lineage = canonical.clone();
    let duplicate = duplicate_lineage.stages[2].lineage.definition_ids[0];
    duplicate_lineage.stages[2]
        .lineage
        .definition_ids
        .push(duplicate);
    assert_eq!(
        duplicate_lineage.validate(),
        Err(ReleaseCapstoneContractError::InvalidStageLineage(
            CapstoneStage::AssemblySolved
        ))
    );

    let mut duplicate_output = canonical.clone();
    let duplicate = duplicate_output.stages[2].required_outputs[0];
    duplicate_output.stages[2].required_outputs.push(duplicate);
    assert_eq!(
        duplicate_output.validate(),
        Err(ReleaseCapstoneContractError::InvalidOutputContract(
            CapstoneStage::AssemblySolved
        ))
    );

    let mut reordered_lifecycle = canonical;
    reordered_lifecycle.stages.swap(1, 2);
    assert_eq!(
        reordered_lifecycle.validate(),
        Err(ReleaseCapstoneContractError::InvalidStageOrder)
    );
}

#[test]
fn complete_stage_evidence_requires_stable_lineage_fresh_outputs_and_lossless_reopen() {
    let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
    let evidence = complete_evidence(&contract);
    let unchanged = evidence.clone();

    contract.validate_evidence(&evidence).unwrap();
    assert_eq!(
        evidence, unchanged,
        "evidence inspection must be observational"
    );
    assert_eq!(evidence[5].source_revision, evidence[6].source_revision);
    assert_eq!(evidence[5].source_digest, evidence[6].source_digest);
    assert_eq!(evidence[5].lineage, evidence[6].lineage);
    assert_eq!(
        evidence[5].output_fingerprints,
        evidence[6].output_fingerprints
    );
}

#[test]
fn evidence_validation_is_permutation_stable_but_duplicate_identity_fails_closed() {
    let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
    let mut permuted = complete_evidence(&contract);
    for stage in &mut permuted {
        stage.lineage.definition_ids.reverse();
        stage.lineage.body_ids.reverse();
        stage.lineage.feature_ids.reverse();
        stage.lineage.occurrence_ids.reverse();
        stage.output_fingerprints.reverse();
    }
    contract.validate_evidence(&permuted).unwrap();

    let mut duplicate = complete_evidence(&contract);
    duplicate[2].lineage.feature_ids[1] = duplicate[2].lineage.feature_ids[0];
    assert_eq!(
        contract.validate_evidence(&duplicate),
        Err(ReleaseCapstoneContractError::InvalidStageLineage(
            CapstoneStage::AssemblySolved
        ))
    );

    let mut duplicate_output = complete_evidence(&contract);
    duplicate_output[2].output_fingerprints[1].0 = duplicate_output[2].output_fingerprints[0].0;
    assert_eq!(
        contract.validate_evidence(&duplicate_output),
        Err(ReleaseCapstoneContractError::InvalidOutputContract(
            CapstoneStage::AssemblySolved
        ))
    );
}

#[test]
fn incomplete_cross_document_stale_and_missing_output_evidence_fail_closed() {
    let contract = ReleaseCapstoneContract::mechanical_plate_fixture();

    let mut incomplete = complete_evidence(&contract);
    incomplete.pop();
    assert_eq!(
        contract.validate_evidence(&incomplete),
        Err(ReleaseCapstoneContractError::IncompleteEvidence)
    );

    let mut cross_document = complete_evidence(&contract);
    cross_document[4].document_id = DocumentId(99);
    assert_eq!(
        contract.validate_evidence(&cross_document),
        Err(ReleaseCapstoneContractError::CrossDocument)
    );

    let mut non_monotonic = complete_evidence(&contract);
    non_monotonic[3].source_revision = non_monotonic[2].source_revision;
    assert_eq!(
        contract.validate_evidence(&non_monotonic),
        Err(ReleaseCapstoneContractError::NonMonotonicRevision(
            CapstoneStage::SharedDefinitionEdited
        ))
    );

    let mut replay_mismatch = complete_evidence(&contract);
    replay_mismatch[6].source_digest = "different-after-open".into();
    assert_eq!(
        contract.validate_evidence(&replay_mismatch),
        Err(ReleaseCapstoneContractError::CanonicalReplayMismatch)
    );

    let mut output_replay_mismatch = complete_evidence(&contract);
    output_replay_mismatch[6].output_fingerprints[6].1 = "different-step-after-open".into();
    assert_eq!(
        contract.validate_evidence(&output_replay_mismatch),
        Err(ReleaseCapstoneContractError::OutputReplayMismatch)
    );

    let mut missing_fingerprint = complete_evidence(&contract);
    missing_fingerprint[2].output_fingerprints[0].1.clear();
    assert_eq!(
        contract.validate_evidence(&missing_fingerprint),
        Err(ReleaseCapstoneContractError::MissingFingerprint(
            CapstoneStage::AssemblySolved,
            CapstoneOutputKind::ExactBrep
        ))
    );

    let mut wrong_lineage = complete_evidence(&contract);
    wrong_lineage[4].lineage.definition_ids.pop();
    assert_eq!(
        contract.validate_evidence(&wrong_lineage),
        Err(ReleaseCapstoneContractError::InvalidStageLineage(
            CapstoneStage::OccurrenceMadeUnique
        ))
    );
}
