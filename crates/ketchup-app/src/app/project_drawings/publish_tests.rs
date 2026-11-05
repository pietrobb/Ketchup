//! Writing a DXF or DWG set and putting it back when that fails (review
//! 2026-11-04 P3).

use super::{
    ProjectDrawingsError, SetFileFormat, SetOwner, pending_list, project_drawings_targets,
    publish_noted, roll_back, set_file_paths, set_list_path, sheet_name,
};
use crate::export_bundle::ExportTransactionError;
use ketchup_model::document::DocumentId;
use ketchup_model::graph::sha256_hex;
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    let mut owner = SetOwner {
        document: DocumentId(42),
        file: None,
        written: BTreeMap::new(),
    };
    let targets =
        |owner: &SetOwner, path| project_drawings_targets(path, format, owner, false).unwrap();
    let first = [b"plan 1".to_vec(), b"section 1".to_vec()];
    publish_noted(&path, format, &first, &mut owner).unwrap();

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
        pending_list(&owner, &named, &targets(&owner, &path), &list_path),
    )
    .unwrap();
    std::fs::write(&sheets[0], &second[0]).unwrap();

    // The window closed: the document noted neither the list in place nor
    // the new one, only the list before, which the one in place replaces.
    publish_noted(&path, format, &second, &mut owner).unwrap();
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), second[0]);
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), second[1]);
    // The list names the sheets only as written now: the old section put
    // back by hand is not the set's.
    std::fs::write(&sheets[1], &first[1]).unwrap();
    let third = [b"plan 3".to_vec()];
    assert!(
        publish_noted(&path, format, &third, &mut owner).is_err(),
        "a sheet changed since is not the set's"
    );
}

/// Save As keeps the project's id, so a copy saved under another name took
/// the set its original exported to a folder for its own and replaced it.
/// Review 2026-11-04 r19 MF-162: the list then named the project by its
/// absolute path, so a project renamed or moved refused its own set. The set
/// is the document's that wrote its list last: a copy that did not is told
/// whose set it is; one saved as "v2" after the export goes on with it, and
/// then the original is told. Lists of Kečup 0.3.2 and older still read.
#[test]
fn a_set_is_the_documents_that_wrote_it_last_whatever_its_file_is_called() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let owner = |file: Option<&str>| SetOwner {
        document: DocumentId(42),
        file: file.map(|file| directory.path().join(file)),
        written: BTreeMap::new(),
    };
    let publish = |owner: &mut SetOwner, files: &[&[u8]]| {
        let files = files.iter().map(|file| file.to_vec()).collect::<Vec<_>>();
        publish_noted(&path, format, &files, owner)
    };
    let exported_from = |result: Result<Vec<PathBuf>, ProjectDrawingsError>| match result {
        Err(ProjectDrawingsError::SetOfAnotherProject { project, .. }) => project,
        other => panic!("not refused as another project's set: {other:?}"),
    };
    let sheets = set_file_paths(&path, format, 2);
    let mut copy = owner(Some("house copy.ketchup"));
    let mut original = owner(Some("house.ketchup"));
    publish(&mut original, &[b"original 1", b"original 2"]).unwrap();
    let list = std::fs::read_to_string(set_list_path(&path, format)).unwrap();
    assert!(list.contains("project house.ketchup\n"), "{list}");
    assert!(
        !list.contains(&directory.path().display().to_string()),
        "{list}"
    );

    // A copy saved before the export did not write the set.
    assert_eq!(
        exported_from(publish(&mut copy, &[b"copy 1", b"copy 2"])),
        Some("house.ketchup".to_owned())
    );
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"original 1");
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), b"original 2");

    // Saved as "v2" (or moved, renamed) after it, the project goes on with it.
    let mut v2 = SetOwner {
        file: Some(
            directory
                .path()
                .join("other folder")
                .join("house v2.ketchup"),
        ),
        ..original.clone()
    };
    publish(&mut v2, &[b"v2 1", b"v2 2"]).unwrap();
    assert_eq!(
        exported_from(publish(&mut original, &[b"again 1", b"again 2"])),
        Some("house v2.ketchup".to_owned())
    );
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"v2 1");

    // Drawn before the project was saved, then saved: still its.
    let mut unsaved = owner(None);
    let list_path = set_list_path(&path, format);
    std::fs::remove_file(&list_path).unwrap();
    for sheet in &sheets {
        std::fs::remove_file(sheet).unwrap();
    }
    publish(&mut unsaved, &[b"unsaved 1", b"unsaved 2"]).unwrap();
    unsaved.file = Some(directory.path().join("house.ketchup"));
    publish(&mut unsaved, &[b"saved 1", b"saved 2"]).unwrap();

    // A list of Kečup 0.3.2 names the project's file by its path; an older
    // one the project's id alone.
    let legacy = |header: String| {
        std::fs::write(
            &list_path,
            format!(
                "{header}house-01.dxf {}\nhouse-02.dxf {}\n",
                sha256_hex(b"saved 1"),
                sha256_hex(b"saved 2")
            ),
        )
        .unwrap();
    };
    let project = directory.path().join("house.ketchup");
    legacy(format!(
        "Ketchup drawing set of a project file\ndocument 42\nproject {}\n",
        project.display()
    ));
    assert_eq!(
        exported_from(publish(&mut copy, &[b"copy 1", b"copy 2"])),
        Some("house.ketchup".to_owned())
    );
    publish(&mut owner(Some("house.ketchup")), &[b"saved 1", b"saved 2"]).unwrap();
    legacy("Ketchup drawing set\ndocument 42\n".to_owned());
    publish(&mut owner(None), &[b"listed by id 1", b"listed by id 2"]).unwrap();
    assert_eq!(std::fs::read(&sheets[1]).unwrap(), b"listed by id 2");
}
