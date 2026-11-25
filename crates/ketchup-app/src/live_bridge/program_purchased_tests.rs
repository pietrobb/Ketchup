//! A program's bought parts read from STEP files in the program's folder: the
//! window places their exact bodies and stays owned by the program.
use super::*;
use ketchup_model::document::FeatureKind;

const WARDROBE: &str = "door_h = param('door_h', 2000)\n\
    for (i, z) in enumerate([100, door_h - 100]):\n\
    \x20   purchased('hinge %d' % i, 'parts/box.step', '9237903', 'Hettich',\n\
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

/// Review 2026-11-24 R1124og-STEP-902: `program check` had no folder, so it
/// refused every bought part the document did not place yet. It reads them
/// from the folder it is given, as an apply does, and keeps nothing.
#[test]
fn a_check_reads_new_bought_parts_from_the_programs_folder_and_keeps_nothing() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    let check = |folder: Option<&Path>| Request::CheckProgram {
        expected: None,
        source: Some(WARDROBE.to_owned()),
        edits: None,
        overrides: None,
        file_name: Some("wardrobe.star".to_owned()),
        source_folder: folder.map(|folder| folder.display().to_string()),
    };
    let before = app.live_bridge_stamp();
    let checked = bridge
        .execute(&mut app, check(Some(folder.path())), false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    assert_eq!(checked["change"], "created", "{checked}");
    assert_eq!(checked["check_only"], true, "{checked}");
    let checked_text = checked.to_string();
    assert!(
        !checked_text.contains("import_unread") && checked_text.contains("hinge 1"),
        "{checked}"
    );
    assert_eq!(app.live_bridge_stamp(), before);
    assert!(hinges(&app).is_empty());
    // The check kept no folder: without one, the file is not read.
    bridge
        .execute(&mut app, check(None), false)
        .expect_err("a check without the folder cannot read the file");
    let reason = take_error_details().unwrap()["reason"].to_string();
    assert!(
        reason.contains("apply the program from its file"),
        "{reason}"
    );
}

/// Five hinges of one file are one read of it, and applies after it read it
/// again only when its bytes change: a new time of change alone is not one.
#[test]
fn a_step_file_is_read_once_until_its_bytes_change() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    let five = WARDROBE.replace(
        "enumerate([100, door_h - 100])",
        "enumerate([100 + k * (door_h - 200) / 4 for k in range(5)])",
    );
    let mut apply_at = |app: &mut KetchupApp, door_h: f64| {
        bridge
            .execute(app, apply(&five, Some(folder.path()), door_h), false)
            .unwrap();
        bridge.program_folder.worker_reads()
    };
    assert_eq!(apply_at(&mut app, 2000.0), 1);
    assert_eq!(hinges(&app).len(), 5);
    assert_eq!(apply_at(&mut app, 2400.0), 1);
    assert_eq!(hinges(&app)["hinge 4"].0, 2300.0);
    assert!(app.document.current_rule_program().is_some());

    let file = folder.path().join("parts/box.step");
    let bytes = std::fs::read(&file).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(apply_at(&mut app, 2200.0), 1);

    // Other bytes: read again, the new body placed.
    let mut changed = String::from_utf8(bytes).unwrap();
    changed.push('\n');
    std::fs::write(&file, changed).unwrap();
    assert_eq!(apply_at(&mut app, 2200.0), 2);
    assert_eq!(hinges(&app)["hinge 4"].0, 2100.0);
}

/// R1124og-STEP-903: a file replaced by other bytes of the same size and time
/// of change (`cp -p`, `rsync -t`, an unpacked archive) is read again; the
/// document does not seal the old body for it.
#[test]
fn a_file_of_other_bytes_with_the_same_size_and_time_is_read_again() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    let file = folder.path().join("parts/box.step");
    let when = std::fs::metadata(&file).unwrap().modified().unwrap();
    bridge
        .execute(
            &mut app,
            apply(WARDROBE, Some(folder.path()), 2000.0),
            false,
        )
        .unwrap();
    assert_eq!(bridge.program_folder.worker_reads(), 1);
    let first = hinges(&app)["hinge 0"].1;

    let bytes = std::fs::read_to_string(&file).unwrap();
    let other = bytes.replacen("('Author')", "('Writer')", 1);
    assert_ne!(other, bytes);
    assert_eq!(other.len(), bytes.len());
    std::fs::write(&file, &other).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(when)
        .unwrap();
    bridge
        .execute(
            &mut app,
            apply(WARDROBE, Some(folder.path()), 2200.0),
            false,
        )
        .unwrap();
    assert_eq!(bridge.program_folder.worker_reads(), 2);
    let second = hinges(&app)["hinge 0"].1;
    assert_ne!(second, first, "the old file's body stayed");
    assert_eq!(second, ketchup_model::graph::sha256_bytes(other.as_bytes()));
}

