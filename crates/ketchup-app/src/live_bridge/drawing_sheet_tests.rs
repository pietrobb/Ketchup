//! `file action=export_drawings` with `sheet_id`: a drawing sheet of the
//! document is written as the DXF of its exact projection, the project
//! drawings as a DXF per sheet, and no export changes the document. Exact
//! circles the views face are CIRCLE and ARC entities, not their lines. A
//! `.dwg` path gets that DXF as an AutoCAD 2018 DWG.

use super::drill_holes::{centroid, edit, face, saved};
use super::program_edit::evaluate_exact;
use super::*;
use ketchup_model::drawing::{DrawingSheetId, project_orthographic_drawing};
use ketchup_model::drawing_export::export_drawing;
use std::collections::BTreeMap;

fn export_drawings(path: &Path, extra: Value) -> Request {
    let mut request = json!({"method": "export_drawings", "path": path});
    request
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(request).unwrap()
}

fn summary(app: &mut KetchupApp, bridge: &mut LiveBridge) -> Value {
    let request = serde_json::from_value(json!({"method": "summary"})).unwrap();
    bridge.execute(app, request, false).unwrap()
}

/// The first part of the new document drawn on a sheet by MCP `create_drawing`,
/// found through the summary.
fn sheet_of_the_block(app: &mut KetchupApp, bridge: &mut LiveBridge) -> u64 {
    sheet_of_parts(app, bridge, 1)
}

/// The first `parts` parts of the new document drawn together on one sheet:
/// with more than one, the sheet draws them where they lie in the plan.
pub(super) fn sheet_of_parts(app: &mut KetchupApp, bridge: &mut LiveBridge, parts: usize) -> u64 {
    evaluate_exact(app);
    let instance_paths = app
        .document
        .current()
        .occurrences()
        .take(parts)
        .map(|occurrence| AssistantInstancePath {
            root_occurrence_id: occurrence.id().0,
            steps: Vec::new(),
        })
        .collect::<Vec<_>>();
    assert_eq!(instance_paths.len(), parts);
    if parts > 1 {
        // Several parts are drawn as one rigid assembly: grounded, they stay put.
        let occurrence_ids = instance_paths
            .iter()
            .map(|path| path.root_occurrence_id)
            .collect();
        let grounded = edit(AssistantCadEditOperation::SetGrounded {
            selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
            grounded: true,
        });
        bridge
            .execute(app, grounded, false)
            .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
        evaluate_exact(app);
    }
    let name = "Block".to_owned();
    let created = edit(AssistantCadEditOperation::CreateDrawing {
        name,
        instance_paths,
    });
    let created = bridge
        .execute(app, created, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    assert_eq!(created["published"], true, "{created}");
    evaluate_exact(app);
    let listed = summary(app, bridge);
    assert_eq!(listed["counts"]["drawing_sheets"], 1, "{listed}");
    assert_eq!(listed["drawing_sheets_complete"], true);
    let [sheet] = listed["drawing_sheets"].as_array().unwrap().as_slice() else {
        panic!("one drawing sheet: {listed}");
    };
    assert_eq!(sheet["name"]["text"], "Block");
    sheet["id"].as_u64().unwrap()
}

fn layer_lines(dxf: &str, layer: &str) -> usize {
    dxf.matches(&format!("0\nLINE\n8\n{layer}\n")).count()
}

#[test]
fn a_drawing_sheet_is_written_as_the_dxf_of_its_exact_projection() {
    let (mut app, mut bridge) = setup();
    assert_eq!(summary(&mut app, &mut bridge)["drawing_sheets"], json!([]));
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("block.dxf");

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");
    assert_eq!(exported["sheet_id"], id);
    assert_eq!(
        summary(&mut app, &mut bridge)["drawing_sheets"][0]["name"]["text"],
        exported["sheet"]
    );
    assert_eq!(exported["basis"], "drawing_sheet_exact_projection");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let expected = export_drawing(&snapshot, &drawing).unwrap();
    assert_eq!(
        written.as_bytes(),
        expected.dxf(),
        "the exact export, byte for byte"
    );
    assert_eq!(exported["bytes"], written.len());
    assert_eq!(
        exported["views"],
        json!(
            drawing
                .views
                .iter()
                .map(|view| view.kind.stable_name())
                .collect::<Vec<_>>()
        )
    );
    let visible = drawing
        .views
        .iter()
        .map(|view| view.visible_lines.len())
        .sum::<usize>();
    let hidden = drawing
        .views
        .iter()
        .map(|view| view.hidden_lines.len())
        .sum::<usize>();
    assert!(visible >= 12, "three views of a block: {visible}");
    assert_eq!(exported["visible_lines"], visible);
    assert_eq!(exported["hidden_lines"], hidden);
    assert_eq!(layer_lines(&written, "VISIBLE"), visible);
    assert_eq!(layer_lines(&written, "HIDDEN"), hidden);
    assert!(written.ends_with("0\nENDSEC\n0\nEOF\n"));

    assert_eq!(app.live_bridge_stamp(), stamp, "an export is no edit");
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(saved(&app), bytes);
}

#[test]
fn wrong_sheet_requests_are_refused_by_name() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);

    for (path, extra, code) in [
        (file("house.txt"), json!({}), "invalid_path"),
        (
            file("missing.dxf"),
            json!({"sheet_id": id + 1}),
            "drawing_sheet_not_found",
        ),
        (file("sheet.pdf"), json!({"sheet_id": id}), "invalid_path"),
        (file("sheet.txt"), json!({"sheet_id": id}), "invalid_path"),
        (
            file("sheet.dxf"),
            json!({"sheet_id": id, "format": "A1"}),
            "invalid_params",
        ),
        (
            file("sheet.dxf"),
            json!({"sheet_id": id, "title_block": {"author": "AI"}}),
            "invalid_params",
        ),
    ] {
        let refused = bridge.execute(&mut app, export_drawings(&path, extra.clone()), false);
        assert_eq!(refused, Err(code), "{} {extra}", path.display());
        assert!(!path.exists(), "{} was written", path.display());
    }
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(saved(&app), bytes);
}

