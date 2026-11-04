//! Writing a DXF or DWG set and putting it back when that fails (review
//! 2026-11-04 P3).

use super::{
    SetFileFormat, SetOwner, pending_list, project_drawings_targets, publish_project_drawings,
    roll_back, set_file_paths, set_list_path, sheet_name,
};
use crate::export_bundle::ExportTransactionError;
use ketchup_model::document::DocumentId;
use ketchup_model::graph::sha256_hex;

/// A failed set export put back what it had written with `let _ =`, so a
/// sheet it could not put back was left half written while the answer said
/// nothing was written.
#[test]
fn a_file_the_roll_back_cannot_put_back_is_named_and_the_others_are_still_put_back() {
    let directory = tempfile::tempdir().unwrap();
    let replaced = directory.path().join("set-01.dxf");
    let new = directory.path().join("set-02.dxf");
    // The export replaced sheet 1, then another program changed it again.
    std::fs::write(&replaced, b"another program").unwrap();
    std::fs::write(&new, b"sheet 2").unwrap();

    let error = roll_back(
        Vec::new(),
        vec![
            (
                replaced.as_path(),
                b"sheet 1".as_slice(),
                Some(b"the user's sheet".to_vec()),
            ),
            (new.as_path(), b"sheet 2".as_slice(), None),
        ],
    )
    .unwrap_err();

    assert!(
        matches!(&error, ExportTransactionError::TargetChanged { path } if *path == replaced),
        "{error}"
    );
    assert_eq!(std::fs::read(&replaced).unwrap(), b"another program");
    assert!(!new.exists(), "the sheet after it is still removed");
}

/// An export stopped between its sheets and its list (the window closed, the
/// power cut) left sheets the list did not name, so every later export of the
/// set was refused as if they were another program's. While the sheets are
/// written, the list names each as it was and as it will be.
#[test]
fn an_export_stopped_between_the_sheets_and_the_list_does_not_block_the_next() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let document = &SetOwner {
        document: DocumentId(42),
        file: None,
    };
    let targets = |path| project_drawings_targets(path, format, document).unwrap();
    let first = [b"plan 1".to_vec(), b"section 1".to_vec()];
    publish_project_drawings(&path, format, &first, &targets(&path), document).unwrap();

    // The next export stopped after its first sheet: the list in place while
    // the sheets are written, and only sheet 1 new.
    let second = [b"plan 2".to_vec(), b"section 2".to_vec()];
    let sheets = set_file_paths(&path, format, 2);
    let named = sheets
        .iter()
        .zip(&second)
        .map(|(sheet, bytes)| (sheet_name(sheet), sha256_hex(bytes)))
        .collect::<Vec<_>>();
    let list_path = set_list_path(&path, format);
    std::fs::write(
        &list_path,
        pending_list(document, &named, &targets(&path), &list_path),
    )
    .unwrap();
    std::fs::write(&sheets[0], &second[0]).unwrap();

    publish_project_drawings(&path, format, &second, &targets(&path), document).unwrap();
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), second[0]);
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), second[1]);
    // The list names the sheets only as written now: the old section put
    // back by hand is not the set's.
    std::fs::write(&sheets[1], &first[1]).unwrap();
    let third = [b"plan 3".to_vec()];
    assert!(
        publish_project_drawings(&path, format, &third, &targets(&path), document).is_err(),
        "a sheet changed since is not the set's"
    );
}

/// Save As keeps the project's id, so a copy saved under another name took
/// the set its original exported to a folder for its own and replaced it.
/// The list names the project's file too; a list written before it did, or
/// before the project was saved, is still the project's.
#[test]
fn a_copy_saved_as_another_file_does_not_replace_the_set_of_the_original() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let owner = |file: Option<&str>| SetOwner {
        document: DocumentId(42),
        file: file.map(|file| directory.path().join(file)),
    };
    let publish = |owner: &SetOwner, files: &[&[u8]]| {
        let files = files.iter().map(|file| file.to_vec()).collect::<Vec<_>>();
        let targets = project_drawings_targets(&path, format, owner).unwrap();
        publish_project_drawings(&path, format, &files, &targets, owner)
    };
    let sheets = set_file_paths(&path, format, 2);
    let original = owner(Some("house.ketchup"));
    publish(&original, &[b"original 1", b"original 2"]).unwrap();

    let copy = owner(Some("house copy.ketchup"));
    assert!(publish(&copy, &[b"copy 1", b"copy 2"]).is_err());
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"original 1");
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), b"original 2");
    publish(&original, &[b"again 1", b"again 2"]).unwrap();

    // Drawn before the project was saved, or listed by its id alone.
    let list_path = set_list_path(&path, format);
    std::fs::remove_file(&list_path).unwrap();
    for sheet in &sheets {
        std::fs::remove_file(sheet).unwrap();
    }
    publish(&owner(None), &[b"unsaved 1", b"unsaved 2"]).unwrap();
    publish(&original, &[b"saved 1", b"saved 2"]).unwrap();
    std::fs::write(
        &list_path,
        format!(
            "Ketchup drawing set\ndocument 42\nhouse-01.dxf {}\nhouse-02.dxf {}\n",
            sha256_hex(b"saved 1"),
            sha256_hex(b"saved 2")
        ),
    )
    .unwrap();
    publish(&original, &[b"listed by id 1", b"listed by id 2"]).unwrap();
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), b"listed by id 2");
}