/// R1124og-STEP-901: the folder an apply gave belongs to that document. After
/// another document is opened, a program of the same file name applied
/// without its folder does not read the files of the first one.
#[test]
fn another_document_does_not_read_from_the_folder_of_the_first() {
    let (mut app, mut bridge) = setup();
    let folder = folder();
    bridge
        .execute(
            &mut app,
            apply(
                "door_h = param('door_h', 2000)\nboard('door', (600, 18, door_h))\n",
                Some(folder.path()),
                2000.0,
            ),
            false,
        )
        .unwrap();
    app.new_document();
    bridge.invalidate_document_context();
    assert!(
        bridge
            .execute(&mut app, apply(WARDROBE, None, 2000.0), false)
            .is_err()
    );
    let details = format!("{:#}", take_error_details().unwrap());
    assert!(
        details.contains("apply the program from its file"),
        "{details}"
    );
}

/// R1125og-STEP-1002: an apply that replaces the document without a folder
/// does not hand the folder of the document before to the new one.
#[test]
fn a_replacing_apply_without_a_folder_keeps_no_folder() {
    let (app, _) = setup();
    let snapshot = app.document.current();
    let folder = folder();
    let mut program_folder = super::program_check::step_files::ProgramFolder::default();
    let mut files = |given: Option<&Path>| {
        drop(program_folder.files(
            given.map(Path::to_path_buf),
            "wardrobe.star",
            None,
            Arc::new(AtomicBool::new(false)),
            &snapshot,
        ));
    };
    // An apply from the folder, then one that replaces the document without it.
    files(Some(folder.path()));
    files(None);
    assert_eq!(program_folder.given_by_request(), None);
    // The folder the replacing apply gave goes with the new document.
    let mut program_folder = super::program_check::step_files::ProgramFolder::default();
    drop(program_folder.files(
        Some(folder.path().to_path_buf()),
        "wardrobe.star",
        None,
        Arc::new(AtomicBool::new(false)),
        &snapshot,
    ));
    assert_eq!(
        program_folder.given_by_request(),
        Some(("wardrobe.star".to_owned(), folder.path().to_path_buf()))
    );
}

/// A hall of 22 bought parts, 5 of one assembly part and 17 of another file,
/// reads each file once; applies after it read none while the files stay
/// unchanged, in a window that has not read them yet too: the document holds
/// the bodies, the assembly part's by the name `part=` gave.
#[test]
fn twenty_two_bought_parts_read_each_file_once_and_reapply_reads_none() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-partly-invalid-assembly.step"),
        folder.path().join("parts/salvage.step"),
    )
    .unwrap();
    let hall = "door_h = param('door_h', 2000)\n\
        for i in range(5):\n\
        \x20   purchased('hinge %d' % i, 'parts/salvage.step', '9073605', 'Hettich',\n\
        \x20             part = 'Sound block', at = (18, 0, 100 + i * (door_h - 200) / 4))\n\
        for i in range(17):\n\
        \x20   purchased('runner %d' % i, 'parts/box.step', '9237903', 'Hettich',\n\
        \x20             at = (700 + 40 * i, 0, door_h / 2))\n\
        board('door', (600, 18, door_h))\n";
    let apply_at = |app: &mut KetchupApp, bridge: &mut LiveBridge, door_h: f64| {
        let started = Instant::now();
        bridge
            .execute(app, apply(hall, Some(folder.path()), door_h), false)
            .unwrap_or_else(|code| panic!("{code}: {:#}", take_error_details().unwrap()));
        eprintln!("hall door_h={door_h}: {:?}", started.elapsed());
        bridge.program_folder.worker_reads()
    };
    assert_eq!(apply_at(&mut app, &mut bridge, 2000.0), 2);
    let snapshot = app.document.current();
    let placed = |prefix: &str| {
        snapshot
            .occurrences()
            .filter(|occurrence| occurrence.name().starts_with(prefix))
            .count()
    };
    assert_eq!((placed("hinge"), placed("runner")), (5, 17));
    assert_eq!(snapshot.import_receipts().count(), 2);

    assert_eq!(apply_at(&mut app, &mut bridge, 2400.0), 2);
    let top = |app: &KetchupApp| {
        app.document
            .current()
            .occurrences()
            .find(|occurrence| occurrence.name() == "hinge 4")
            .map(|occurrence| occurrence.transform().matrix()[11])
    };
    assert_eq!(top(&app), Some(2300.0));

    // A window that has read nothing yet, as after opening the saved
    // document: the bodies come from it, neither file is read by the worker.
    bridge.program_folder = Default::default();
    assert_eq!(apply_at(&mut app, &mut bridge, 2200.0), 0);
    assert_eq!(top(&app), Some(2100.0));
    assert!(app.document.current_rule_program().is_some());

    // Another part of the same file is not the one the document holds.
    bridge.program_folder = Default::default();
    let other = hall.replace("'Sound block'", "'Block and figure eight'");
    if bridge
        .execute(&mut app, apply(&other, Some(folder.path()), 2200.0), false)
        .is_err()
    {
        take_error_details();
    }
    assert_eq!(bridge.program_folder.worker_reads(), 1);
}