/// The project drawings as DXF are a file per sheet beside the path,
/// numbered, each with the title block saying which sheet of how many it is;
/// as DWG the same sheets in AutoCAD 2018. A file named like a sheet that no
/// earlier set of this project wrote refuses the set and leaves nothing of it
/// behind.
#[test]
fn the_project_drawings_are_written_as_a_dxf_or_dwg_per_sheet() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let (stamp, undo) = (app.live_bridge_stamp(), app.undo_step_count());
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&file("house.dxf"), json!({})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["file_format"], "dxf", "{exported}");
    // A block is one storey: its plan, the sections and the elevations.
    assert_eq!(exported["sheet_count"], 3, "{exported}");
    for (index, shows) in ["plan", "sections", "elevations"].into_iter().enumerate() {
        let number = index + 1;
        let path = file(&format!("house-{number:02}.dxf"));
        let sheet = &exported["sheets"][index];
        assert_eq!(sheet["number"], number);
        assert_eq!(sheet["shows"]["kind"], shows, "{sheet}");
        assert_eq!(sheet["path"], path.display().to_string());
        assert_eq!(exported["files"][index], path.display().to_string());
        let dxf = std::fs::read_to_string(&path).unwrap();
        assert!(dxf.contains("$ACADVER\n1\nAC1009"));
        assert!(
            dxf.contains(&format!("\n1\n{number} / 3\n")),
            "sheet {number}"
        );
        assert!(
            dxf.contains("0\nLINE\n8\nPEN-"),
            "sheet {number} draws lines"
        );
    }
    assert!(!file("house.dxf").exists());

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&file("house.dwg"), json!({})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["sheet_count"], 3, "{exported}");
    for number in 1..=3 {
        let dwg = std::fs::read(file(&format!("house-{number:02}.dwg"))).unwrap();
        assert!(dwg.starts_with(b"AC1032"), "sheet {number}");
    }

    std::fs::write(file("taken-02.dxf"), b"user file").unwrap();
    assert_eq!(
        bridge.execute(
            &mut app,
            export_drawings(&file("taken.dxf"), json!({})),
            false
        ),
        Err("drawings_target_exists")
    );
    assert!(
        !file("taken-01.dxf").exists(),
        "the set is written whole or not at all"
    );
    assert!(!file("taken-03.dxf").exists());
    assert_eq!(std::fs::read(file("taken-02.dxf")).unwrap(), b"user file");
    assert_eq!(app.live_bridge_stamp(), stamp, "an export is no edit");
    assert_eq!(app.undo_step_count(), undo);
}

/// The names of the files in `directory`, sorted.
fn names_in(directory: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// MECH-2 (Měchenice 2026-11-03): the drawings exported again after an edit
/// replace their files at the same path, as a PDF, a DXF set and a drawing
/// sheet, and MECH-3: nothing is left beside them, no lock, no temporary
/// file, no backup.
#[test]
fn the_drawings_exported_again_replace_their_files() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);
    for _ in 0..2 {
        for (path, extra) in [
            (file("house.pdf"), json!({})),
            (file("house.dxf"), json!({})),
            (file("block.dwg"), json!({"sheet_id": id})),
        ] {
            let exported = bridge
                .execute(&mut app, export_drawings(&path, extra.clone()), false)
                .unwrap_or_else(|code| panic!("{code} {extra}: {:?}", take_error_details()));
            assert_eq!(exported["exported"], true, "{exported}");
        }
    }
    assert!(
        std::fs::read(file("house.pdf"))
            .unwrap()
            .starts_with(b"%PDF-")
    );
    assert!(
        std::fs::read(file("block.dwg"))
            .unwrap()
            .starts_with(b"AC1032")
    );
    assert_eq!(
        names_in(directory.path()),
        [
            "block.dwg",
            "house-01.dxf",
            "house-02.dxf",
            "house-03.dxf",
            "house.dxf.sheets.txt",
            "house.pdf",
        ]
    );
}

/// Review 2026-11-04 MCP-140: a PDF or a drawing sheet's file at the path was
/// replaced whatever it was (a PDF the user added pages to, a surveyor's DWG).
/// Only the file the window last wrote there is replaced, or one the client
/// asks to replace with `replace: true`; the answer names the file and says so.
#[test]
fn a_pdf_or_sheet_the_window_did_not_write_is_replaced_only_when_asked() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);
    for (name, extra) in [
        ("house.pdf", json!({})),
        ("survey.dwg", json!({"sheet_id": id})),
    ] {
        let path = file(name);
        let mut export =
            |extra: &Value| bridge.execute(&mut app, export_drawings(&path, extra.clone()), false);
        std::fs::write(&path, b"the user's own").unwrap();
        assert_eq!(export(&extra), Err("drawings_target_exists"), "{name}");
        let answer = take_error_details().unwrap_or_default().to_string();
        for said in [name, "replace: true"] {
            assert!(answer.contains(said), "{said:?} missing: {answer}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"the user's own");

        let mut asked = extra.clone();
        asked["replace"] = json!(true);
        export(&asked).unwrap_or_else(|code| panic!("{code} {name}: {:?}", take_error_details()));
        assert_ne!(std::fs::read(&path).unwrap(), b"the user's own");
        // Its own file is replaced again without asking, until it changes.
        export(&extra).unwrap_or_else(|code| panic!("{code} {name}: {:?}", take_error_details()));
        std::fs::write(&path, b"edited since").unwrap();
        assert_eq!(export(&extra), Err("drawings_target_exists"), "{name}");
        assert_eq!(std::fs::read(&path).unwrap(), b"edited since");
    }
}

/// Review 2026-11-04 r19 SEC-161: which file the window wrote lived only in
/// memory, keyed by the path as typed, so after a restart, or with the path
/// spelled otherwise (`c:\X` for `C:\x`), the usual export after an edit was
/// refused with drawings_target_exists. The SHA-256 of each file the document
/// wrote is kept by its canonical path with the user's settings (review
/// 2026-11-06 SEC-182), and the refusal says to ask the user.
#[test]
fn the_documents_own_drawing_is_replaced_after_a_restart_and_however_its_path_is_spelled() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("drawings.txt");
    let (app, mut bridge) = setup();
    let mut app = app.with_written_drawings(&store);
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let targets = [
        (directory.path().join("House.pdf"), json!({})),
        (directory.path().join("Survey.dwg"), json!({"sheet_id": id})),
    ];
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge, path: &Path, extra: &Value| {
        bridge
            .execute(app, export_drawings(path, extra.clone()), false)
            .unwrap_or_else(|code| panic!("{code} {path:?}: {:?}", take_error_details()))
    };
    for (path, extra) in &targets {
        export(&mut app, &mut bridge, path, extra);
        if cfg!(windows) {
            // Windows names ignore case: the same file, spelled otherwise.
            let other = PathBuf::from(path.display().to_string().to_lowercase());
            export(&mut app, &mut bridge, &other, extra);
        }
    }
    std::fs::write(&targets[0].0, b"the user's own").unwrap();
    assert_eq!(
        bridge.execute(&mut app, export_drawings(&targets[0].0, json!({})), false),
        Err("drawings_target_exists")
    );
    let answer = take_error_details().unwrap_or_default().to_string();
    assert!(answer.contains("Ask the user"), "{answer}");
    export(
        &mut app,
        &mut bridge,
        &targets[0].0,
        &json!({"replace": true}),
    );

    let document = directory.path().join("plan.ketchup");
    let save = Request::SaveAs {
        expected: None,
        path: document.display().to_string(),
    };
    bridge.execute(&mut app, save, false).unwrap();
    drop((app, bridge));
    let (opened, mut bridge) = setup();
    let mut opened = opened.with_written_drawings(&store);
    let open = Request::Open {
        expected: None,
        path: document.display().to_string(),
    };
    bridge.execute(&mut opened, open, false).unwrap();
    evaluate_exact(&mut opened);
    for (path, extra) in &targets {
        export(&mut opened, &mut bridge, path, extra);
    }
}

