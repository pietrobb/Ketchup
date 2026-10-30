//! Large documents: a house edit checkpoints only its own step, and a document
//! that saves also opens.
use ketchup_model::document::{CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId};
use ketchup_model::persistence;

/// Idea #49: every edit of a saved house wrote its whole 6 MB work-recovery
/// checkpoint again. An edit or an Undo now appends only its own step to the
/// checkpoint, and after a crash the checkpoint still opens at the last edit
/// with the same Undo and Redo steps.
#[test]
fn a_house_edit_appends_only_its_step_to_the_work_recovery_and_a_crash_restores_it() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/house-project.ketchup"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.ketchup");
    std::fs::copy(source, &path).unwrap();
    let recovery_path = persistence::work_recovery_path(&path);
    let bytes = persistence::read_native_document_file(&path).unwrap();
    let base = persistence::FileIdentity::from_bytes(&bytes);
    let (mut document, container) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .ok()
        .unwrap();
    let tag = document.current().tags().next().unwrap().id();
    let mut recovery = None;
    let mut written: Option<Vec<u8>> = None;
    // An edit, an edit, Undo, an edit that drops the Redo step, an edit, Undo.
    for (step, undo) in [false, false, true, false, false, true]
        .into_iter()
        .enumerate()
    {
        document
            .try_canonical_transaction(
                |document| {
                    if undo {
                        return document
                            .undo()
                            .map(drop)
                            .ok_or("nothing to undo".to_owned());
                    }
                    let visible = document.current().tag(tag).unwrap().visible();
                    document
                        .apply_batch(&CommandBatch::new(vec![
                            CanonicalCommand::SetTagVisibility {
                                id: tag,
                                visible: !visible,
                            },
                        ]))
                        .map(drop)
                        .map_err(|error| error.to_string())
                },
                |document| {
                    recovery = Some(
                        persistence::publish_work_recovery(
                            &path, document, &container, base, recovery,
                        )
                        .map_err(|error| error.to_string())?,
                    );
                    Ok(())
                },
            )
            .unwrap();
        let file = std::fs::read(&recovery_path).unwrap();
        if let Some(before) = &written {
            assert!(
                file.starts_with(before),
                "step {step} rewrote the {}-byte checkpoint instead of appending to it",
                before.len()
            );
            let appended = file.len() - before.len();
            assert!(
                appended > 0 && appended < 64 * 1024,
                "step {step} appended {appended} bytes"
            );
        }
        written = Some(file);
    }

    // A crash: nothing is saved, the next open finds the checkpoint.
    let opened = persistence::load_file_with_source(&path).unwrap();
    assert_eq!(opened.source_path(), recovery_path);
    let mut recovered = opened.into_parts().0.into_editable().ok().unwrap();
    let ids = |store: &DocumentStore| {
        store
            .revision_history()
            .map(|revision| revision.id())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&recovered), ids(&document));
    assert_eq!(recovered.history_cursor(), document.history_cursor());
    assert_eq!(recovered.history_digest(), document.history_digest());
    assert_eq!(
        recovered.current().canonical_digest(),
        document.current().canonical_digest()
    );
    document.redo().unwrap();
    recovered.redo().unwrap();
    assert_eq!(
        recovered.current().canonical_digest(),
        document.current().canonical_digest()
    );
}

fn document_with_named_megabytes(megabytes: u64) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(
            (1..=megabytes)
                .map(|id| CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(id),
                    name: format!("{id:04}{}", "x".repeat(1024 * 1024 - 8)),
                    dimension: Dimension::new("0", 0.0).unwrap(),
                    dependencies: Vec::new(),
                })
                .collect(),
        ))
        .unwrap();
    document
}

#[test]
fn a_document_over_32_mib_saves_and_opens() {
    // document.bin has no size limit of its own: it is as large as the document.
    let container = persistence::ContainerData::default();
    let large = document_with_named_megabytes(34);
    let saved = persistence::save_document_store(&large, &container).unwrap();
    assert!(saved.len() > 34 * 1024 * 1024, "{}", saved.len());
    assert_eq!(
        persistence::load(&saved)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        large.current().canonical_digest()
    );
    let released = persistence::save_container(&large.current(), &container).unwrap();
    assert!(persistence::load(&released).is_ok());
}

/// The container schema of a saved file, and the digest of the document it opens at.
fn opened(bytes: &[u8]) -> (u16, String) {
    let persistence::LoadOutcome::Editable { document, .. } = persistence::load(bytes).unwrap()
    else {
        panic!("the file opens editable")
    };
    (
        u16::from_le_bytes([bytes[10], bytes[11]]),
        document.current().canonical_digest(),
    )
}

/// Review 2026-10-29 proposal 5.4: Kečup 0.2.0 takes a file with an entry over 32 MiB
/// for damaged, whatever format its document is in, opens its `.recovery` in its place
/// and overwrites the newer file at its next Save. Such a file is written as container
/// schema 2, which 0.2.0 checks first and refuses as newer, and opens here as before.
#[test]
fn a_file_kecup_0_2_0_would_take_for_damaged_is_refused_by_it_as_newer() {
    let mut container = persistence::ContainerData::default();
    let large = document_with_named_megabytes(34);
    let digest = large.current().canonical_digest();
    for saved in [
        persistence::save_document_store(&large, &container).unwrap(),
        persistence::save_document_store_current_snapshot(&large, &container).unwrap(),
        persistence::save_container(&large.current(), &container).unwrap(),
    ] {
        assert_eq!(opened(&saved), (2, digest.clone()));
    }

    let small = document_with_named_megabytes(1);
    let digest = small.current().canonical_digest();
    let save = |container: &persistence::ContainerData| {
        opened(&persistence::save_document_store(&small, container).unwrap())
    };
    assert_eq!(save(&container), (1, digest.clone()));
    let mut one = container.clone();
    one.insert_blob(vec![1; 32 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(save(&one), (2, digest.clone()));
    // Entries within 32 MiB in a file over 64 MiB: 0.2.0 refuses the file by its size,
    // before reading it, and keeps no `.recovery` in its place.
    for byte in 2..=4 {
        container.insert_blob(vec![byte; 22 * 1024 * 1024]).unwrap();
    }
    assert_eq!(save(&container), (1, digest));
}