/// A part named in an assembly file is taken from what the window holds while
/// the file stays the same, and read again once its bytes change, in the same
/// window and in one that has read nothing yet: the document's body of the old
/// file is not the new file's.
#[test]
fn a_changed_assembly_file_is_read_again_for_its_named_part() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = folder();
    let file = folder.path().join("parts/salvage.step");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-partly-invalid-assembly.step"),
        &file,
    )
    .unwrap();
    let source = "door_h = param('door_h', 2000)\n\
        for i in range(5):\n\
        \x20   purchased('hinge %d' % i, 'parts/salvage.step', '9073605', 'Hettich',\n\
        \x20             part = 'Sound block', at = (18, 0, 100 + i * (door_h - 200) / 4))\n\
        board('door', (600, 18, door_h))\n";
    let apply_at = |app: &mut KetchupApp, bridge: &mut LiveBridge, door_h: f64| {
        bridge
            .execute(app, apply(source, Some(folder.path()), door_h), false)
            .unwrap_or_else(|code| panic!("{code}: {:#}", take_error_details().unwrap()));
        bridge.program_folder.worker_reads()
    };
    assert_eq!(apply_at(&mut app, &mut bridge, 2000.0), 1);
    assert_eq!(apply_at(&mut app, &mut bridge, 2400.0), 1);
    let first = hinges(&app)["hinge 4"].1;

    let mut changed = std::fs::read_to_string(&file).unwrap();
    changed.push('\n');
    std::fs::write(&file, &changed).unwrap();
    assert_eq!(apply_at(&mut app, &mut bridge, 2200.0), 2);
    let second = hinges(&app)["hinge 4"];
    assert_eq!(second.0, 2100.0);
    assert_ne!(second.1, first, "the body of the old file stayed");
    assert_eq!(apply_at(&mut app, &mut bridge, 2300.0), 2);

    changed.push('\n');
    std::fs::write(&file, &changed).unwrap();
    bridge.program_folder = Default::default();
    assert_eq!(apply_at(&mut app, &mut bridge, 2000.0), 1);
    let third = hinges(&app)["hinge 4"];
    assert_eq!(third.0, 1900.0);
    assert_ne!(
        third.1, second.1,
        "the document's body of the old file stayed"
    );
}

/// The self-authored box, and a part `part=` names in the self-authored assembly.
const ONE_FILE_PARTS: [(&str, &str); 2] = [
    ("self-authored-box.step", ""),
    (
        "self-authored-partly-invalid-assembly.step",
        ", part = 'Sound block'",
    ),
];

/// The size of `name`'s body as its file gives it.
fn extent(app: &KetchupApp, name: &str) -> [f64; 3] {
    let snapshot = app.document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap();
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    let [feature] = definition.feature_ids() else {
        panic!("{name}: {:?}", definition.feature_ids());
    };
    let FeatureKind::ImportedExactBody(spec) = snapshot.feature(*feature).unwrap().kind() else {
        panic!("{name} is not the file's body");
    };
    let [low, high] = spec.bounds_mm;
    [0, 1, 2].map(|axis| high[axis] - low[axis])
}