/// Review 2026-11-06 SEC-182: the export noted its file in the document and so
/// made it unsaved; once the user did not save it, the next window refused the
/// usual export again. An export leaves a saved document saved, and the
/// document's own file is replaced after a restart without its being saved.
#[test]
fn an_export_leaves_the_document_saved_and_its_file_its_own_after_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("user").join("drawings.txt");
    let with_store = || {
        let (app, bridge) = setup();
        (app.with_written_drawings(&store), bridge)
    };
    let (mut app, mut bridge) = with_store();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let document = directory.path().join("plan.ketchup");
    let save = Request::SaveAs {
        expected: None,
        path: document.display().to_string(),
    };
    bridge.execute(&mut app, save, false).unwrap();
    let sheet = directory.path().join("Survey.dwg");
    let answer = bridge
        .execute(
            &mut app,
            export_drawings(&sheet, json!({"sheet_id": id})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    assert_eq!(answer["dirty"], false, "{answer}");
    assert!(!app.is_dirty());
    drop((app, bridge));

    let (mut opened, mut bridge) = with_store();
    let open = Request::Open {
        expected: None,
        path: document.display().to_string(),
    };
    bridge.execute(&mut opened, open, false).unwrap();
    evaluate_exact(&mut opened);
    bridge
        .execute(
            &mut opened,
            export_drawings(&sheet, json!({"sheet_id": id})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
}

/// Review 2026-11-06 SEC-182: Save As carried the list of the files the
/// document wrote, so the copy replaced the first document's drawing unasked.
/// A document saved as another file asks before replacing what the first wrote.
#[test]
fn a_document_saved_as_another_asks_before_replacing_the_first_ones_drawing() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let save_as = |app: &mut KetchupApp, bridge: &mut LiveBridge, name: &str| {
        let path = directory.path().join(name).display().to_string();
        let save = Request::SaveAs {
            expected: None,
            path,
        };
        bridge.execute(app, save, false).unwrap();
    };
    save_as(&mut app, &mut bridge, "a.ketchup");
    let sheet = directory.path().join("A.dxf");
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge| {
        bridge.execute(app, export_drawings(&sheet, json!({"sheet_id": id})), false)
    };
    export(&mut app, &mut bridge).unwrap();
    export(&mut app, &mut bridge).expect("its own file is replaced");
    save_as(&mut app, &mut bridge, "b.ketchup");
    assert_eq!(export(&mut app, &mut bridge), Err("drawings_target_exists"));
    let answer = take_error_details().unwrap_or_default().to_string();
    assert!(answer.contains("Ask the user"), "{answer}");
}

/// Review 2026-11-06 SEC-182 (capability review run 529): a moved document
/// kept its drawings by its id, which Save As carries to the copy, so once
/// the first was renamed the copy replaced its drawing unasked. The copy
/// still asks; the first, renamed, still replaces its own.
#[test]
fn a_copy_saved_as_asks_once_the_first_document_is_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("user").join("drawings.txt");
    let with_store = || {
        let (app, bridge) = setup();
        (app.with_written_drawings(&store), bridge)
    };
    let (mut app, mut bridge) = with_store();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let save_as = |app: &mut KetchupApp, bridge: &mut LiveBridge, path: &Path| {
        let save = Request::SaveAs {
            expected: None,
            path: path.display().to_string(),
        };
        bridge.execute(app, save, false).unwrap();
    };
    let sheet = directory.path().join("A.dxf");
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge| {
        bridge.execute(app, export_drawings(&sheet, json!({"sheet_id": id})), false)
    };
    let first = directory.path().join("a.ketchup");
    save_as(&mut app, &mut bridge, &first);
    export(&mut app, &mut bridge).unwrap();
    save_as(&mut app, &mut bridge, &directory.path().join("b.ketchup"));
    let renamed = directory.path().join("a2.ketchup");
    std::fs::rename(&first, &renamed).unwrap();
    assert_eq!(export(&mut app, &mut bridge), Err("drawings_target_exists"));
    let answer = take_error_details().unwrap_or_default().to_string();
    assert!(answer.contains("Ask the user"), "{answer}");
    drop((app, bridge));

    let (mut opened, mut bridge) = with_store();
    let open = Request::Open {
        expected: None,
        path: renamed.display().to_string(),
    };
    bridge.execute(&mut opened, open, false).unwrap();
    evaluate_exact(&mut opened);
    export(&mut opened, &mut bridge).unwrap_or_else(|code| {
        panic!(
            "the renamed first's own: {code}: {:?}",
            take_error_details()
        )
    });
}

/// Review 2026-11-06 SEC-182: the list of the files written only grew. A file
/// that is gone leaves it at the next export, and a file written again is
/// listed once.
#[test]
fn the_list_of_written_drawings_forgets_files_that_are_gone() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("drawings.txt");
    let (app, mut bridge) = setup();
    let mut app = app.with_written_drawings(&store);
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let document = directory.path().join("plan.ketchup");
    let save = Request::SaveAs {
        expected: None,
        path: document.display().to_string(),
    };
    bridge.execute(&mut app, save, false).unwrap();
    let mut export = |name: &str| {
        let path = directory.path().join(name);
        bridge
            .execute(
                &mut app,
                export_drawings(&path, json!({"sheet_id": id})),
                false,
            )
            .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    };
    export("first.dxf");
    let listed = || std::fs::read_to_string(&store).unwrap();
    assert!(listed().contains("first.dxf"), "{}", listed());
    std::fs::remove_file(directory.path().join("first.dxf")).unwrap();
    export("second.dxf");
    assert!(!listed().contains("first.dxf"), "{}", listed());
    assert!(listed().contains("second.dxf"), "{}", listed());
    export("second.dxf");
    assert_eq!(listed().lines().count(), 1, "{:?}", listed());
}

/// Review 2026-11-06 P3: a client's export of a drawing sheet took the SHA-256
/// of the file at the path, drew the sheet and wrote it on the window's
/// thread. It runs as a job, answered once its worker is done; the file of
/// someone else's is still kept unless the client says replace.
#[test]
fn a_clients_sheet_export_runs_off_the_windows_thread() {
    use crate::live_bridge::program_check::ProgramCheckJob;
    use crate::live_bridge::program_dry_run::QueuedReply;
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let dwg = directory.path().join("Block.dwg");
    std::fs::write(&dwg, b"the user's own").unwrap();
    let context = egui::Context::default();
    let mut export = |app: &mut KetchupApp, extra: Value| {
        let (reply, answers) = mpsc::sync_channel(1);
        let queued = QueuedReply {
            id: 1,
            reply,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        bridge.start_project_drawings_export(
            app,
            &context,
            queued,
            export_drawings(&dwg, extra),
            false,
        );
        assert!(
            matches!(
                bridge.program_check_job,
                Some(ProgramCheckJob::ProjectDrawings(_))
            ),
            "the window drew and wrote the sheet itself"
        );
        assert!(answers.try_recv().is_err(), "answered before its job");
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            assert!(Instant::now() < deadline, "the export did not finish");
            bridge.poll_program_check_job(app, &context);
            if let Ok(answer) = answers.recv_timeout(Duration::from_millis(5)) {
                break answer;
            }
        }
    };
    let kept = export(&mut app, json!({"sheet_id": id}));
    assert_eq!(
        kept.error.as_deref(),
        Some("drawings_target_exists"),
        "{kept:?}"
    );
    assert_eq!(std::fs::read(&dwg).unwrap(), b"the user's own");
    let asked = export(&mut app, json!({"sheet_id": id, "replace": true}));
    assert!(asked.ok, "{asked:?}");
    assert_eq!(asked.result.unwrap()["sheet"], "Block");
    assert!(std::fs::read(&dwg).unwrap().starts_with(b"AC1032"));
    let again = export(&mut app, json!({"sheet_id": id}));
    assert!(again.ok, "its own file is replaced unasked: {again:?}");
}

