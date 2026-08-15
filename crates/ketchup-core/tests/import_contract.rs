use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, OccurrenceId,
    ProposalCommitError, Transform,
};
use ketchup_core::import::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
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
