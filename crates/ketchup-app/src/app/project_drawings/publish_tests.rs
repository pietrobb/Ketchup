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
use std::path::{Path, PathBuf};

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

/// Review 2026-11-06 r23 MF-201: a copy saved as another file of the same
/// name, `variant-b/house.ketchup`, took the set of its original, as the list
/// named the document id a copy shares and the file's name. It names the
/// file's token, which a copy saved as another file has anew.
#[test]
fn a_copy_saved_as_a_file_of_the_same_name_does_not_take_the_originals_set() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let owner = |folder: &str, token: &str| SetOwner {
        document: DocumentId(42),
        file: Some(directory.path().join(folder).join("house.ketchup")),
        token: Some(token.to_owned()),
        written: BTreeMap::new(),
    };
    let files = |text: &[u8]| vec![text.to_vec()];
    let mut original = owner("variant-a", "original");
    publish_noted(&path, format, &files(b"original"), &mut original).unwrap();
    let sheets = set_file_paths(&path, format, 1);
    let mut copy = owner("variant-b", "copy");
    assert!(
        matches!(
            publish_noted(&path, format, &files(b"copy"), &mut copy),
            Err(ProjectDrawingsError::SetOfAnotherProject { .. })
        ),
        "the copy's export was not refused"
    );
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"original");
    // The original, moved with its token, still has it.
    let mut moved = owner("archive", "original");
    publish_noted(&path, format, &files(b"moved"), &mut moved).unwrap();
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"moved");
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
        token: None,
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
        token: None,
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

/// Review 2026-11-07 P3: a file without a token, not saved since tokens came
/// or with no random bytes for one, is named in its set's list by the document
/// id its copies share, so its copy from a file manager, of its name in
/// another folder, took the set by the name. A file without a token takes a
/// set as the list it last wrote only; one with a token, moved, by its name
/// still.
#[test]
fn a_file_without_a_token_takes_no_set_by_its_name() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let sheets = set_file_paths(&path, format, 1);
    let publish =
        |owner: &mut SetOwner, file: &[u8]| publish_noted(&path, format, &[file.to_vec()], owner);
    let mut original = SetOwner {
        document: DocumentId(42),
        file: Some(directory.path().join("house.ketchup")),
        token: None,
        written: BTreeMap::new(),
    };
    publish(&mut original, b"original").unwrap();
    let mut copy = SetOwner {
        file: Some(directory.path().join("copy").join("house.ketchup")),
        written: BTreeMap::new(),
        ..original.clone()
    };
    assert!(
        matches!(
            publish(&mut copy, b"copy"),
            Err(ProjectDrawingsError::SetOfAnotherProject { .. })
        ),
        "the copy without a token is not the project"
    );
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"original");
    publish(&mut original, b"again").unwrap();

    let mut saved = SetOwner {
        token: Some("abc".to_owned()),
        ..original
    };
    publish(&mut saved, b"saved").unwrap();
    let mut moved = SetOwner {
        file: Some(directory.path().join("moved").join("house.ketchup")),
        written: BTreeMap::new(),
        ..saved
    };
    publish(&mut moved, b"moved").unwrap();
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"moved");
}