/// MECH-2: a PDF another program holds open (a PDF viewer shares it for
/// reading only) cannot be replaced: the answer names the file and says to
/// close it, the file keeps its bytes and nothing is left beside it.
#[cfg(windows)]
#[test]
fn a_drawing_held_open_elsewhere_is_named_and_kept() {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_SHARE_READ: u32 = 1;
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("house.pdf");
    std::fs::write(&pdf, b"open in a viewer").unwrap();
    let viewer = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&pdf)
        .unwrap();
    assert_eq!(
        bridge.execute(
            &mut app,
            export_drawings(&pdf, json!({"replace": true})),
            false
        ),
        Err("drawings_target_in_use")
    );
    let answer = take_error_details().unwrap_or_default().to_string();
    assert!(answer.contains("house.pdf"), "{answer}");
    drop(viewer);
    assert_eq!(std::fs::read(&pdf).unwrap(), b"open in a viewer");
    assert_eq!(names_in(directory.path()), ["house.pdf"]);
}

/// MECH-2 (operator 2026-11-03): a set replaces only the sheets this
/// project's earlier export wrote; a file of someone else's named like one of
/// its sheets refuses the export, and the answer names the file and says what
/// to do: rename or move it, or export to another folder.
#[test]
fn a_file_of_someone_elses_named_like_a_sheet_is_named_and_kept() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);
    for foreign in ["house-02.dxf", "house-09.dxf"] {
        std::fs::write(file(foreign), b"another project's sheet").unwrap();
        assert_eq!(
            bridge.execute(
                &mut app,
                export_drawings(&file("house.dxf"), json!({})),
                false
            ),
            Err("drawings_target_exists"),
            "{foreign}"
        );
        let answer = take_error_details().unwrap_or_default().to_string();
        for said in [foreign, "rename", "another folder"] {
            assert!(answer.contains(said), "{said:?} missing: {answer}");
        }
        assert_eq!(
            std::fs::read(file(foreign)).unwrap(),
            b"another project's sheet"
        );
        assert_eq!(
            names_in(directory.path()),
            [foreign],
            "nothing else written"
        );
        std::fs::remove_file(file(foreign)).unwrap();
    }
}

/// Review 2026-11-04 r19 MF-162: the set's list named the project by its
/// absolute path, compared as text, so a project renamed or moved (another
/// folder, drive letter or letter case, Save As "v2") refused its own set as
/// "changed concurrently after authorization", and the list carried the
/// user's name in that path. Which list the document wrote is kept, and it
/// stays its own once moved; another project, or a copy, is told whose set it
/// is and may take it over.
#[test]
fn a_moved_project_exports_its_set_again_and_another_is_told_whose_it_is() {
    let directory = tempfile::tempdir().unwrap();
    let folder = |name: &str| {
        let folder = directory.path().join(name);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    };
    let set = folder("drawings").join("house.dxf");
    let saved = folder("projects").join("house.ketchup");
    let request = |value: Value| serde_json::from_value::<Request>(value).unwrap();
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge, extra: Value| {
        let answer = bridge.execute(app, export_drawings(&set, extra), false);
        answer.map_err(|code| (code, take_error_details().unwrap_or_default().to_string()))
    };

    let store = directory.path().join("drawings.txt");
    let with_store = || {
        let (app, bridge) = setup();
        (app.with_written_drawings(&store), bridge)
    };
    let (mut app, mut bridge) = with_store();
    evaluate_exact(&mut app);
    let save_as = |path: &Path| request(json!({"method": "save_as", "path": path}));
    bridge.execute(&mut app, save_as(&saved), false).unwrap();
    export(&mut app, &mut bridge, json!({})).unwrap();
    bridge.execute(&mut app, save_as(&saved), false).unwrap();
    let list = std::fs::read_to_string(set.with_file_name("house.dxf.sheets.txt")).unwrap();
    assert!(
        !list.contains(&directory.path().display().to_string()),
        "no absolute path: {list}"
    );

    // A copy of the project, the original left where it was, is not it
    // (review 2026-11-06 SEC-182).
    let copy = folder("copies").join("house copy.ketchup");
    std::fs::copy(&saved, &copy).unwrap();
    let (mut copied, mut copy_bridge) = with_store();
    let open = request(json!({"method": "open", "path": copy}));
    copy_bridge.execute(&mut copied, open, false).unwrap();
    evaluate_exact(&mut copied);
    let (code, answer) = export(&mut copied, &mut copy_bridge, json!({})).unwrap_err();
    assert_eq!(code, "drawings_set_of_another_project", "{answer}");
    drop((copied, copy_bridge));
    std::fs::remove_file(&copy).unwrap();

    let moved = folder("moved").join("house v2.ketchup");
    std::fs::rename(&saved, &moved).unwrap();
    drop((app, bridge));
    let (mut app, mut bridge) = with_store();
    let open = request(json!({"method": "open", "path": moved}));
    bridge.execute(&mut app, open, false).unwrap();
    evaluate_exact(&mut app);
    export(&mut app, &mut bridge, json!({}))
        .unwrap_or_else(|refused| panic!("the moved project's own set: {refused:?}"));
    let list = std::fs::read_to_string(set.with_file_name("house.dxf.sheets.txt")).unwrap();
    assert!(list.contains("house v2.ketchup"), "{list}");

    let (mut other, mut other_bridge) = with_store();
    evaluate_exact(&mut other);
    let (code, answer) = export(&mut other, &mut other_bridge, json!({})).unwrap_err();
    assert_eq!(code, "drawings_set_of_another_project", "{answer}");
    // Taking it over deletes sheets of the other set, which the user is told
    // before (review 2026-11-06 P3).
    for said in [
        "house v2.ketchup",
        "take_over",
        "Ask the user",
        "deletes those beyond the last sheet",
        "\"sheets\":",
    ] {
        assert!(answer.contains(said), "{said:?} missing: {answer}");
    }
    export(&mut other, &mut other_bridge, json!({"take_over": true})).unwrap();
    // Taken over, the set is no longer the moved project's.
    let (code, _) = export(&mut app, &mut bridge, json!({})).unwrap_err();
    assert_eq!(code, "drawings_set_of_another_project");
}

