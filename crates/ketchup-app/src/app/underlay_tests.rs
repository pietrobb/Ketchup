//! File > Import DXF/DWG as underlay: the drawing is kept under the model as one Undo
//! step, every drawn layer shows and hides by its own tag, and the whole drawing by one
//! more.
use crate::app::dwg_import::LAYERED_DRAWING_DXF;
use crate::*;

/// The vertex count each underlay of the frame draws.
fn drawn(app: &mut KetchupApp) -> Vec<u64> {
    app.underlay_draws()
        .iter()
        .map(|draw| {
            draw.visible
                .iter()
                .map(|&layer| draw.geometry.layer_vertices(layer))
                .sum()
        })
        .collect()
}

#[test]
fn an_underlay_imports_in_one_undo_step_and_its_tags_hide_the_drawing_or_one_layer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    std::fs::write(&path, LAYERED_DRAWING_DXF).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let (undo, occurrences) = (
        app.undo_step_count(),
        app.document.current().occurrences().count(),
    );

    // Without a frame the drawing is read here, in the program worker.
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);

    let snapshot = app.document.current();
    let underlay = snapshot
        .underlays()
        .next()
        .unwrap_or_else(|| panic!("no underlay: {}", app.action_digest()))
        .clone();
    assert_eq!(app.undo_step_count(), undo + 1);
    assert_eq!(snapshot.occurrences().count(), occurrences, "no part");
    let layers = underlay
        .layers
        .iter()
        .map(|layer| layer.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(layers.len(), 3, "{layers:?}");
    let all = drawn(&mut app);
    assert!(all[0] > 0);

    let outline = underlay.layers.iter().find(|layer| layer.name == "outline");
    assert!(app.set_tag_visibility(outline.unwrap().tag, false));
    let without_outline = drawn(&mut app);
    assert!(without_outline[0] > 0 && without_outline[0] < all[0]);
    assert!(app.set_tag_visibility(underlay.tag, false));
    assert_eq!(drawn(&mut app), vec![0]);
    assert!(app.set_tag_visibility(underlay.tag, true));
    assert_eq!(drawn(&mut app), without_outline);

    // Undo of the three visibility changes, then of the import, removes the drawing.
    for _ in 0..4 {
        assert!(app.undo());
    }
    assert_eq!(app.document.current().underlays().count(), 0);
    assert!(drawn(&mut app).is_empty());
}

#[test]
fn an_underlay_draws_its_texts_on_their_layer_which_hides_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("labelled.dxf");
    std::fs::write(
        &path,
        concat!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
            "0\nSECTION\n2\nENTITIES\n",
            "0\nLINE\n8\nwalls\n10\n0\n20\n0\n11\n4000\n21\n0\n",
            "0\nTEXT\n8\nlabels\n10\n500\n20\n500\n40\n250\n1\nKitchen\n",
            "0\nENDSEC\n0\nEOF\n",
        ),
    )
    .unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let underlay = app
        .document
        .current()
        .underlays()
        .next()
        .unwrap_or_else(|| panic!("no underlay: {}", app.action_digest()))
        .clone();
    assert!(
        app.action_digest().contains("1 texts"),
        "{}",
        app.action_digest()
    );

    // The wall is two vertices; the letters of "Kitchen" are many more.
    let all = drawn(&mut app);
    assert!(all[0] > 100, "{all:?}");
    let labels = underlay.layers.iter().find(|layer| layer.name == "labels");
    assert!(app.set_tag_visibility(labels.unwrap().tag, false));
    assert_eq!(drawn(&mut app), vec![2]);
}

#[test]
fn removing_an_underlay_is_one_undo_step_that_takes_its_tags_along() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    std::fs::write(&path, LAYERED_DRAWING_DXF).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let tags = app.document.current().tags().count();
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let id = app.document.current().underlays().next().unwrap().id;
    assert_eq!(app.document.current().tags().count(), tags + 4);
    let undo = app.undo_step_count();

    app.remove_underlay(id).unwrap();

    assert_eq!(app.undo_step_count(), undo + 1);
    assert_eq!(app.document.current().underlays().count(), 0);
    assert_eq!(app.document.current().tags().count(), tags);
}
