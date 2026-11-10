//! `file action=import` of a SketchUp `.skp` file: it imports as the GLB `ketchup-skp`
//! converts it to, in one Undo step and as a job the window does not wait for; the review
//! and the receipt name the SketchUp version, and a damaged file is refused without
//! changing the document.

use super::*;
use crate::live_bridge::dwg_import::imports_as_job;
use ketchup_model::import::ImportFormat;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../ketchup-skp/fixtures")
            .join(name),
    )
    .unwrap()
}

fn import(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: false,
        revision_of: None,
        code_page: None,
    }
}

#[test]
fn a_sketchup_file_imports_as_its_glb_in_one_undo_step_noting_its_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("solid.skp");
    let source = fixture("SU_File.skp");
    std::fs::write(&path, &source).unwrap();
    assert!(
        imports_as_job(&import(&path)),
        "read off the window's thread"
    );

    let (mut app, mut bridge) = setup();
    let (occurrences, undo) = (app.occurrence_count(), app.undo_step_count());
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));

    assert_eq!(answer["imported"], true, "{answer}");
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    assert!(app.occurrence_count() > occurrences, "{answer}");
    let version = answer["review"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "skp_version_verified")
        .unwrap_or_else(|| panic!("no SketchUp version in {answer}"));
    assert!(
        version["subject"].as_str().unwrap().starts_with("{25."),
        "{version}"
    );

    let snapshot = app.document_snapshot();
    let receipt = snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Glb)
        .unwrap();
    assert_eq!(receipt.source_name(), "solid.skp");
    assert!(
        receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "skp_version_verified"),
        "the receipt keeps the version"
    );
}

/// The boxes on the tag "Walls", hidden in the file, import on a hidden tag "Walls": showing
/// the tag shows them. Before, every part imported untagged and the tag was only a note.
#[test]
fn the_parts_of_a_tagged_component_import_on_that_tag_hidden_as_in_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tagged.skp");
    std::fs::write(&path, fixture("tagged_boxes.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let before = app.document_snapshot();
    let answer = bridge
        .execute(&mut app, import(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["imported"], true, "{answer}");

    let snapshot = app.document_snapshot();
    let walls = snapshot
        .tags()
        .find(|tag| tag.name() == "Walls")
        .unwrap_or_else(|| panic!("no tag Walls after {answer}"));
    assert!(!walls.visible(), "hidden as in the file");
    let imported: Vec<_> = snapshot
        .occurrences()
        .filter(|occurrence| before.occurrence(occurrence.id()).is_none())
        .collect();
    assert_eq!(imported.len(), 2, "both boxes");
    for occurrence in imported {
        assert_eq!(
            occurrence.tags(),
            &std::collections::BTreeSet::from([walls.id()])
        );
    }
}

#[test]
fn a_damaged_sketchup_file_is_refused_and_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged.skp");
    let source = fixture("SU_File.skp");
    std::fs::write(&path, &source[..source.len() / 3]).unwrap();
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    assert_eq!(
        bridge.execute(&mut app, import(&path), false),
        Err("import_failed")
    );
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo);
}
