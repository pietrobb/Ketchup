//! The sandbox of a client's write targets (review 2026-11-08 §5.4): a client
//! writes into the project's folders unasked, anywhere else only after the user
//! agrees in the window, and the folder agreed to is the project's from then on.

use super::*;

/// A closed profile in a DXF without a header, a model a DXF export writes
/// without an exact result.
const PROFILE_DXF: &[u8] = b"0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nclosed\n90\n3\n70\n1\n\
10\n20\n20\n0\n10\n30\n20\n0\n10\n20\n20\n10\n\
0\nENDSEC\n0\nEOF\n";

fn export(path: &Path) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: "dxf".to_owned(),
        drawing_scale: None,
        image: None,
    }
}

fn save_as(app: &KetchupApp, path: &Path) -> Request {
    Request::SaveAs {
        expected: Some(app.live_bridge_stamp()),
        path: path.display().to_string(),
    }
}

/// A window with a profile imported from a DXF whose temporary folder is not
/// the project's, so the test's folders under it are outside the sandbox, saved
/// as `project/house.ketchup` under `directory`.
fn saved_window(
    directory: &Path,
    dialogs: ScriptedFileDialogs,
    settings: Option<&Path>,
) -> (KetchupApp, LiveBridge, PathBuf) {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    if let Some(settings) = settings {
        app = app.with_written_drawings(settings);
    }
    app.window.temporary_folder = None;
    let document = directory.join("project").join("house.ketchup");
    std::fs::create_dir_all(document.parent().unwrap()).unwrap();
    app.remember_recent_file(&document);
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let source = document.with_file_name("profile.dxf");
    std::fs::write(&source, PROFILE_DXF).unwrap();
    let import = Request::Import {
        expected: None,
        path: source.display().to_string(),
        unit: Some("mm".to_owned()),
        as_underlay: false,
        revision_of: None,
        code_page: None,
    };
    bridge.execute(&mut app, import, false).unwrap();
    let request = save_as(&app, &document);
    bridge.execute(&mut app, request, false).unwrap();
    (app, bridge, document)
}

/// Before, a client wrote its exports into any folder it named, a startup
/// folder or the user's documents, unless the file there was another's. Into
/// the document's folder and those under it it writes unasked; into another
/// the window asks first, and the folder agreed to, with its subfolders, is
/// the project's from then on.
#[test]
fn an_export_outside_the_project_asks_first_and_the_folder_agreed_to_is_the_projects() {
    let directory = tempfile::tempdir().unwrap();
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(9);
    let probe = dialogs.clone();
    let (mut app, mut bridge, document) = saved_window(directory.path(), dialogs, None);

    let beside = document.parent().unwrap().join("out").join("house.dxf");
    std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
    bridge
        .execute(&mut app, export(&beside), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert!(beside.exists());
    assert!(
        probe.high_risk_prompts().is_empty(),
        "under the document's folder"
    );

    let elsewhere = directory.path().join("Startup");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let outside = elsewhere.join("house.dxf");
    assert_eq!(
        bridge.execute(&mut app, export(&outside), false),
        Err("write_target_declined")
    );
    assert!(!outside.exists());
    let details = take_error_details().unwrap();
    let canonical = std::fs::canonicalize(&elsewhere).unwrap();
    assert_eq!(details["details"]["folder"], json!(canonical), "{details}");
    assert!(app.write_targets().is_empty());
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert!(
        probe.high_risk_prompts()[0].contains("write into a new folder"),
        "{:?}",
        probe.high_risk_prompts()
    );

    bridge
        .execute(&mut app, export(&outside), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert!(outside.exists());
    assert_eq!(app.write_targets(), std::slice::from_ref(&canonical));
    // The folder agreed to and those under it are the project's now.
    let below = elsewhere.join("sheets").join("house.dxf");
    std::fs::create_dir_all(below.parent().unwrap()).unwrap();
    bridge
        .execute(&mut app, export(&below), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert!(below.exists());
    assert_eq!(probe.high_risk_prompts().len(), 2);

    // A sibling of the folder agreed to, or `..` out of it, is not.
    let sibling = directory.path().join("Startup-2");
    std::fs::create_dir_all(&sibling).unwrap();
    assert_eq!(
        bridge.execute(&mut app, export(&sibling.join("house.dxf")), false),
        Err("write_target_declined")
    );
    let up = elsewhere.join("..").join("Startup-2").join("house.dxf");
    assert_eq!(
        bridge.execute(&mut app, export(&up), false),
        Err("write_target_declined")
    );
    assert!(!sibling.join("house.dxf").exists());
}

/// The folders agreed to are kept with the user's settings for the document's
/// file, so they hold after a restart; a copy saved as another file, or a
/// document someone sends, grants none.
#[test]
fn the_folders_agreed_to_are_the_documents_after_a_restart_and_not_a_copys() {
    let directory = tempfile::tempdir().unwrap();
    let settings = directory.path().join("settings").join("drawings.txt");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval(9);
    let (mut app, mut bridge, document) = saved_window(directory.path(), dialogs, Some(&settings));
    let elsewhere = directory.path().join("exports");
    std::fs::create_dir_all(&elsewhere).unwrap();
    bridge
        .execute(&mut app, export(&elsewhere.join("house.dxf")), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let canonical = std::fs::canonicalize(&elsewhere).unwrap();
    assert_eq!(app.write_targets(), std::slice::from_ref(&canonical));

    let mut reopened = KetchupApp::new()
        .with_dialogs(Box::new(ScriptedFileDialogs::new()))
        .with_written_drawings(&settings);
    assert!(reopened.open_document_path(&document));
    assert_eq!(reopened.write_targets(), [canonical]);

    let copy = document.with_file_name("house-copy.ketchup");
    let request = save_as(&reopened, &copy);
    bridge.execute(&mut reopened, request, false).unwrap();
    assert!(
        reopened.write_targets().is_empty(),
        "a copy starts with none"
    );
}

/// Before, a folder that did not resolve was taken as one still to be made,
/// whatever kept it from resolving: a link in the project's folder whose
/// target is gone stood for a folder of the project, and what was written
/// there went to the target. Only a folder that is not there is still to be
/// made; the link answers its error.
#[cfg(unix)]
#[test]
fn a_link_whose_target_is_gone_is_no_folder_still_to_be_made() {
    use crate::live_bridge::write_targets::folder_of;
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
/// the document under any name in any folder. It saves a `.ketchup` document
/// only, into the project's folders unless the user agrees.
#[test]
fn apply_and_verify_saves_only_a_ketchup_document_into_the_projects_folders() {
    let directory = tempfile::tempdir().unwrap();
    let dialogs = ScriptedFileDialogs::new().queue_refused_high_risk();
    let probe = dialogs.clone();
    let (mut app, _bridge, _document) = saved_window(directory.path(), dialogs, None);
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
    let startup = directory.path().join("Startup");
    std::fs::create_dir_all(&startup).unwrap();
    for refused in [startup.join("run.cmd"), startup.join(".house.ketchup")] {
        assert_eq!(
            crate::live_bridge::write_targets::refusal(&mut app, &saved_at(&refused)),
            Some("invalid_path")
        );
    }
    assert!(probe.high_risk_prompts().is_empty());
    assert_eq!(
        crate::live_bridge::write_targets::refusal(
            &mut app,
            &saved_at(&startup.join("house.ketchup"))
        ),
        Some("save_declined")
    );
    assert_eq!(probe.high_risk_prompts().len(), 1);
}