/// Review 2026-11-06 MF-182: a project moved together with its drawings
/// folder, `C:\Jobs\Dom\` to `D:\Archiv\Dom\` or a folder synced to a
/// colleague, refused its own set as exported "from dom.ketchup, not this
/// document": the list it wrote was kept by its old path. A list of the same
/// document naming the project's file is its own.
#[test]
fn a_project_moved_with_its_drawings_folder_exports_its_set_again() {
    let directory = tempfile::tempdir().unwrap();
    let request = |value: Value| serde_json::from_value::<Request>(value).unwrap();
    let job = directory.path().join("Jobs").join("Dom");
    std::fs::create_dir_all(job.join("dwg")).unwrap();
    let store = directory.path().join("drawings.txt");
    let (app, mut bridge) = setup();
    let mut app = app.with_written_drawings(&store);
    evaluate_exact(&mut app);
    let save = request(json!({"method": "save_as", "path": job.join("dom.ketchup")}));
    bridge.execute(&mut app, save, false).unwrap();
    let set = job.join("dwg").join("dom.dxf");
    bridge
        .execute(&mut app, export_drawings(&set, json!({})), false)
        .unwrap();
    drop((app, bridge));

    let archive = directory.path().join("Archiv");
    std::fs::create_dir_all(&archive).unwrap();
    let moved = archive.join("Dom");
    std::fs::rename(&job, &moved).unwrap();
    // On this computer, and on a colleague's, which never wrote the set.
    for store in [store, directory.path().join("colleague.txt")] {
        let (app, mut bridge) = setup();
        let mut app = app.with_written_drawings(&store);
        let open = request(json!({"method": "open", "path": moved.join("dom.ketchup")}));
        bridge.execute(&mut app, open, false).unwrap();
        evaluate_exact(&mut app);
        let set = moved.join("dwg").join("dom.dxf");
        bridge
            .execute(&mut app, export_drawings(&set, json!({})), false)
            .unwrap_or_else(|code| panic!("{store:?} {code}: {:?}", take_error_details()));
    }
}

/// One DXF entity: its type, layer, numeric codes and KETCHUP id.
struct Entity {
    kind: String,
    layer: String,
    codes: BTreeMap<u16, f64>,
    id: String,
    text: String,
}

/// The entities of the ENTITIES section, read pair by pair (a code may be
/// padded with spaces, as other writers do).
fn entities(dxf: &str) -> Vec<Entity> {
    let lines = dxf.lines().collect::<Vec<_>>();
    let pairs = lines
        .chunks(2)
        .filter_map(|pair| match pair {
            [code, value] => Some((code.trim(), value.trim())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let start = pairs
        .iter()
        .position(|pair| *pair == ("2", "ENTITIES"))
        .unwrap()
        + 1;
    let mut entities = Vec::<Entity>::new();
    for &(code, value) in &pairs[start..] {
        if (code, value) == ("0", "ENDSEC") {
            break;
        }
        match code {
            "0" => entities.push(Entity {
                kind: (*value).to_owned(),
                layer: String::new(),
                codes: BTreeMap::new(),
                id: String::new(),
                text: String::new(),
            }),
            "8" => entities.last_mut().unwrap().layer = (*value).to_owned(),
            "1000" => entities.last_mut().unwrap().id = (*value).to_owned(),
            "1" => entities.last_mut().unwrap().text = (*value).to_owned(),
            code => {
                if let (Ok(code), Ok(number)) = (code.parse::<u16>(), value.parse::<f64>()) {
                    entities.last_mut().unwrap().codes.insert(code, number);
                }
            }
        }
    }
    entities
}

fn edges_of_kind(app: &mut KetchupApp, bridge: &mut LiveBridge, kind: &str) -> Vec<Value> {
    let query = json!({"kind": "edges", "limit": 100, "search": kind});
    let page = bridge
        .execute(
            app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(query).unwrap(),
            },
            false,
        )
        .unwrap();
    assert!(page["next_cursor"].is_null(), "{page}");
    let rows = page["items"].as_array().unwrap().clone();
    let last = rows
        .iter()
        .filter_map(|row| row["producer_feature_id"].as_u64())
        .max();
    rows.into_iter()
        .filter(|row| row["producer_feature_id"].as_u64() == last)
        .collect()
}

#[test]
fn a_drilled_hole_and_a_rounded_corner_are_an_exact_circle_and_arc_in_the_dxf() {
    const HOLE_MM: f64 = 6.0;
    const ROUND_MM: f64 = 5.0;
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let [x, y, z] = centroid(&top);
    let drilled = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::DrillHoles {
                definition_id: top["definition_id"].as_u64().unwrap(),
                name: "hole".to_owned(),
                target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
                face_reference_id: top["reference_id"].as_str().unwrap().to_owned(),
                points_local_mm: vec![[x, y, z]],
                diameter_mm: HOLE_MM,
                depth_mm: None,
                through: true,
                thread: None,
            }),
            false,
        )
        .unwrap();
    assert_eq!(drilled["published"], true, "{drilled}");
    evaluate_exact(&mut app);
    // The vertical edge at the corner of largest x + y.
    let corner = edges_of_kind(&mut app, &mut bridge, "line")
        .into_iter()
        .filter(|edge| {
            let bounds = &edge["geometry"]["bounds_mm"];
            let span =
                |axis: usize| bounds[1][axis].as_f64().unwrap() - bounds[0][axis].as_f64().unwrap();
            span(0).abs() < 1e-3 && span(1).abs() < 1e-3 && span(2) > 1.0
        })
        .max_by(|left, right| {
            let sum = |edge: &Value| {
                edge["geometry"]["centroid_mm"][0].as_f64().unwrap()
                    + edge["geometry"]["centroid_mm"][1].as_f64().unwrap()
            };
            sum(left).total_cmp(&sum(right))
        })
        .expect("a vertical edge");
    let rounded = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::FilletEdges {
                definition_id: corner["definition_id"].as_u64().unwrap(),
                name: "round".to_owned(),
                target_feature_id: corner["producer_feature_id"].as_u64().unwrap(),
                edge_reference_ids: vec![corner["reference_id"].as_str().unwrap().to_owned()],
                radius_mm: ROUND_MM,
            }),
            false,
        )
        .unwrap();
    assert_eq!(rounded["published"], true, "{rounded}");
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rounded.dxf");
    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let index = drawing
        .views
        .iter()
        .position(|view| view.kind.stable_name() == "top")
        .unwrap();
    let (view, placement) = (
        &drawing.views[index],
        &drawing.layout.view_placements[index],
    );
    let scale = (placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0])
        / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
    let model = |page_x: f64, page_y: f64| {
        [
            (page_x - placement.origin_mm[0]) / scale,
            (page_y - placement.origin_mm[1]) / scale,
        ]
    };
    let prefix = format!("{}/", view.stable_view_id);
    let entities = entities(&written);
    let top_view = entities
        .iter()
        .filter(|entity| entity.id.starts_with(&prefix))
        .collect::<Vec<_>>();
    // DXF numbers carry six decimals of page millimetres.
    let close = |left: f64, right: f64| (left - right).abs() <= 2e-6 / scale.min(1.0);

    // The hole is one CIRCLE at the centre of the top face, radius 3.
    let [lo, hi] = view.bounds_mm;
    let middle = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let hole = top_view
        .iter()
        .find(|entity| entity.kind == "CIRCLE" && close(entity.codes[&40] / scale, HOLE_MM * 0.5))
        .unwrap_or_else(|| panic!("no hole circle in the top view:\n{written}"));
    assert_eq!(hole.layer, "VISIBLE");
    let center = model(hole.codes[&10], hole.codes[&20]);
    assert!(
        close(center[0], middle[0]) && close(center[1], middle[1]),
        "{center:?} != {middle:?}"
    );
    // No line of the top view is a chord of the hole any more.
    let on_hole = |page_x: f64, page_y: f64| {
        let point = model(page_x, page_y);
        close(
            (point[0] - center[0]).hypot(point[1] - center[1]),
            HOLE_MM * 0.5,
        )
    };
    assert!(
        !top_view.iter().any(|entity| entity.kind == "LINE"
            && on_hole(entity.codes[&10], entity.codes[&20])
            && on_hole(entity.codes[&11], entity.codes[&21])),
        "the hole is still drawn as lines"
    );

    // The rounded corner is a quarter ARC of radius 5, its centre 5 inside
    // both sides it joins.
    let round = top_view
        .iter()
        .find(|entity| entity.kind == "ARC" && entity.layer == "VISIBLE")
        .unwrap_or_else(|| panic!("no arc in the top view:\n{written}"));
    assert!(close(round.codes[&40] / scale, ROUND_MM));
    let span = (round.codes[&51] - round.codes[&50]).rem_euclid(360.0);
    assert!((span - 90.0).abs() < 1e-4, "{span}");
    let center = model(round.codes[&10], round.codes[&20]);
    let inside = |axis: usize| {
        close(hi[axis] - center[axis], ROUND_MM) || close(center[axis] - lo[axis], ROUND_MM)
    };
    assert!(inside(0) && inside(1), "{center:?} in {lo:?}..{hi:?}");

    let curves = &exported["dxf_curves"];
    let count = |kind: &str| entities.iter().filter(|entity| entity.kind == kind).count();
    assert_eq!(curves["circles"], count("CIRCLE"), "{curves}");
    assert_eq!(curves["arcs"], count("ARC"), "{curves}");
    assert!(curves["replaced_lines"].as_u64().unwrap() > 0, "{curves}");
    let lines = drawing
        .views
        .iter()
        .map(|view| view.visible_lines.len() + view.hidden_lines.len())
        .sum::<usize>();
    assert_eq!(
        layer_lines(&written, "VISIBLE") + layer_lines(&written, "HIDDEN"),
        lines - curves["replaced_lines"].as_u64().unwrap() as usize,
        "every replaced line is gone, every other line stays"
    );
    // Front and right see the hole and the round edge-on, as straight lines;
    // no exact curve is left as lines.
    assert_eq!(curves["curves_as_lines"], json!([]), "{curves}");
    assert_eq!(curves["curves_as_lines_count"], 0);

    assert_the_dwg_carries_the_dxf(
        &mut app,
        &mut bridge,
        id,
        &written,
        &directory.path().join("rounded.dwg"),
    );
}

