//! Where a client writes files (owner decision R1 of review 2026-11-19 og):
//! a new file anywhere the system lets the user write, unasked; never in the
//! system's own places; each file written listed for the Connect tab.

use super::*;
use crate::live_bridge::write_policy::{Place, refusal, system_place, system_places};

/// A closed profile in a DXF without a header, a model a DXF export writes
/// without an exact result.
const PROFILE_DXF: &[u8] = b"0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nclosed\n90\n3\n70\n1\n\
10\n20\n20\n0\n10\n30\n20\n0\n10\n20\n20\n10\n\
0\nENDSEC\n0\nEOF\n";

fn export(path: &Path) -> Request {
    Request::Export(crate::live_bridge::interchange::ExportRequest {
        expected: None,
        path: path.display().to_string(),
        export_format: "dxf".to_owned(),
        drawing_scale: None,
        image: None,
        cam_dialect: None,
        cam_operations: None,
        overwrite: false,
    })
}

fn import(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: Some("mm".to_owned()),
        as_underlay: false,
        revision_of: None,
        code_page: None,
    }
}

/// A window with a profile imported from a DXF, saved as
/// `project/house.ketchup` under `directory`.
fn saved_window(
    directory: &Path,
    dialogs: ScriptedFileDialogs,
) -> (KetchupApp, LiveBridge, PathBuf) {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let document = directory.join("project").join("house.ketchup");
    std::fs::create_dir_all(document.parent().unwrap()).unwrap();
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let source = document.with_file_name("profile.dxf");
    std::fs::write(&source, PROFILE_DXF).unwrap();
    bridge.execute(&mut app, import(&source), false).unwrap();
    let request = Request::SaveAs {
        expected: Some(app.live_bridge_stamp()),
        path: document.display().to_string(),
    };
    bridge.execute(&mut app, request, false).unwrap();
    (app, bridge, document)
}

/// Review 2026-11-08 §5.4 had a client export unasked only into the
/// document's folder, one in Open Recent or the temporary folder, and asked
/// in the window before any other; R1119og-SEC-800 found the question
/// protected nothing. Every folder the user may write into is written unasked
/// now, the document's and any other, one named Startup too when it is not
/// the system's.
#[test]
fn a_new_file_is_written_into_any_folder_of_the_users_unasked() {
    let directory = tempfile::tempdir().unwrap();
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let (mut app, mut bridge, document) = saved_window(directory.path(), dialogs);
    for written in [
        document.with_file_name("house.dxf"),
        directory.path().join("elsewhere").join("house.dxf"),
        directory
            .path()
            .join("Startup")
            .join("new")
            .join("house.dxf"),
    ] {
        std::fs::create_dir_all(written.parent().unwrap()).unwrap();
        bridge
            .execute(&mut app, export(&written), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        assert!(written.exists(), "{}", written.display());
    }
    let copy = directory.path().join("copies").join("house.ketchup");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    let request = Request::SaveAs {
        expected: Some(app.live_bridge_stamp()),
        path: copy.display().to_string(),
    };
    bridge.execute(&mut app, request, false).unwrap();
    assert!(copy.exists());
    assert!(probe.high_risk_prompts().is_empty(), "nothing is asked");
}

/// The system's own places are refused, each with its reason: a folder that
/// starts programs at login and those under it, the system's folder and those
/// under it, and the drive's root only as itself, not its folders.
#[test]
fn a_system_place_is_named_by_the_folder_or_the_one_it_is_under() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let startup = root.join("Start Menu").join("Startup");
    let windows = root.join("Windows");
    std::fs::create_dir_all(&startup).unwrap();
    std::fs::create_dir_all(&windows).unwrap();
    let places = [
        (startup.clone(), Place::Startup),
        (windows.clone(), Place::System),
        (root.clone(), Place::DriveRoot),
    ];
    assert_eq!(system_place(&startup, &places), Some(Place::Startup));
    assert_eq!(
        system_place(&startup.join("still").join("to make"), &places),
        Some(Place::Startup)
    );
    assert_eq!(
        system_place(&windows.join("System32"), &places),
        Some(Place::System)
    );
    assert_eq!(system_place(&root, &places), Some(Place::DriveRoot));
    assert_eq!(system_place(&root.join("Users"), &places), None);
    assert_eq!(system_place(&root.join("Start Menu"), &places), None);
    assert_eq!(system_place(&root.join("Windows-old"), &places), None);
    if cfg!(windows) {
        let shouted = PathBuf::from(windows.to_string_lossy().to_uppercase());
        assert_eq!(system_place(&shouted, &places), Some(Place::System));
    }
}

/// The Startup folders are where the shell keeps them, not only where the
/// environment says: a folder moved by "User Shell Folders" is still refused
/// (review 2026-11-20 ac P3).
#[cfg(windows)]
#[test]
fn the_startup_folders_the_shell_names_are_system_places() {
    let known = crate::live_bridge::write_policy::known_startup_folders();
    assert_eq!(known.len(), 2, "the user's and all users': {known:?}");
    let places = system_places();
    for folder in &known {
        assert!(
            places.contains(&(folder.clone(), Place::Startup)),
            "{folder:?} in {places:?}"
        );
        let inside = folder.join("run.cmd");
        if let Ok(Some(resolved)) = crate::live_bridge::write_policy::folder_of(&inside) {
            assert_eq!(system_place(&resolved, &places), Some(Place::Startup));
        }
    }
}