/// 22 bought parts of `file` (`part` names one in an assembly): the first apply
/// parses the file once, two re-applies after a parameter change parse it no
/// more and move the parts, and once the file says centimetres instead of
/// millimetres it is parsed again and every part is ten times the size. The
/// two re-apply times, for the release budget.
fn twenty_two_parts_of_one_file(file: &str, part: &str) -> Vec<Duration> {
    let corpora = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpora/r0/step");
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = tempfile::tempdir().unwrap();
    let copy = folder.path().join("part.step");
    std::fs::copy(corpora.join(file), &copy).unwrap();
    let source = format!(
        "door_h = param('door_h', 2000)\n\
         for i in range(22):\n\
         \x20   purchased('hinge %d' % i, 'part.step', '9073605', 'Hettich'{part},\n\
         \x20             at = (700 + 40 * i, 0, door_h / 2))\n\
         board('door', (600, 18, door_h))\n"
    );
    let mut apply_at = |app: &mut KetchupApp, door_h: f64| {
        let reads = bridge.program_folder.worker_reads();
        let started = Instant::now();
        bridge
            .execute(app, apply(&source, Some(folder.path()), door_h), false)
            .unwrap_or_else(|code| panic!("{code}: {:#}", take_error_details().unwrap()));
        let elapsed = started.elapsed();
        eprintln!("{file} door_h={door_h}: {elapsed:?}");
        (bridge.program_folder.worker_reads() - reads, elapsed)
    };
    assert_eq!(apply_at(&mut app, 2000.0).0, 1, "{file}");
    assert_eq!(hinges(&app).len(), 22, "{file}");
    let size = extent(&app, "hinge 21");
    let mut times = Vec::new();
    for door_h in [2400.0, 2100.0] {
        let (parsed, elapsed) = apply_at(&mut app, door_h);
        assert_eq!(parsed, 0, "{file} parsed again at door_h={door_h}");
        assert_eq!(hinges(&app)["hinge 21"].0, door_h / 2.0, "{file}");
        times.push(elapsed);
    }

    let text = std::fs::read_to_string(&copy).unwrap();
    let centimetres = text.replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.CENTI.,.METRE.)");
    assert_ne!(centimetres, text, "{file} names no millimetres");
    std::fs::write(&copy, centimetres).unwrap();
    assert_eq!(apply_at(&mut app, 2200.0).0, 1, "{file}");
    assert_eq!(hinges(&app).len(), 22, "{file}");
    for name in ["hinge 0", "hinge 21"] {
        let grown = extent(&app, name);
        for axis in 0..3 {
            assert!(
                (grown[axis] - 10.0 * size[axis]).abs() < 1e-6 * grown[axis].max(1.0),
                "{file} {name}: {grown:?}, before {size:?}"
            );
        }
    }
    times
}

#[test]
fn twenty_two_parts_of_one_file_reapply_without_parsing_and_reread_a_changed_file() {
    for (file, part) in ONE_FILE_PARTS {
        twenty_two_parts_of_one_file(file, part);
    }
}

#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_twenty_two_parts_of_one_file_reapply_within_a_second() {
    let budget = ketchup_test_env::perf_budget(Duration::from_secs(1));
    for (file, part) in ONE_FILE_PARTS {
        for elapsed in twenty_two_parts_of_one_file(file, part) {
            assert!(elapsed < budget, "{file}: {elapsed:?}, budget {budget:?}");
        }
    }
}

