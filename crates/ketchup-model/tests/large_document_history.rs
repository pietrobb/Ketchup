//! Large documents: a house keeps its Undo history through Save and Open, and a
//! document that saves also opens.
use ketchup_model::document::{CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId};
use ketchup_model::persistence::{self, PersistenceError};

fn container_entry_len(bytes: &[u8], wanted: &str) -> usize {
    let mut cursor = 16;
    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    for _ in 0..count {
        let path_len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let path = std::str::from_utf8(&bytes[cursor..cursor + path_len]).unwrap();
        cursor += path_len + 1;
        let len = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap()) as usize;
        if path == wanted {
            return len;
        }
        cursor += 8 + 32 + len;
    }
    panic!("no {wanted} in the container")
}

#[test]
fn a_house_keeps_its_undo_history_through_save_and_open() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/house-project.ketchup"
    );
    let bytes = persistence::read_native_document_file(path).unwrap();
    let (mut document, container) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .ok()
        .unwrap();
    let tag = document.current().tags().next().unwrap().id();
    let mut digests = vec![document.current().canonical_digest()];
    let mut saved = None;
    let mut first_history = 0;
    // Before format 4 every revision was a full copy: the third save of a house
    // no longer fit and Save offered to drop the Undo history.
    for step in 0..6 {
        let visible = document.current().tag(tag).unwrap().visible();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetTagVisibility {
                    id: tag,
                    visible: !visible,
                },
            ]))
            .unwrap();
        digests.push(document.current().canonical_digest());
        saved = Some(
            persistence::save_document_store(&document, &container)
                .expect("a house saves with its Undo history"),
        );
        if step == 0 {
            first_history = container_entry_len(saved.as_ref().unwrap(), "history.bin");
        }
    }
    let saved = saved.unwrap();
    let document_bin = container_entry_len(&saved, "document.bin");
    let history = container_entry_len(&saved, "history.bin");
    // The program source is stored once and every toggle only the chunks it changed.
    let per_toggle = (history - first_history) / 5;
    assert!(
        per_toggle < 200 * 1024 && history * 4 < document_bin,
        "{history} history bytes ({per_toggle} per toggle) next to a {document_bin}-byte document"
    );

    let mut reopened: DocumentStore = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.revision_count(), document.revision_count());
    assert_eq!(
        reopened.current().canonical_digest(),
        *digests.last().unwrap()
    );
    for expected in digests.iter().rev().skip(1) {
        reopened.undo().unwrap();
        assert_eq!(reopened.current().canonical_digest(), *expected);
    }
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
fn a_document_that_saves_also_opens_and_a_refused_save_names_the_size() {
    let container = persistence::ContainerData::default();
    let fitting = document_with_named_megabytes(30);
    let saved = persistence::save_document_store(&fitting, &container).unwrap();
    assert!(persistence::load(&saved).is_ok());

    // Saving used to allow document.bin up to the 64 MB file limit; opening refused
    // anything over 32 MB, so the saved file could not be opened again.
    let oversized = document_with_named_megabytes(34);
    let error = persistence::save_document_store(&oversized, &container).unwrap_err();
    let PersistenceError::TooLarge {
        entry,
        bytes,
        limit,
    } = &error
    else {
        panic!("unexpected error {error:?}")
    };
    assert_eq!(entry, "document.bin");
    assert_eq!(*limit, 32 * 1024 * 1024);
    assert!(*bytes > 34 * 1024 * 1024, "{bytes}");
    assert!(error.to_string().contains("34.0 MB"), "{error}");
    assert!(
        persistence::save_container(&oversized.current(), &container).is_err(),
        "a release or export of the same document is refused the same way"
    );
}