/// Review 2026-11-04 r19 MF-162 (capability review run 501): a list of Kečup
/// 0.3.2 names the project's file by its absolute path, compared as text, so
/// the same file opened as `c:\proj\dom.ketchup` after an export from
/// `C:\Proj\Dom.ketchup`, or by another spelling of its path, took the
/// project's own set for another project's. The file the list names is the
/// project's when it is the same file on disk; another file still is not.
#[test]
fn a_0_3_2_list_is_the_projects_by_its_file_on_disk_not_by_the_spelling_of_its_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    let format = SetFileFormat::Dxf;
    let project = directory.path().join("House.ketchup");
    std::fs::write(&project, b"project").unwrap();
    let other = directory.path().join("Other.ketchup");
    std::fs::write(&other, b"another project").unwrap();
    let sheets = set_file_paths(&path, format, 1);
    let list_path = set_list_path(&path, format);
    let mut spellings = vec![
        directory.path().join(".").join("House.ketchup"),
        std::fs::canonicalize(&project).unwrap(),
    ];
    if cfg!(windows) {
        spellings.push(directory.path().join("HOUSE.KETCHUP"));
        let lower = project.display().to_string().to_lowercase();
        spellings.push(PathBuf::from(lower));
    }
    for (index, spelling) in spellings.into_iter().enumerate() {
        let before = format!("sheet {index}");
        std::fs::write(&sheets[0], &before).unwrap();
        std::fs::write(
            &list_path,
            format!(
                "Ketchup drawing set of a project file\ndocument 42\nproject {}\nhouse-01.dxf {}\n",
                project.display(),
                sha256_hex(before.as_bytes())
            ),
        )
        .unwrap();
        let mut another = SetOwner {
            document: DocumentId(42),
            file: Some(other.clone()),
            token: None,
            written: BTreeMap::new(),
        };
        assert!(
            matches!(
                publish_noted(&path, format, &[b"other".to_vec()], &mut another),
                Err(ProjectDrawingsError::SetOfAnotherProject { .. })
            ),
            "another file is not the project's"
        );
        let mut owner = SetOwner {
            document: DocumentId(42),
            file: Some(spelling.clone()),
            token: None,
            written: BTreeMap::new(),
        };
        publish_noted(&path, format, &[b"again".to_vec()], &mut owner)
            .unwrap_or_else(|error| panic!("{} refused: {error:?}", spelling.display()));
        assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"again");
    }

    // Review 2026-11-06 MF-182 and P3: the file the list names is gone, the
    // project moved with its drawings, or on a share that is offline; the
    // file was opened to compare, on the window's thread. The same document's
    // file of that name is the project's, no file looked at.
    std::fs::write(&sheets[0], b"moved").unwrap();
    std::fs::write(
        &list_path,
        format!(
            "Ketchup drawing set of a project file\ndocument 42\nproject {}\nhouse-01.dxf {}\n",
            directory
                .path()
                .join("Jobs")
                .join("House.ketchup")
                .display(),
            sha256_hex(b"moved")
        ),
    )
    .unwrap();
    let mut moved = SetOwner {
        document: DocumentId(42),
        file: Some(project),
        token: None,
        written: BTreeMap::new(),
    };
    publish_noted(&path, format, &[b"moved again".to_vec()], &mut moved).unwrap();
    assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"moved again");
}

/// Review 2026-11-08 MF-243: a list of Kečup 0.3.2 was the project's by the
/// name of its file alone, so a copy of the project in another folder,
/// exporting into the original's drawings folder, replaced its drawings. The
/// file the list names is the project's when it lies where the project lies
/// from the list, wherever both were moved together; a copy elsewhere is
/// another project.
#[test]
fn a_copy_of_a_0_3_2_project_in_another_folder_does_not_take_the_originals_set() {
    let directory = tempfile::tempdir().unwrap();
    let shared = directory.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    let path = shared.join("house.dxf");
    let format = SetFileFormat::Dxf;
    let sheets = set_file_paths(&path, format, 1);
    let list_path = set_list_path(&path, format);
    let write_list = |project: &Path| {
        std::fs::write(&sheets[0], b"original").unwrap();
        std::fs::write(
            &list_path,
            format!(
                "Ketchup drawing set of a project file\ndocument 42\nproject {}\nhouse-01.dxf {}\n",
                project.display(),
                sha256_hex(b"original")
            ),
        )
        .unwrap();
    };
    let owner = |file: PathBuf| SetOwner {
        document: DocumentId(42),
        file: Some(file),
        token: None,
        written: BTreeMap::new(),
    };
    let original = directory.path().join("a").join("House.ketchup");
    // A copy in a folder above the drawings is where the original would be
    // had both been moved together, which no file opened tells apart
    // (2026-11-06 P3); one in a folder beside them is not.
    for copy in [
        directory.path().join("b").join("House.ketchup"),
        directory.path().join("c").join("d").join("House.ketchup"),
    ] {
        write_list(&original);
        assert!(
            matches!(
                publish_noted(&path, format, &[b"copy".to_vec()], &mut owner(copy.clone())),
                Err(ProjectDrawingsError::SetOfAnotherProject { .. })
            ),
            "{} took the set of {}",
            copy.display(),
            original.display()
        );
        assert_eq!(std::fs::read(&sheets[0]).unwrap(), b"original");
    }

    // Moved with its drawings: `a\House.ketchup` with `a\..\shared` is now
    // `moved\House.ketchup` with `moved\..\shared`, as one folder up from it.
    let moved_shared = directory.path().join("moved").join("shared");
    std::fs::create_dir_all(&moved_shared).unwrap();
    let moved_path = moved_shared.join("house.dxf");
    let moved_sheets = set_file_paths(&moved_path, format, 1);
    std::fs::write(&moved_sheets[0], b"original").unwrap();
    std::fs::write(
        set_list_path(&moved_path, format),
        format!(
            "Ketchup drawing set of a project file\ndocument 42\nproject {}\nhouse-01.dxf {}\n",
            directory.path().join("House.ketchup").display(),
            sha256_hex(b"original")
        ),
    )
    .unwrap();
    let mut moved = owner(directory.path().join("moved").join("House.ketchup"));
    publish_noted(&moved_path, format, &[b"moved".to_vec()], &mut moved).unwrap();
    assert_eq!(std::fs::read(&moved_sheets[0]).unwrap(), b"moved");
}
