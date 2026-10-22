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

/// Review 2026-10-21 P3: the layers the drawing turned off (negative color) or froze
/// (flag 1) showed, and gave snaps, in the underlay. They come in with their tags hidden,
/// so they are kept and can be shown.
#[test]
fn layers_the_drawing_turned_off_or_froze_come_in_hidden() {
    let line =
        |layer: &str, y: u32| format!("0\nLINE\n8\n{layer}\n10\n0\n20\n{y}\n11\n100\n21\n{y}\n");
    let layer = |name: &str, color: i32, flags: u32| {
        format!("0\nLAYER\n2\n{name}\n70\n{flags}\n62\n{color}\n6\nCONTINUOUS\n")
    };
    let dxf = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n{}{}{}0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{}{}{}0\nENDSEC\n0\nEOF\n",
        layer("shown", 1, 0),
        layer("off", -3, 0),
        layer("frozen", 5, 1),
        line("shown", 0),
        line("off", 50),
        line("frozen", 100),
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    std::fs::write(&path, dxf).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let snapshot = app.document.current();
    let underlay = snapshot
        .underlays()
        .next()
        .unwrap_or_else(|| panic!("no underlay: {}", app.action_digest()))
        .clone();
    let visible = underlay
        .layers
        .iter()
        .map(|layer| {
            let tag = snapshot.tag(layer.tag).unwrap();
            (layer.name.as_str(), tag.visible())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        [("shown", true), ("off", false), ("frozen", false)],
        "{}",
        app.action_digest()
    );
    let shown_only = drawn(&mut app);
    let frozen = underlay.layers.iter().find(|layer| layer.name == "frozen");
    assert!(app.set_tag_visibility(frozen.unwrap().tag, true));
    assert!(drawn(&mut app)[0] > shown_only[0]);
}

/// Capability review of the fix above: a block placed on a frozen layer showed, and gave
/// snaps, when what it holds is on a shown layer. Freezing the layer of a block's
/// placement hides all of it, its nested blocks and attributes too; it comes in on that
/// layer, hidden. Turning a layer off hides only what the block holds on layer 0.
#[test]
fn a_block_placed_on_a_frozen_layer_comes_in_hidden_with_that_layer() {
    let line =
        |layer: &str, y: u32| format!("0\nLINE\n8\n{layer}\n10\n0\n20\n{y}\n11\n100\n21\n{y}\n");
    let layer = |name: &str, color: i32, flags: u32| {
        format!("0\nLAYER\n2\n{name}\n70\n{flags}\n62\n{color}\n6\nCONTINUOUS\n")
    };
    let block = |name: &str, entities: &str| {
        format!("0\nBLOCK\n8\n0\n2\n{name}\n70\n0\n10\n0\n20\n0\n{entities}0\nENDBLK\n")
    };
    let insert = |name: &str, layer: &str, x: u32| {
        format!("0\nINSERT\n8\n{layer}\n2\n{name}\n10\n{x}\n20\n0\n")
    };
    let read = |entities: &str| {
        let dxf = format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
             0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n{}{}{}0\nENDTAB\n0\nENDSEC\n\
             0\nSECTION\n2\nBLOCKS\n{}{}0\nENDSEC\n\
             0\nSECTION\n2\nENTITIES\n{}{entities}0\nENDSEC\n0\nEOF\n",
            layer("WALL", 1, 0),
            layer("FURN", 5, 1),
            layer("OFF", -3, 0),
            block("B", &line("WALL", 200)),
            block("NEST", &insert("B", "FURN", 1000)),
            line("WALL", 0),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan.dxf");
        std::fs::write(&path, dxf).unwrap();
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
        (app, underlay)
    };
    let (mut walls, _) = read("");
    let walls = drawn(&mut walls);
    let (mut app, underlay) = read(&format!(
        "{}{}{}",
        // B on the frozen layer, with an attribute on a shown one; NEST, on a shown
        // layer, places B on the frozen layer; B on a layer turned off still shows.
        "0\nINSERT\n8\nFURN\n2\nB\n10\n0\n20\n0\n66\n1\n\
         0\nATTRIB\n8\nWALL\n2\nTAG\n1\nchair\n10\n0\n20\n300\n40\n50\n0\nSEQEND\n",
        insert("NEST", "WALL", 0),
        insert("B", "OFF", 3000),
    ));
    let with_off = drawn(&mut app)[0];
    let off = underlay.layers.iter().find(|layer| layer.name == "OFF");
    if let Some(off) = off {
        assert!(app.set_tag_visibility(off.tag, true));
        assert_eq!(
            drawn(&mut app)[0],
            with_off,
            "B's line is on WALL, never off"
        );
    }
    let (mut only_off, _) = read(&insert("B", "OFF", 3000));
    assert_eq!(
        with_off,
        drawn(&mut only_off)[0],
        "nothing of a block on a frozen layer shows: {walls:?}, {}",
        app.action_digest()
    );
    assert!(with_off > walls[0], "a block on a layer turned off shows");
    let furn = underlay.layers.iter().find(|layer| layer.name == "FURN");
    assert!(app.set_tag_visibility(furn.expect("FURN layer").tag, true));
    assert!(drawn(&mut app)[0] > with_off, "shown once FURN is shown");
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

    // The wall is one quad of six vertices; the letters of "Kitchen" are many more.
    let all = drawn(&mut app);
    assert!(all[0] > 100, "{all:?}");
    let labels = underlay.layers.iter().find(|layer| layer.name == "labels");
    assert!(app.set_tag_visibility(labels.unwrap().tag, false));
    assert_eq!(drawn(&mut app), vec![6]);
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
