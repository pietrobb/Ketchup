use crate::*;
use egui_kittest::kittest::Queryable as _;
use ketchup_model::document::InstancePath;

use crate::live_bridge::measurement::tests::integration_support;

#[test]
fn sheet_annotations_gui_places_datum_and_undoes() {
    let _turn = integration_support::file_turn();
    let mut app = KetchupApp::new();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let root = app.document.current().occurrences().next().unwrap().id();
    app.clear_selection();
    app.selection.occurrences.insert(InstancePath::root(root));
    assert!(app.preview_selection_drawing());
    assert!(app.confirm_assembly_preview());
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let id = app.document.current().drawing_sheets().next().unwrap().id();
    app.dispatch_file_command(AppCommand::ExportProjectDrawings);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1800.0, 1600.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(2);
    harness.get_by_label("Annotate sheet").click();
    harness.run_steps(2);
    harness.get_by_label("Datum").click();
    harness.run_steps(2);
    harness.get_by_label("Place annotation").click();
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .document
            .current()
            .drawing_sheet(id)
            .unwrap()
            .datum_symbols()
            .len(),
        1
    );
    // Editing the sheet can start an exact refresh; replace it with one awaited on its
    // result channel instead of polling frames.
    drop(harness.state_mut().exact.task.take());
    crate::drawn_shape::tests::evaluate_exact(harness.state_mut());
    harness.run_steps(1);
    let drawing = harness.state().drawing_sheet_geometry(id).unwrap();
    assert_eq!(drawing.layout.datum_symbols[0].label, "A");
    assert!(!drawing.views[0].visible_lines.is_empty());
    harness.state_mut().dispatch_command(AppCommand::Undo);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .document
            .current()
            .drawing_sheet(id)
            .unwrap()
            .datum_symbols()
            .is_empty()
    );
}