/// The pairs of parts the apply answer reports as colliding, sorted.
fn collisions(applied: &Value) -> Vec<Vec<String>> {
    let mut pairs = applied["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue["kind"] == "collision")
        .map(|issue| {
            let mut pair = issue["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            pair.sort();
            pair
        })
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
}

/// A turned fitting and a bent pipe are measured only against the boards their
/// boxes reach: those they cut into collide, the boards 1 mm clear of the pipe
/// and touching it are measured and clear, and the board far off is never
/// measured.
#[test]
fn turned_and_swept_parts_are_measured_only_against_the_parts_near_them() {
    let mut app = with_exact_worker(KetchupApp::new());
    app.selection.clear();
    let mut wire = Wire::connect(app);
    let source = "\
door_h = param('door_h', 2000)\n\
revolve('fitting', profile = [[55, 0], [63, 0], [63, 40], [55, 40]], axis = [[0, 0], [0, 1]],\n\
        at = (1000, 0, 1000))\n\
sweep('pipe', profile = round_thread(55), path = [(0, 500, 0), (0, 500, 300), (0, 800, 300)],\n\
      bend = 150)\n\
board('wall', (200, 200, 200), at = (900, -100, 900))\n\
board('floor', (200, 200, 100), at = (-100, 400, -50))\n\
board('near', (20, 100, 100), at = (56, 450, 100))\n\
board('touch', (20, 60, 30), at = (55, 470, 60))\n\
board('far', (100, 18, 100), at = (5000, 5000, 5000))\n";
    let applied = wire
        .call_within(apply(source, None, 2000.0), Duration::from_secs(120))
        .result
        .unwrap();
    assert_eq!(
        collisions(&applied),
        [
            vec!["fitting".to_owned(), "wall".to_owned()],
            vec!["floor".to_owned(), "pipe".to_owned()],
        ],
        "{applied}"
    );
    // Unbounded, the fitting and the pipe were each measured against all six
    // other parts.
    assert_eq!(
        applied["exact_collisions"]["state"], "verified",
        "{applied}"
    );
    assert_eq!(
        applied["exact_collisions"]["exact_pair_count"], 4,
        "{applied}"
    );
}

/// A bought part that overlaps a board is a collision of the two, and one that
/// clears it is none; the exact body goes out to STEP and GLB with the boards.
#[test]
fn bought_parts_collide_with_boards_and_export_with_them() {
    let mut app = with_exact_worker(KetchupApp::new());
    app.selection.clear();
    let mut wire = Wire::connect(app);
    let folder = folder();
    let mut applied = |source: &str| {
        wire.call_within(
            apply(source, Some(folder.path()), 2000.0),
            Duration::from_secs(120),
        )
        .result
        .unwrap()
    };
    // The box is 10 x 20 x 30 mm, its 30 mm along -x from x = 18: 18 x 18 x
    // 10 mm of it lie in the door.
    let first = applied(WARDROBE);
    assert_eq!(first["exact_collisions"]["collisions"], 2, "{first}");
    assert_eq!(
        collisions(&first),
        [
            vec!["door".to_owned(), "hinge 0".to_owned()],
            vec!["door".to_owned(), "hinge 1".to_owned()],
        ],
        "{first}"
    );

    // In front of the door, 12 mm off its face: clear.
    let clear = applied(&WARDROBE.replace("at = (18, 0, z)", "at = (18, 30, z)"));
    assert_eq!(collisions(&clear), Vec::<Vec<String>>::new(), "{clear}");

    // A board far off is never measured against the bought parts: the boxes
    // let only the two hinges against the door reach the exact check.
    let far = applied(&format!(
        "{WARDROBE}board('far', (100, 18, 100), at = (5000, 5000, 5000))\n"
    ));
    assert_eq!(collisions(&far), collisions(&first), "{far}");
    assert_eq!(far["exact_collisions"]["exact_pair_count"], 2, "{far}");
    // Hinges set against the back of the door touch it: their boxes meet the
    // door's, so both pairs are still measured, and touching is no collision.
    let touching = applied(&WARDROBE.replace("at = (18, 0, z)", "at = (18, 18, z)"));
    assert_eq!(
        collisions(&touching),
        Vec::<Vec<String>>::new(),
        "{touching}"
    );
    assert_eq!(
        touching["exact_collisions"]["exact_pair_count"], 2,
        "{touching}"
    );

    // The Viewer package needs a laid-out 3D view and a scene to share.
    wire.app.camera.viewport_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800.0, 600.0),
    ));
    wire.call(Request::SaveView {
        expected: None,
        name: "Front".into(),
        viewer: None,
    });
    // The window makes every body exact, the bought ones from their stored file.
    crate::drawn_shape::tests::evaluate_exact(&mut wire.app);
    let out = tempfile::tempdir().unwrap();
    for (format, extension) in [("step", "step"), ("glb", "glb"), ("viewer", "ketchup-view")] {
        let target = out.path().join(format!("wardrobe.{extension}"));
        let exported = wire.answer_within(
            Request::Export(crate::live_bridge::interchange::ExportRequest {
                expected: None,
                path: target.display().to_string(),
                export_format: format.to_owned(),
                drawing_scale: None,
                image: None,
                cam_dialect: None,
                cam_operations: None,
                overwrite: false,
            }),
            Duration::from_secs(120),
        );
        assert!(exported.ok, "{format}: {exported:?}");
        let bytes = std::fs::read(&target).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for part in ["hinge 0", "hinge 1", "door"] {
            assert!(text.contains(part), "{format} has no {part}");
        }
    }
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
    // The file read as one body and as its parts, once each: the part named
    // and a later apply of it take the parts read for the error.
    assert_eq!(bridge.program_folder.worker_reads(), 2);
    bridge
        .execute(
            &mut app,
            apply(
                &program(", part = 'Sound block'"),
                Some(folder.path()),
                2400.0,
            ),
            false,
        )
        .unwrap();
    assert_eq!(bridge.program_folder.worker_reads(), 2);

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

