//! The Layers panel counts what an underlay's tags show: the lines and texts of the whole
//! drawing on its own tag and of one layer on each layer's tag, never "0 occurrences".

use super::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

/// Layer `walls`: four lines and one text; layer `far`: one line. Millimetres.
fn drawing() -> String {
    let line = |layer: &str, [x0, y0, x1, y1]: [i32; 4]| {
        format!("0\nLINE\n8\n{layer}\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n")
    };
    let layer = |name: &str| format!("0\nLAYER\n2\n{name}\n70\n0\n62\n7\n6\nCONTINUOUS\n");
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n{}{}0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{}{}{}{}{}\
         0\nTEXT\n8\nwalls\n10\n500\n20\n500\n40\n250\n1\nKitchen\n0\nENDSEC\n0\nEOF\n",
        layer("walls"),
        layer("far"),
        line("walls", [0, 0, 4_000, 0]),
        line("walls", [4_000, 0, 4_000, 3_000]),
        line("walls", [4_000, 3_000, 0, 3_000]),
        line("walls", [0, 3_000, 0, 0]),
        line("far", [10_000, 0, 11_000, 0]),
    )
}

#[test]
fn layers_panel_counts_the_lines_and_texts_of_an_underlays_tags() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    std::fs::write(&path, drawing()).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.document = DocumentStore::new();
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    assert_eq!(
        app.document.current().underlays().count(),
        1,
        "{}",
        app.action_digest()
    );

    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    for row in [
        "house (5 lines, 1 text)",
        "house · walls (4 lines, 1 text)",
        "house · far (1 line)",
    ] {
        assert!(
            harness
                .query_by_role_and_label(Role::CheckBox, row)
                .is_some(),
            "no Layers row {row:?}"
        );
    }
    assert!(harness.query_by_label("house (0 occurrences)").is_none());

    // The row still turns the layer off, and its count stays while it is hidden.
    harness
        .get_by_role_and_label(Role::CheckBox, "house · far (1 line)")
        .click();
    harness.run_steps(2);
    let far = harness
        .state()
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "house · far")
        .map(|tag| tag.visible());
    assert_eq!(far, Some(false));
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, "house (5 lines, 1 text)")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, "house · far (1 line)")
            .is_some()
    );

    // The Slovak row names the same counts.
    let slovak = LocaleCatalog::slovak();
    let counts =
        |key: &str, count: &str| slovak.format(key, &BTreeMap::from([("count", count.to_owned())]));
    assert_eq!(
        slovak.format(
            "tags-row-underlay",
            &BTreeMap::from([
                ("name", "dom".to_owned()),
                (
                    "counts",
                    [
                        counts("tags-count-lines", "5"),
                        counts("tags-count-texts", "1")
                    ]
                    .join(", ")
                ),
            ])
        ),
        "dom (čiar: 5, textov: 1)"
    );
}
