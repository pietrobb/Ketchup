//! File > Drawings in the window: a drawing sheet of the document exports as
//! DXF or DWG from its own row, the project sheet still exports as PDF, and
//! none of them changes the document.
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::*;
use egui_kittest::kittest::Queryable as _;
use ketchup_model::document::InstancePath;

#[test]
fn the_drawings_window_exports_a_document_sheet_as_dxf_and_dwg_and_the_project_sheet_as_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let (dxf, dwg, pdf) = (
        directory.path().join("block.dxf"),
        directory.path().join("block.dwg"),
        directory.path().join("house.pdf"),
    );
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_export(&dxf)
        .queue_export(&dwg)
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
    let (_, dwg_bytes) = app.drawing_sheet_file(sheet, SheetFileFormat::Dwg).unwrap();
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

    let label = harness.state().catalog.text("drawings-export-sheet-dwg");
    harness.get_by_label(&label).click();
    harness.run();
    assert_eq!(std::fs::read(&dwg).unwrap(), dwg_bytes);
    assert!(dwg_bytes.starts_with(ketchup_dwg::WRITTEN_VERSION.as_bytes()));
    let exported = harness.state().catalog.format(
        "digest-exported-drawing-sheet-dwg",
        &BTreeMap::from([
            ("name", expected.name.clone()),
            ("path", dwg.display().to_string()),
        ]),
    );
    assert_eq!(harness.state().digest, exported);

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
        ["dxf", "dwg", "pdf"]
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

#[test]
fn a_sheet_titled_by_a_long_slovak_part_name_exports_as_dxf_and_dwg() {
    let note = "Smrekový hranol, brúsený, dvakrát olejovaný; dĺžka ±1 mm, šírka ±0,5 mm, \
                hrúbka ±0,5 mm, zaoblené hrany, tesné škáry, žiadne súky či trhliny";
    let mut app = KetchupApp::new();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let root = app.document.current().occurrences().next().unwrap().id();
    app.clear_selection();
    app.selection.occurrences.insert(InstancePath::root(root));
    app.begin_occurrence_rename();
    app.modal.get_mut::<PendingOccurrenceRename>().unwrap().name = note.to_owned();
    assert!(app.confirm_occurrence_rename(), "{}", app.action_digest());
    assert!(app.preview_selection_drawing(), "{}", app.action_digest());
    assert!(app.confirm_assembly_preview());
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheets().next().unwrap();
    let title = sheet.title_block().title().to_owned();
    assert!(title.contains(note), "{title}");
    // Every non-ASCII letter is a 7-character \U+XXXX escape in R12, so the
    // title the model accepted needs more than one 255-character string.
    let escaped: String = title
        .chars()
        .map(|character| {
            if character.is_ascii() {
                character.to_string()
            } else {
                format!("\\U+{:04X}", u32::from(character))
            }
        })
        .collect();
    assert!(escaped.len() > 255);

    let (_, dxf) = app
        .drawing_sheet_file(sheet.id(), SheetFileFormat::Dxf)
        .unwrap();
    let dxf = String::from_utf8(dxf).unwrap();
    assert!(dxf.lines().all(|line| line.chars().count() <= 255));
    let lines: Vec<&str> = dxf.lines().collect();
    let mut entity = "";
    let mut value = "";
    let mut title_pieces = Vec::new();
    for pair in lines.chunks(2) {
        let [code, data] = pair else { break };
        match code.trim() {
            "0" => entity = data,
            "1" if entity == "TEXT" => value = data,
            "1000" if entity == "TEXT" && *data == "title-block/title" => title_pieces.push(value),
            _ => {}
        }
    }
    assert!(title_pieces.len() > 1, "{title_pieces:?}");
    assert_eq!(title_pieces.concat(), escaped);

    let (_, dwg) = app
        .drawing_sheet_file(sheet.id(), SheetFileFormat::Dwg)
        .unwrap();
    assert!(dwg.starts_with(ketchup_dwg::WRITTEN_VERSION.as_bytes()));
}
