//! Every committed native document must open and roundtrip without changing its
//! model. Older writers can have different digests; current documents need no
//! migration. The fixture set must exercise both cases.

use std::path::{Path, PathBuf};

use ketchup_model::persistence::{self, LoadOutcome};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives in <root>/crates/<name>")
        .to_path_buf()
}

fn native_documents(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                native_documents(&path, found);
            }
        } else if name.ends_with(".ketchup")
            || path
                .ancestors()
                .any(|ancestor| ancestor.ends_with("fixtures/persistence"))
        {
            found.push(path);
        }
    }
}

fn resave(outcome: LoadOutcome) -> Vec<u8> {
    match outcome.into_editable_with_container() {
        Ok((document, container_data)) => {
            persistence::save_document_store(&document, &container_data).unwrap()
        }
        Err(candidate) => {
            persistence::save_container(candidate.snapshot(), candidate.container_data()).unwrap()
        }
    }
}

#[test]
fn every_committed_document_migrates_to_the_current_schema_without_change() {
    let root = repository_root();
    let mut documents = Vec::new();
    for directory in ["crates", "examples"] {
        native_documents(&root.join(directory), &mut documents);
    }
    assert!(
        documents.len() >= 10,
        "expected the committed fixtures, found {documents:?}"
    );

    let mut audited_recipes = 0;
    let mut rejected = 0;
    let mut changed_digests = 0;
    let mut unchanged_digests = 0;
    for path in documents {
        let bytes = std::fs::read(&path).unwrap();
        if path
            .parent()
            .is_some_and(|parent| parent.ends_with("invalid"))
        {
            assert!(
                persistence::load(&bytes).is_err(),
                "{} is invalid yet opens",
                path.display()
            );
            rejected += 1;
            continue;
        }
        let loaded = persistence::load(&bytes)
            .unwrap_or_else(|error| panic!("{} does not open: {error}", path.display()));
        let source_schema = loaded.source_schema();
        assert_eq!(
            loaded.audit().history_discarded,
            None,
            "{} lost its Undo history",
            path.display()
        );
        let snapshot = loaded.snapshot();
        let editable = loaded.is_editable();
        if loaded.audit().source_canonical_digest == snapshot.canonical_digest() {
            unchanged_digests += 1;
        } else {
            changed_digests += 1;
        }
        if let Some(recipe) = snapshot.assembly_recipe() {
            recipe.audit(&snapshot).unwrap_or_else(|error| {
                panic!(
                    "{} recipe no longer owns its features: {error:?}",
                    path.display()
                )
            });
            audited_recipes += 1;
        }

        let migrated = resave(loaded);
        let reopened = persistence::load(&migrated).unwrap_or_else(|error| {
            panic!("{} migrated copy does not open: {error}", path.display())
        });
        assert_eq!(
            reopened.source_schema(),
            persistence::CURRENT_SCHEMA,
            "{}",
            path.display()
        );
        assert_eq!(reopened.is_editable(), editable, "{}", path.display());
        let reopened_snapshot = reopened.snapshot();
        assert_eq!(
            reopened_snapshot.occurrences().collect::<Vec<_>>(),
            snapshot.occurrences().collect::<Vec<_>>(),
            "{} changed occurrence geometry or placement",
            path.display()
        );
        assert_eq!(
            reopened_snapshot.features().collect::<Vec<_>>(),
            snapshot.features().collect::<Vec<_>>(),
            "{} changed model features",
            path.display()
        );
        assert_eq!(
            reopened.audit().source_canonical_digest,
            reopened_snapshot.canonical_digest(),
            "{}",
            path.display()
        );
        assert_eq!(
            reopened_snapshot.canonical_digest(),
            snapshot.canonical_digest(),
            "{} (schema {source_schema}) changed during migration",
            path.display()
        );
        assert_eq!(resave(reopened), migrated, "{}", path.display());
    }
    assert!(
        audited_recipes > 0,
        "no committed document has an assembly recipe"
    );
    assert!(rejected > 0, "no committed invalid document is exercised");
    assert!(changed_digests > 0, "no older writer digest is migrated");
    assert!(
        unchanged_digests > 0,
        "no unchanged writer digest is exercised"
    );
}
