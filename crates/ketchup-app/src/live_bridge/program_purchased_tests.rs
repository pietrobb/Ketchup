//! A program's bought parts read from STEP files in the program's folder: the
//! window places their exact bodies and stays owned by the program.
use super::*;
use ketchup_model::document::FeatureKind;

const WARDROBE: &str = "door_h = param('door_h', 2000)\n\
    for (i, z) in enumerate([100, door_h - 100]):\n\
    \x20   purchased('hinge %d' % i, 'parts/box.step', '9073662', 'Hettich',\n\
    \x20             at = (18, 0, z), z = (-1, 0, 0), x = (0, 0, 1))\n\
    board('door', (600, 18, door_h))\n";

fn with_exact_worker(mut app: KetchupApp) -> KetchupApp {
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    app.connect_exact_worker(worker).unwrap();
    app
}

/// A program folder holding `parts/box.step`, a small self-authored body.
fn folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("parts")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpora/r0/step/self-authored-box.step"),
        folder.path().join("parts/box.step"),
    )
    .unwrap();
    folder
}

fn apply(source: &str, folder: Option<&Path>, door_h: f64) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: Some(BTreeMap::from([("door_h".to_owned(), door_h)])),
        file_name: Some("wardrobe.star".to_owned()),
        replace_document: true,
        source_folder: folder.map(|folder| folder.display().to_string()),
    }
}

/// The height of each hinge and the SHA-256 of the file its body came from.
fn hinges(app: &KetchupApp) -> BTreeMap<String, (f64, [u8; 32])> {
    let snapshot = app.document.current();
    snapshot
        .occurrences()
        .filter(|occurrence| occurrence.name().starts_with("hinge"))
        .map(|occurrence| {
            let definition = snapshot.definition(occurrence.definition_id()).unwrap();
            let [feature] = definition.feature_ids() else {
                panic!("{}: {:?}", occurrence.name(), definition.feature_ids());
            };
            let FeatureKind::ImportedExactBody(spec) = snapshot.feature(*feature).unwrap().kind()
            else {
                panic!("{} is not the file's body", occurrence.name());
            };
            (
                occurrence.name().to_owned(),
                (occurrence.transform().matrix()[11], spec.source_sha256),
            )
        })
        .collect()
}

#[test]
fn bought_parts_from_the_programs_folder_move_with_its_parameters() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    bridge
        .execute(
            &mut app,
            apply(WARDROBE, Some(folder.path()), 2000.0),
            false,
        )
        .unwrap();
    let placed = hinges(&app);
    assert_eq!(placed.len(), 2, "{placed:?}");
    assert_eq!(placed["hinge 1"].0, 1900.0);
    assert_eq!(placed["hinge 0"].1, placed["hinge 1"].1);
    let snapshot = app.document.current();
    // One file for both hinges: one receipt, its bytes in the document.
    assert_eq!(snapshot.import_receipts().count(), 1);
    assert!(app.document.current_rule_program().is_some());

    // A taller door, applied without the folder again, moves the upper hinge;
    // the document stays the program's.
    bridge
        .execute(&mut app, apply(WARDROBE, None, 2400.0), false)
        .unwrap();
    let moved = hinges(&app);
    assert_eq!(moved["hinge 1"].0, 2300.0);
    assert_eq!(moved["hinge 0"].0, 100.0);
    assert_eq!(app.document.current().import_receipts().count(), 1);
    assert!(app.document.current_rule_program().is_some());
}

/// The program's error for `source` over `folder`, the document unchanged.
fn refused(app: &mut KetchupApp, bridge: &mut LiveBridge, source: &str, folder: &Path) -> String {
    let before = (app.live_bridge_stamp(), app.undo_step_count());
    bridge
        .execute(app, apply(source, Some(folder), 2000.0), false)
        .unwrap_err();
    assert_eq!((app.live_bridge_stamp(), app.undo_step_count()), before);
    take_error_details().unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_part_of_an_assembly_file_is_named_and_a_name_two_parts_share_is_refused() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = tempfile::tempdir().unwrap();
    let assembly = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-partly-invalid-assembly.step"),
    )
    .unwrap();
    std::fs::write(folder.path().join("salvage.step"), &assembly).unwrap();
    let program = |part: &str| {
        format!(
            "door_h = param('door_h', 2000)\n\
             purchased('block', 'salvage.step', '1', 'Test'{part})\n\
             board('door', (600, 18, door_h))\n"
        )
    };

    // Not one valid body as a whole: the error names the parts to choose from.
    let reason = refused(&mut app, &mut bridge, &program(""), folder.path());
    assert!(
        reason.contains("part=") && reason.contains("\"Sound block\""),
        "{reason}"
    );

    bridge
        .execute(
            &mut app,
            apply(
                &program(", part = 'Sound block'"),
                Some(folder.path()),
                2000.0,
            ),
            false,
        )
        .unwrap();
    let snapshot = app.document.current();
    let placed = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "block")
        .unwrap();
    let definition = snapshot.definition(placed.definition_id()).unwrap();
    let FeatureKind::ImportedExactBody(spec) = snapshot
        .feature(definition.feature_ids()[0])
        .unwrap()
        .kind()
    else {
        panic!("not the file's body");
    };
    assert!(spec.source_part_index.is_some());
    assert!(app.document.current_rule_program().is_some());

    // Two parts of one name: the name does not say which one.
    let twins = tempfile::tempdir().unwrap();
    let renamed = assembly.replace("'Block and figure eight'", "'Sound block'");
    assert_ne!(renamed, assembly);
    std::fs::write(twins.path().join("salvage.step"), renamed).unwrap();
    let reason = refused(
        &mut app,
        &mut bridge,
        &program(", part = 'Sound block'"),
        twins.path(),
    );
    assert!(reason.contains("are named \"Sound block\""), "{reason}");
}

#[test]
fn a_broken_step_file_is_refused_with_its_part_and_name() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    std::fs::write(folder.path().join("parts/box.step"), "not a STEP file\n").unwrap();
    let before = (app.live_bridge_stamp(), app.undo_step_count());
    let error = bridge
        .execute(
            &mut app,
            apply(WARDROBE, Some(folder.path()), 2000.0),
            false,
        )
        .unwrap_err();
    assert_eq!(error, "program_rejected");
    let details = take_error_details().unwrap();
    assert_eq!(details["details"]["program_code"], "program_import_failed");
    let reason = details["reason"].as_str().unwrap();
    assert!(
        reason.contains("hinge 0") && reason.contains("parts/box.step"),
        "{details}"
    );
    assert!(
        details["fix_hint"].as_str().unwrap().contains("STEP"),
        "{details}"
    );
    assert_eq!((app.live_bridge_stamp(), app.undo_step_count()), before);
    assert!(app.document.current_rule_program().is_none());

    // A file that is no STEP body at all names no parts.
    assert!(!reason.contains("part="), "{details}");

    // A folder that is not absolute is refused before anything runs.
    let mut request = apply(WARDROBE, None, 2000.0);
    if let Request::ApplyProgram { source_folder, .. } = &mut request {
        *source_folder = Some("relative/folder".to_owned());
    }
    assert!(bridge.execute(&mut app, request, false).is_err());
}