/// A supplier's assembly often names every part's product after its shape
/// ("SOLID") and only the placement after the article: `part=` takes the
/// placement's name too, one holding several parts is refused as not saying
/// which, and a name the file lacks lists the parts it refused and why.
#[test]
fn a_part_is_named_by_its_placement_in_the_assembly() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = tempfile::tempdir().unwrap();
    let assembly = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-partly-invalid-assembly.step"),
    )
    .unwrap();
    let placed = assembly
        .replace("'Sound block','Sound block'", "'SOLID','SOLID'")
        .replace("'1','=>[0:1:1:2]'", "'1','Hettich 1 hinge'");
    assert_ne!(placed, assembly);
    std::fs::write(folder.path().join("salvage.step"), placed).unwrap();
    let program = |part: &str| {
        format!(
            "door_h = param('door_h', 2000)\n\
             purchased('block', 'salvage.step', '1', 'Test', part = '{part}')\n\
             board('door', (600, 18, door_h))\n"
        )
    };
    bridge
        .execute(
            &mut app,
            apply(&program("Hettich 1 hinge"), Some(folder.path()), 2000.0),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {:#}", take_error_details().unwrap()));
    assert!(
        app.document
            .current()
            .occurrences()
            .any(|occurrence| occurrence.name() == "block")
    );

    let reason = refused(
        &mut app,
        &mut bridge,
        &program("Salvage assembly"),
        folder.path(),
    );
    assert!(
        reason.contains("holds") && reason.contains("parts"),
        "{reason}"
    );

    let reason = refused(&mut app, &mut bridge, &program("Wire group"), folder.path());
    assert!(
        reason.contains("no part \"Wire group\"")
            && reason.contains("refused \"Crossed prism\": ")
            && reason.contains("encloses no volume")
            && reason.contains("\"Wire frame\": ")
            && reason.contains("has no face"),
        "{reason}"
    );
}

/// `name`'s occurrence transform and its body's bounds in its own frame.
fn placed_at(app: &KetchupApp, name: &str) -> ([f64; 16], [[f64; 3]; 2]) {
    let snapshot = app.document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap_or_else(|| panic!("{name} not placed"));
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    let FeatureKind::ImportedExactBody(spec) = snapshot
        .feature(definition.feature_ids()[0])
        .unwrap()
        .kind()
    else {
        panic!("{name} is not the file's body");
    };
    (*occurrence.transform().matrix(), spec.bounds_mm)
}

/// The world bounds of `bounds` placed by the row-major `matrix`.
fn world_bounds(matrix: &[f64; 16], [low, high]: [[f64; 3]; 2]) -> Vec<f64> {
    let mut placed = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for corner in 0..8 {
        let point: [f64; 3] = std::array::from_fn(|axis| {
            if corner & (1 << axis) == 0 {
                low[axis]
            } else {
                high[axis]
            }
        });
        for axis in 0..3 {
            let value = (0..3)
                .map(|column| matrix[axis * 4 + column] * point[column])
                .sum::<f64>()
                + matrix[axis * 4 + 3];
            placed[0][axis] = placed[0][axis].min(value);
            placed[1][axis] = placed[1][axis].max(value);
        }
    }
    placed.concat()
}

fn assert_near(actual: &[f64], expected: &[f64], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() < 1e-9,
            "{what}: {actual:?} is not {expected:?}"
        );
    }
}

