//! File > Drawings in the window: a drawing sheet of the document exports as
//! DXF from its own row, the project sheet still exports as PDF, and neither
//! changes the document.
use crate::*;
use egui_kittest::kittest::Queryable as _;
use ketchup_model::document::InstancePath;

#[test]
fn the_drawings_window_exports_a_document_sheet_as_dxf_and_the_project_sheet_as_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let (dxf, pdf) = (
        directory.path().join("block.dxf"),
        directory.path().join("house.pdf"),
    );
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&dxf)
        .queue_export(&pdf);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let root = app.document.current().occurrences().next().unwrap().id();
    app.clear_selection();
    app.selection.occurrences.insert(InstancePath::root(root));
    assert!(app.preview_selection_drawing(), "{}", app.action_digest());
    assert!(app.confirm_assembly_preview());
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let sheet = app.document.current().drawing_sheets().next().unwrap().id();
    let (expected, dxf_bytes) = app.drawing_sheet_dxf(sheet).unwrap();
    let state = (
        app.document_revision(),
        app.canonical_digest(),
        app.undo_step_count(),
        app.is_dirty(),
    );

    app.dispatch_file_command(AppCommand::ExportProjectDrawings);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let label = harness.state().catalog.text("drawings-export-sheet-dxf");
    harness.get_by_label(&label).click();
    harness.run();
    assert_eq!(std::fs::read(&dxf).unwrap(), dxf_bytes);
    assert!(
        harness.state().digest.contains(&expected.name),
        "{}",
        harness.state().digest
    );

    let label = harness.state().catalog.text("drawings-export-pdf");
    harness.get_by_label(&label).click();
    harness.run();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    assert_eq!(
        script
            .export_requests()
            .iter()
            .map(|request| request.extension.as_str())
            .collect::<Vec<_>>(),
        ["dxf", "pdf"]
    );

    let app = harness.state();
    assert_eq!(
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.is_dirty(),
        ),
        state,
        "exporting changes nothing in the document"
    );
}
