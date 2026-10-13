//! File > Import DXF/DWG in the window: a `.dwg` drawing opens the DXF review
//! with the DWG version named, and confirming it imports the profiles in one
//! Undo step.
use crate::app::dwg_import::LAYERED_DRAWING_DXF;
use crate::*;
use egui_kittest::kittest::Queryable as _;

#[test]
fn the_window_reviews_and_imports_a_dwg_drawing() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = directory.path().join("plan.dwg");
    std::fs::write(
        &dwg,
        ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap(),
    )
    .unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &dwg);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let undo = app.undo_step_count();

    app.dispatch_file_command(AppCommand::ImportDrawingDxf);
    let [request] = script.import_requests().try_into().unwrap();
    assert_eq!(request.extensions, ["dxf", "dwg"]);
    assert_eq!(
        request.filter_label,
        app.catalog.text("file-filter-dxf-dwg")
    );
    let pending = app
        .modal
        .get::<PendingDxfImport>()
        .unwrap_or_else(|| panic!("no review: {}", app.action_digest()));
    assert_eq!(pending.plan.review.profiles().len(), 3);
    let conversion = pending.plan.dwg.clone().expect("read from a DWG file");
    assert_eq!(conversion.version, ketchup_dwg::WRITTEN_VERSION);
    assert!(conversion.skipped.is_empty());
    assert!(pending.unit_confirmed, "the DWG declares millimetres");

    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let named = harness.state().catalog.format(
        "dialog-import-dxf-dwg",
        &BTreeMap::from([("version", conversion.version.clone())]),
    );
    harness.get_by_label(&named);
    let confirm = harness.state().catalog.text("dialog-import-dxf-confirm");
    harness.get_by_label(&confirm).click();
    harness.run();

    let app = harness.state();
    assert!(app.modal.get::<PendingDxfImport>().is_none());
    assert_eq!(
        app.digest,
        app.catalog.format(
            "digest-imported-dxf",
            &BTreeMap::from([("path", dwg.display().to_string())]),
        )
    );
    assert_eq!(app.undo_step_count(), undo + 1);
}
