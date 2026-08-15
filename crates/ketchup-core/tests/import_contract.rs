use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, FeatureKind,
    OccurrenceId, ProposalCommitError, Transform,
};
use ketchup_core::import::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision, StepImportEvidence,
    StepImportPlanError, plan_step_import,
};
use ketchup_core::persistence;

fn receipt(id: u64, definition_id: u64, occurrence_id: u64, source: &[u8]) -> ImportReceipt {
    ImportReceipt::from_source_bytes(
        ImportId(id),
        ImportFormat::Stl,
        source,
        "part.stl",
        ImportUnitDecision::new(
            ImportLengthUnit::Millimetre,
            ImportUnitAuthority::UserDeclared,
        ),
        "ketchup-stl",
        "1",
        vec![
            ImportDiagnostic::new(ImportDiagnosticSeverity::Info, "mesh.manifold", None, 1)
                .unwrap(),
        ],
        vec![
            ImportOutputRef::Definition(DefinitionId(definition_id)),
            ImportOutputRef::Occurrence(OccurrenceId(occurrence_id)),
        ],
    )
    .unwrap()
}

fn import_batch(id: u64, definition_id: u64, occurrence_id: u64, source: &[u8]) -> CommandBatch {
    CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(definition_id),
            name: "Imported part".to_owned(),
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(occurrence_id),
            definition_id: DefinitionId(definition_id),
            name: "Imported part".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt(id, definition_id, occurrence_id, source)),
    ])
}

#[test]
fn reviewed_import_is_one_undoable_persistent_deterministic_batch() {
    let source = b"solid deterministic-import-contract";
    let mut first = DocumentStore::new();
    let before = first.current().canonical_digest();
    let batch = import_batch(1, 1, 1, source);
    let batch_digest = batch.digest();
    let proposal = first.prepare_proposal(batch).unwrap();

    assert_eq!(first.current().canonical_digest(), before);
    assert_eq!(first.visible_undo_steps(), 0);

    first.commit_verified_proposal(&proposal).unwrap();
    let committed = first.current();
    let committed_digest = committed.canonical_digest();
    assert_ne!(committed_digest, before);
    assert_eq!(first.visible_undo_steps(), 1);
    assert_eq!(committed.import_receipts().count(), 1);
    assert_eq!(
        committed.import_receipt(ImportId(1)).unwrap().source_name(),
        "part.stl"
    );

    assert_eq!(first.undo().unwrap().canonical_digest(), before);
    assert_eq!(first.redo().unwrap().canonical_digest(), committed_digest);

    let encoded = persistence::save(&first.current());
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened.snapshot().import_receipt(ImportId(1)).unwrap(),
        &receipt(1, 1, 1, source)
    );
    assert_eq!(persistence::save(&reopened.snapshot()), encoded);

    let second_batch = import_batch(1, 1, 1, source);
    assert_eq!(second_batch.digest(), batch_digest);
}

#[test]
fn exact_step_import_is_one_deterministic_persistent_undoable_transaction() {
    let source = b"ISO-10303-21;DATA;#1=SI_UNIT(.MILLI.,.METRE.);ENDSEC;END-ISO-10303-21;";
    let evidence = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        solid_count: 1,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch = plan_step_import(&document.current(), source, "part.step", &evidence).unwrap();
    assert_eq!(
        batch.digest(),
        plan_step_import(&document.current(), source, "part.step", &evidence)
            .unwrap()
            .digest()
    );
    let proposal = document.prepare_proposal(batch).unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let exact = committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(exact.source_byte_len, source.len() as u64);
    assert_eq!(exact.result_fingerprint, evidence.result_fingerprint);
    let mut container = persistence::ContainerData::default();
    let source_hash = container.insert_import_blob(source.to_vec()).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    let undone =
        persistence::load(&persistence::save_container(&document.current(), &container).unwrap())
            .unwrap();
    assert!(!undone.container_data().blobs().contains_key(&source_hash));
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert!(matches!(
        persistence::load(&persistence::save(&document.current())),
        Err(persistence::PersistenceError::InvalidBlobHash)
    ));

    let encoded = persistence::save_container(&document.current(), &container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened
            .container_data()
            .blobs()
            .get(&source_hash)
            .map(Vec::as_slice),
        Some(source.as_slice())
    );
    assert_eq!(
        persistence::save_container(&reopened.snapshot(), reopened.container_data()).unwrap(),
        encoded
    );
}

#[test]
fn exact_step_plan_refuses_invalid_worker_evidence_without_mutation() {
    let document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let valid = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        solid_count: 1,
        volume_mm3: 1.0,
        bounds_mm: [[0.0; 3], [1.0; 3]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut invalid = valid;
    invalid.result_fingerprint.clear();
    assert_eq!(
        plan_step_import(
            &document.current(),
            b"#1=SI_UNIT(.MILLI.,.METRE.);",
            "part.step",
            &invalid,
        ),
        Err(StepImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn invalid_or_stale_import_leaves_the_published_state_unchanged() {
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let invalid = CommandBatch::new(vec![CanonicalCommand::RecordImport(receipt(
        1, 9, 9, b"invalid",
    ))]);
    let error = match document.apply_batch(&invalid) {
        Ok(_) => panic!("missing import outputs must reject the entire batch"),
        Err(error) => error,
    };
    assert_eq!(error, CanonicalError::InvalidImportReceipt);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);

    let proposal = document
        .prepare_proposal(import_batch(1, 2, 2, b"stale"))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Concurrent".to_owned(),
            },
        ]))
        .unwrap();
    let concurrent = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert!(matches!(
        document.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(document.current().canonical_digest(), concurrent);
    assert_eq!(document.visible_undo_steps(), undo_steps);
}