/// A file named at the root of the system drive is refused before anything
/// is written, saying which place and why.
#[test]
fn a_file_at_the_root_of_the_system_drive_is_refused_with_the_reason() {
    let places = system_places();
    let written = [Place::DriveRoot, Place::System]
        .into_iter()
        .filter_map(|kind| places.iter().find(|(_, place)| *place == kind))
        .map(|(folder, place)| (folder.join("ketchup-r1-test.dxf"), place.name()));
    let written: Vec<_> = written.collect();
    assert_eq!(written.len(), if cfg!(windows) { 2 } else { 1 });
    for (path, place) in written {
        assert_eq!(
            refusal(&export(&path)),
            Some("system_folder"),
            "{}",
            path.display()
        );
        let details = take_error_details().unwrap();
        assert_eq!(details["details"]["place"], place, "{details}");
        assert!(details.to_string().contains("so Kečup writes"), "{details}");
        assert!(!path.exists());
    }
}

/// Before, a folder that did not resolve was taken as one still to be made,
/// whatever kept it from resolving: a link in the project's folder whose
/// target is gone stood for a folder of the project, and what was written
/// there went to the target. Only a folder that is not there is still to be
/// made; the link answers its error.
#[cfg(unix)]
#[test]
fn a_link_whose_target_is_gone_is_no_folder_still_to_be_made() {
    use crate::live_bridge::write_policy::folder_of;
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let canonical = std::fs::canonicalize(&project).unwrap();
    assert_eq!(
        folder_of(&project.join("new").join("house.dxf")).unwrap(),
        Some(canonical.join("new"))
    );
    let link = project.join("link");
    std::os::unix::fs::symlink(directory.path().join("gone"), &link).unwrap();
    let error = folder_of(&link.join("house.dxf")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(folder_of(&link.join("deeper").join("house.dxf")).is_err());
}

/// Review 2026-11-08 SEC-240 held for `save_as` only: `apply_and_verify` saved
/// the document under any name. It saves a `.ketchup` document only, in any
/// folder, as `save_as` does.
#[test]
fn apply_and_verify_saves_only_a_ketchup_document() {
    let directory = tempfile::tempdir().unwrap();
    let saved_at = |path: &Path| Request::ApplyAndVerify {
        expected: None,
        selection: None,
        program: program(),
        validators: Vec::new(),
        timeout_ms: 1,
        strict: false,
        save: Some(ApplyAndVerifySave::Path {
            path: path.display().to_string(),
        }),
    };
    let folder = directory.path().join("documents");
    std::fs::create_dir_all(&folder).unwrap();
    for refused in [folder.join("run.cmd"), folder.join(".house.ketchup")] {
        assert_eq!(refusal(&saved_at(&refused)), Some("invalid_path"));
    }
    assert_eq!(refusal(&saved_at(&folder.join("house.ketchup"))), None);
}

/// What a client writes over its connection is listed for the Connect tab:
/// the path and size of each file, the newest first; a request refused or
/// writing nothing adds none.
#[test]
fn each_file_a_client_writes_is_listed_for_the_connect_tab() {
    let directory = tempfile::tempdir().unwrap();
    let mut wire = Wire::new();
    let source = directory.path().join("profile.dxf");
    std::fs::write(&source, PROFILE_DXF).unwrap();
    wire.call(import(&source));
    assert!(wire.app.live.write_log.files().is_empty(), "an import");

    let first = directory.path().join("out").join("house.dxf");
    let second = directory.path().join("elsewhere").join("plan.dxf");
    for path in [&first, &second] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wire.call(export(path));
    }
    let refused = wire.answer_within(export(&first), Duration::from_secs(60));
    assert!(!refused.ok, "the file is there: {:?}", refused.error);

    let files = wire.app.live.write_log.files();
    let listed: Vec<&Path> = files.iter().map(|file| file.path.as_path()).collect();
    // Each export with the report of what it lost beside it (review
    // 2026-11-20 ac P3: the report went unlisted).
    let report = |path: &Path| PathBuf::from(format!("{}.loss.txt", path.display()));
    let (second_report, first_report) = (report(&second), report(&first));
    assert_eq!(
        listed,
        [
            second.as_path(),
            second_report.as_path(),
            first.as_path(),
            first_report.as_path()
        ]
    );
    assert_eq!(
        files[0].bytes,
        Some(std::fs::metadata(&second).unwrap().len())
    );
    assert_eq!(files[0].at.len(), "12:00:00".len());
}

/// A save to the document's own file names no path, and was not listed
/// (review 2026-11-20 ac P3); it is, as the save as before it.
#[test]
fn a_save_in_place_is_listed_as_its_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut wire = Wire::new();
    let path = directory.path().join("house.ketchup");
    wire.call(Request::SaveAs {
        expected: None,
        path: path.display().to_string(),
    });
    wire.call(Request::Save { expected: None });
    let files = wire.app.live.write_log.files();
    let listed: Vec<&Path> = files.iter().map(|file| file.path.as_path()).collect();
    assert_eq!(listed, [path.as_path(), path.as_path()]);
}

/// `apply_and_verify` saving to the document's own file names no path either,
/// and went unlisted as `save` did (review 2026-11-20 ac P3); it is listed.
#[test]
fn an_edit_saved_in_place_is_listed_as_its_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut wire = Wire::connect(grounded_app());
    let path = directory.path().join("house.ketchup");
    wire.call(Request::SaveAs {
        expected: None,
        path: path.display().to_string(),
    });
    let report = wire.call(Request::ApplyAndVerify {
        expected: None,
        selection: Some(vec![]),
        program: program(),
        validators: mandatory_validators(),
        timeout_ms: MAX_APPLY_VERIFY_TIMEOUT_MS,
        strict: false,
        save: Some(ApplyAndVerifySave::Current {}),
    });
    let report = report.result.unwrap();
    assert_eq!(report["saved"], true, "{report}");
    let files = wire.app.live.write_log.files();
    let listed: Vec<&Path> = files.iter().map(|file| file.path.as_path()).collect();
    assert_eq!(listed, [path.as_path(), path.as_path()]);
}