/// A part named in an assembly sits where the assembly places it, every
/// placement from its root down to it: a bracket under a subassembly moved
/// to (-913, 0, -77.5) and one 40 mm along in it, a pin turned half a turn
/// about x (diag(1, -1, -1)). It stays there in a window that has not read
/// the file and with no folder at all; a part placed twice is named by one
/// of its placements.
#[test]
fn a_named_assembly_part_sits_where_its_assembly_places_it() {
    let (app, mut bridge) = setup();
    let mut app = with_exact_worker(app);
    let folder = tempfile::tempdir().unwrap();
    let original = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step"),
    )
    .unwrap();
    let (carriage, pin_axis) = (
        "#16 = CARTESIAN_POINT('',(0.,50.,0.));",
        "#21 = DIRECTION('',(0.,0.,1.));",
    );
    assert!(original.contains(carriage) && original.contains(pin_axis));
    let fixture = original
        .replace(carriage, "#16 = CARTESIAN_POINT('',(-913.,0.,-77.5));")
        .replace(pin_axis, "#21 = DIRECTION('',(0.,0.,-1.));");
    std::fs::write(folder.path().join("fixture.step"), fixture).unwrap();
    let program = "door_h = param('door_h', 2000)\n\
        purchased('left', 'fixture.step', '1', 'Test', part = 'Bracket left instance')\n\
        purchased('right', 'fixture.step', '1', 'Test', part = 'Bracket right instance',\n\
        \x20         at = (0, 0, door_h))\n\
        purchased('pin', 'fixture.step', '1', 'Test', part = 'Pin definition')\n\
        board('door', (600, 18, door_h))\n";
    let check = |app: &KetchupApp, door_h: f64| {
        let moved = |x: f64, y: f64, z: f64| {
            [
                1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0,
            ]
        };
        for (name, expected, offset) in [
            ("left", moved(-913.0, 0.0, -77.5), [-913.0, 0.0, -77.5]),
            (
                "right",
                moved(-873.0, 0.0, door_h - 77.5),
                [-873.0, 0.0, door_h - 77.5],
            ),
        ] {
            let (matrix, [low, high]) = placed_at(app, name);
            assert_near(&matrix, &expected, name);
            let shifted = |corner: [f64; 3]| (0..3).map(move |axis| corner[axis] + offset[axis]);
            let expected: Vec<f64> = shifted(low).chain(shifted(high)).collect();
            assert_near(&world_bounds(&matrix, [low, high]), &expected, name);
        }
        // Half a turn about x at (20, 10, 5): x stays, y and z turn over.
        let (matrix, [low, high]) = placed_at(app, "pin");
        assert_near(
            &matrix,
            &[
                1.0, 0.0, 0.0, 20.0, 0.0, -1.0, 0.0, 10.0, 0.0, 0.0, -1.0, 5.0, 0.0, 0.0, 0.0, 1.0,
            ],
            "pin",
        );
        assert_near(
            &world_bounds(&matrix, [low, high]),
            &[
                low[0] + 20.0,
                10.0 - high[1],
                5.0 - high[2],
                high[0] + 20.0,
                10.0 - low[1],
                5.0 - low[2],
            ],
            "pin",
        );
    };
    let apply_at =
        |app: &mut KetchupApp, bridge: &mut LiveBridge, folder: Option<&Path>, door_h| {
            bridge
                .execute(app, apply(program, folder, door_h), false)
                .unwrap_or_else(|code| panic!("{code}: {:#}", take_error_details().unwrap()));
            check(app, door_h);
        };
    apply_at(&mut app, &mut bridge, Some(folder.path()), 2000.0);
    assert_eq!(bridge.program_folder.worker_reads(), 1);
    // Three parts of one file: its bytes hashed once (R1125og-STEP-1003).
    assert_eq!(bridge.program_folder.hashes(), 1);

    // A window that has read nothing yet takes the placements the document
    // holds, reading nothing; a host with no folder keeps them too.
    bridge.program_folder = Default::default();
    apply_at(&mut app, &mut bridge, Some(folder.path()), 2400.0);
    assert_eq!(bridge.program_folder.worker_reads(), 0);
    assert_eq!(bridge.program_folder.hashes(), 1);
    apply_at(&mut app, &mut bridge, None, 2200.0);

    // One bracket placed twice: its own name, or a subassembly holding both,
    // does not say which placement.
    for part in ["Bracket definition", "Carriage nested instance"] {
        let source = program.replace("'Pin definition'", &format!("'{part}'"));
        let reason = refused(&mut app, &mut bridge, &source, folder.path());
        assert!(
            reason.contains("2 times") && reason.contains("placement"),
            "{part}: {reason}"
        );
    }
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