/// A round bar with a flat (review DR-1): Ø100 with a 3 mm flat is, seen along
/// its axis, an ARC of 320.1 degrees and the LINE of the flat. The flat's ends
/// lie on the exact circle and it spans 39.9 degrees, like one step of a
/// tessellated circle, but it is no part of the circular edge.
#[test]
fn a_round_bar_with_a_flat_is_an_arc_and_the_line_of_the_flat_in_the_dxf() {
    const RADIUS_MM: f64 = 50.0;
    const FLAT_MM: f64 = 3.0;
    let (mut app, mut bridge) = setup();
    app.new_document();
    let source = format!(
        "bar = revolve('bar', profile = [(0, 0), ({RADIUS_MM}, 0), ({RADIUS_MM}, 20), (0, 20)], \
         axis = [(0, 0), (0, 1)])\n\
         rotate(bar, axis = (1, 0, 0), angle = 90)\n\
         trim(bar, ({}, 0, 0), (1, 0, 0))\n",
        RADIUS_MM - FLAT_MM
    );
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "flat.star".into(),
            source,
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("flat.dxf");
    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    // The view along the bar's axis: the one that faces its rims.
    let index = drawing
        .views
        .iter()
        .position(|view| !view.circles.is_empty() && view.kind.stable_name() != "back")
        .unwrap();
    let (view, placement) = (
        &drawing.views[index],
        &drawing.layout.view_placements[index],
    );
    let scale = (placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0])
        / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
    let prefix = format!("{}/", view.stable_view_id);
    let entities = entities(&written);
    let axis_view = entities
        .iter()
        .filter(|entity| entity.id.starts_with(&prefix) && entity.layer == "VISIBLE")
        .collect::<Vec<_>>();
    let close = |left: f64, right: f64| (left - right).abs() <= 1e-4;

    assert!(
        !axis_view.iter().any(|entity| entity.kind == "CIRCLE"),
        "the bar with a flat is drawn as a full circle:\n{written}"
    );
    let half_gap = ((RADIUS_MM - FLAT_MM) / RADIUS_MM).acos().to_degrees();
    let rim = axis_view
        .iter()
        .find(|entity| entity.kind == "ARC" && close(entity.codes[&40] / scale, RADIUS_MM))
        .unwrap_or_else(|| {
            panic!(
                "no rim arc in the view along the axis: {} {:?} {:?}",
                exported["dxf_curves"],
                view.circles,
                axis_view
                    .iter()
                    .map(|entity| (
                        entity.kind.as_str(),
                        entity.codes.get(&40).map(|r| r / scale)
                    ))
                    .collect::<Vec<_>>()
            )
        });
    let span = (rim.codes[&51] - rim.codes[&50]).rem_euclid(360.0);
    assert!((span - (360.0 - 2.0 * half_gap)).abs() < 1e-3, "{span}");
    let chord = 2.0 * RADIUS_MM * half_gap.to_radians().sin();
    assert!(
        axis_view.iter().any(|entity| {
            entity.kind == "LINE"
                && close(
                    (entity.codes[&11] - entity.codes[&10])
                        .hypot(entity.codes[&21] - entity.codes[&20])
                        / scale,
                    chord,
                )
        }),
        "the flat of {chord} mm is no line of the view along the axis:\n{written}"
    );
}

/// The geometry of every LINE, CIRCLE and ARC: type, layer, KETCHUP id and
/// its numbers to five decimals of page millimetres, sorted.
fn drawn(dxf: &str) -> Vec<String> {
    let mut drawn = entities(dxf)
        .into_iter()
        .filter(|entity| matches!(entity.kind.as_str(), "LINE" | "CIRCLE" | "ARC"))
        .map(|entity| {
            let numbers = [10, 20, 11, 21, 40, 50, 51]
                .iter()
                .filter_map(|code| {
                    entity
                        .codes
                        .get(code)
                        .map(|value| format!("{code}={value:.5}"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {} {} {numbers}", entity.kind, entity.layer, entity.id)
        })
        .collect::<Vec<_>>();
    drawn.sort();
    drawn
}

/// The `\U+XXXX` escapes of an R12 string as the characters they stand for.
fn decode_dxf_unicode(value: &str) -> String {
    let mut units = Vec::new();
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        if let Some(hex) = rest.strip_prefix("\\U+").and_then(|tail| tail.get(..4)) {
            units.push(u16::from_str_radix(hex, 16).unwrap());
            rest = &rest[7..];
        } else {
            let mut buffer = [0; 2];
            units.extend_from_slice(character.encode_utf16(&mut buffer));
            rest = &rest[character.len_utf8()..];
        }
    }
    String::from_utf16(&units).unwrap()
}

/// The same sheet as DWG: an AutoCAD 2018 file that reads back with every
/// line, circle and arc of the DXF on its layer, at its place, with its id.
fn assert_the_dwg_carries_the_dxf(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    id: u64,
    dxf: &str,
    path: &Path,
) {
    let exported = bridge
        .execute(app, export_drawings(path, json!({"sheet_id": id})), false)
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");
    assert_eq!(exported["file_format"], "dwg");
    let dwg = std::fs::read(path).unwrap();
    assert_eq!(exported["bytes"], dwg.len());
    assert!(dwg.starts_with(ketchup_dwg::WRITTEN_VERSION.as_bytes()));
    let read = ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap();
    assert_eq!(read.version, ketchup_dwg::WRITTEN_VERSION);
    assert!(read.skipped.is_empty(), "{:?}", read.skipped);
    let back = String::from_utf8(read.dxf).unwrap();
    let expected = drawn(dxf);
    assert!(expected.iter().any(|entity| entity.starts_with("CIRCLE ")));
    assert!(expected.iter().any(|entity| entity.starts_with("ARC ")));
    assert_eq!(drawn(&back), expected);
    // The R12 sheet writes a character outside ASCII as its \U+XXXX escape;
    // the DWG holds the character itself.
    let texts = |dxf: &str| {
        let mut texts = entities(dxf)
            .into_iter()
            .filter(|entity| entity.kind == "TEXT")
            .map(|entity| (entity.id, decode_dxf_unicode(&entity.text)))
            .collect::<Vec<_>>();
        texts.sort();
        texts
    };
    assert!(!texts(dxf).is_empty());
    assert_eq!(texts(&back), texts(dxf));
}

/// The top view of a sheet of several parts draws the plan's shown annotations where they lie: the
/// texts at their points, each dimension with the value Kečup measured centred
/// above its line, and the leader. A dimension straight up has no length in
/// plan and one text off the sheet is named; hidden, they leave the sheet.
#[test]
fn a_drawing_sheet_draws_the_plan_annotations_in_its_top_view() {
    let (mut app, mut bridge) = setup();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('slab', (6000, 3000, 200), at=(0, 0, -200))\n\
                 box('post', (200, 200, 2000), at=(5800, 2800, 0))\n\
                 points = [(0, 0), (3000, 0), (6000, 0)]\n\
                 for i, p in enumerate(points):\n    \
                 text('P' + str(i + 1), 'P' + str(i + 1), at=p, height_mm=3.5, tags='plan')\n\
                 chain_dim('south', points, offset_mm=500, tags='plan')\n\
                 leader('anchor', 'M24', point=(1000, 1000), at=(2000, 2000), tags='plan')\n\
                 dim('up', (0, 0, 0), (0, 0, 2000), offset_mm=300, tags='plan')\n\
                 text('far', 'far', at=(900000, 0), tags='plan')\n"
            .into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let id = sheet_of_parts(&mut app, &mut bridge, 2);
    let directory = tempfile::tempdir().unwrap();
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge, name: &str| {
        let path = directory.path().join(name);
        let exported = bridge
            .execute(app, export_drawings(&path, json!({"sheet_id": id})), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        (exported, std::fs::read_to_string(&path).unwrap())
    };

    let (exported, written) = export(&mut app, &mut bridge, "plan.dxf");
    assert_eq!(
        exported["annotations"],
        json!({"texts": 3, "dimensions": 2, "leaders": 1, "lines": 0,
               "without_plan_view": 0, "outside_plan": 1,
               "off_sheet": ["far"], "off_sheet_count": 1}),
        "{exported}"
    );
    let sheet = entities(&written);
    let of = |kind: &str, id: &str| {
        let found = sheet
            .iter()
            .filter(|entity| entity.kind == kind && entity.id == format!("annotation-{id}"))
            .collect::<Vec<_>>();
        assert!(!found.is_empty(), "no {kind} of {id}");
        assert!(
            found.iter().all(|entity| entity.layer == "plan"),
            "{id} is on the tag's layer"
        );
        found
    };
    let at = |entity: &Entity| [entity.codes[&10], entity.codes[&20]];
    let points = ["P1", "P2", "P3"].map(|name| {
        let [label] = of("TEXT", name).try_into().ok().unwrap();
        assert_eq!(label.text, name);
        at(label)
    });
    // The points lie 3000 mm apart along the sheet's x at the view's scale.
    let scale = (points[1][0] - points[0][0]) / 3000.0;
    assert!(scale > 0.0);
    assert!((points[2][0] - points[1][0] - 3000.0 * scale).abs() < 0.01);
    assert!((points[1][1] - points[0][1]).abs() < 0.01);
    // Each dimension is a DIMENSION through its two points, carrying the
    // measured value, which its block writes centred above the line.
    for (name, pair) in ["south/1", "south/2"].iter().zip(points.windows(2)) {
        let [dimension] = of("DIMENSION", name).try_into().ok().unwrap();
        assert_eq!(dimension.text, "3000");
        assert_eq!(dimension.codes[&70], 1.0, "aligned");
        for (code, point) in [(13, pair[0]), (14, pair[1])] {
            assert!((dimension.codes[&code] - point[0]).abs() < 0.01);
            assert!((dimension.codes[&(code + 10)] - point[1]).abs() < 0.01);
        }
        let middle = [dimension.codes[&11], dimension.codes[&21]];
        assert!((middle[0] - f64::midpoint(pair[0][0], pair[1][0])).abs() < 0.01);
        // 500 mm into the plan, and 1 mm on paper above the line.
        assert!((middle[1] - (pair[0][1] + 500.0 * scale + 1.0)).abs() < 0.01);
    }
    let block_values = written.matches("\n1\n3000\n").count();
    assert_eq!(
        block_values, 4,
        "two DIMENSION values and the two TEXTs of their blocks"
    );
    let [leader] = of("TEXT", "anchor").try_into().ok().unwrap();
    assert_eq!(leader.text, "M24");
    // Review 2026-10-24 MF-28: the leader's arrowhead is filled, as in the
    // plan DXF and the drawing set, not two open strokes.
    let [arrow] = of("SOLID", "anchor-arrow").try_into().ok().unwrap();
    // Its tip at the leader's point, 1000 mm before the text in x and y.
    for axis in 0..2 {
        assert!((at(arrow)[axis] - (at(leader)[axis] - 1000.0 * scale)).abs() < 0.01);
    }
    assert!(text_free(&sheet, "up") && text_free(&sheet, "far"));

    // As DWG the sheet keeps both DIMENSION entities and the texts on the tag's layer.
    let dwg = directory.path().join("plan.dwg");
    bridge
        .execute(
            &mut app,
            export_drawings(&dwg, json!({"sheet_id": id})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let back = ketchup_dwg::dwg_to_dxf(&std::fs::read(&dwg).unwrap(), 64 << 20).unwrap();
    let back = entities(&String::from_utf8(back.dxf).unwrap());
    let on_plan = |kind: &str| {
        back.iter()
            .filter(|entity| entity.kind == kind && entity.layer == "plan")
            .collect::<Vec<_>>()
    };
    let dimensions = on_plan("DIMENSION");
    assert_eq!(dimensions.len(), 2);
    assert!(dimensions.iter().all(|dimension| dimension.text == "3000"));
    let mut labels = on_plan("TEXT")
        .iter()
        .map(|text| text.text.clone())
        .collect::<Vec<_>>();
    labels.sort();
    assert_eq!(labels, ["M24", "P1", "P2", "P3"]);

    // The PDF of the same sheet writes both values centred by a text matrix.
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let bundle = export_drawing(&snapshot, &drawing).unwrap();
    let pdf = String::from_utf8_lossy(bundle.pdf().unwrap());
    let value = ketchup_model::sheet_pdf::EmbeddedText::default()
        .hex("3000")
        .unwrap();
    assert_eq!(pdf.matches(&format!("Tm <{value}> Tj")).count(), 2, "{pdf}");
    assert_eq!(pdf.matches(" l h f\n").count(), 1, "the arrowhead filled");
    let svg = String::from_utf8_lossy(bundle.svg());
    assert!(
        svg.contains("<polygon id=\"annotation-anchor-arrow\""),
        "{svg}"
    );
    drop(snapshot);

    // Hidden, the annotations leave the sheet.
    let hide = Request::TagVisibility {
        expected: None,
        name: "plan".into(),
        visible: false,
    };
    bridge
        .execute(&mut app, hide, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let (exported, written) = export(&mut app, &mut bridge, "hidden.dxf");
    assert_eq!(exported["annotations"]["texts"], 0, "{exported}");
    assert_eq!(exported["annotations"]["dimensions"], 0);
    assert!(!written.contains("annotation-"));
}

/// Review 2026-10-24 MF-22: the plan's annotations were drawn into every top
/// view in the plan's coordinates. A sheet of one part draws it in the part's
/// own frame, so a slab placed at (5000, 4000) got its texts moved by that
/// much; such a sheet now draws none and counts them. On a sheet of parts drawn
/// where they lie, an annotation landing in another view is left out and named.
#[test]
fn plan_annotations_are_drawn_only_in_a_plan_view_and_inside_it() {
    let parts = "box('slab', (6000, 3000, 200), at=(5000, 4000, -200))\n\
                 box('post', (200, 200, 2000), at=(10800, 6800, 0))\n\
                 text('P1', 'P1', at=(5000, 4000), tags='plan')\n";
    let directory = tempfile::tempdir().unwrap();
    let exports = std::cell::Cell::new(0);
    let sheet = |source: String, parts: usize| {
        exports.set(exports.get() + 1);
        let (mut app, mut bridge) = setup();
        let program = Request::ApplyProgram {
            expected: None,
            source,
            overrides: None,
            file_name: Some("plan.star".into()),
            replace_document: true,
        };
        bridge
            .execute(&mut app, program, false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        let id = sheet_of_parts(&mut app, &mut bridge, parts);
        let path = directory.path().join(format!("plan-{}.dxf", exports.get()));
        let exported = bridge
            .execute(
                &mut app,
                export_drawings(&path, json!({"sheet_id": id})),
                false,
            )
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        let written = entities(&std::fs::read_to_string(&path).unwrap());
        let snapshot = app.document.current();
        let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
        let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
        (exported["annotations"].clone(), written, drawing)
    };
    let counts = |texts: usize, without_plan_view: usize, off_sheet: &[&str]| {
        json!({"texts": texts, "dimensions": 0, "leaders": 0, "lines": 0,
               "without_plan_view": without_plan_view, "outside_plan": 0,
               "off_sheet": off_sheet, "off_sheet_count": off_sheet.len()})
    };

    // The slab alone, in its own frame: the sheet has no plan to draw P1 in.
    let (annotations, written, _) = sheet(parts.to_owned(), 1);
    assert_eq!(annotations, counts(0, 1, &[]));
    assert!(text_free(&written, "P1"));

    // Both parts, where they lie: P1 is drawn at its point of the plan.
    let (annotations, written, drawing) = sheet(parts.to_owned(), 2);
    assert_eq!(annotations, counts(1, 0, &[]));
    let top = drawing
        .views
        .iter()
        .position(|view| view.kind == ketchup_model::drawing::OrthographicViewKind::Top)
        .unwrap();
    let (view, plan) = (&drawing.views[top], &drawing.layout.view_placements[top]);
    let scale = (plan.page_bounds_mm[1][0] - plan.page_bounds_mm[0][0])
        / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
    let [label] = written
        .iter()
        .filter(|entity| entity.kind == "TEXT" && entity.id == "annotation-P1")
        .collect::<Vec<_>>()
        .try_into()
        .ok()
        .unwrap();
    for (axis, (code, world)) in [(10, 5000.0), (20, 4000.0)].into_iter().enumerate() {
        let expected = plan.origin_mm[axis] + world * scale;
        assert!(
            (label.codes[&code] - expected).abs() < 0.01,
            "{:?}",
            label.codes
        );
    }

    // A text whose point of the plan lands in the middle of another view.
    let other = drawing
        .layout
        .view_placements
        .iter()
        .find(|placement| placement.kind != plan.kind)
        .unwrap();
    let stray = [0, 1].map(|axis| {
        let middle = f64::midpoint(other.page_bounds_mm[0][axis], other.page_bounds_mm[1][axis]);
        (middle - plan.origin_mm[axis]) / scale
    });
    let source = format!(
        "{parts}text('stray', 'stray', at=({:.3}, {:.3}), tags='plan')\n",
        stray[0], stray[1]
    );
    let (annotations, written, _) = sheet(source, 2);
    assert_eq!(annotations, counts(1, 0, &["stray"]));
    assert!(text_free(&written, "stray") && !text_free(&written, "P1"));
}

fn text_free(entities: &[Entity], id: &str) -> bool {
    let id = format!("annotation-{id}");
    !entities.iter().any(|entity| entity.id.starts_with(&id))
}
