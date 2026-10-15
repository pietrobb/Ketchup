//! An AI client brings files into the window and writes the model out over the bridge.

use super::*;

const TETRAHEDRON: &[u8] = b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 10 0\n   vertex 10 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 10 0 0\n   vertex 0 0 10\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 10\n   vertex 0 10 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 10 0 0\n   vertex 0 10 0\n   vertex 0 0 10\n  endloop\n endfacet\n\
endsolid tetrahedron\n";

/// A DXF without a header, so it declares no length unit.
const UNITLESS_DXF: &[u8] = b"0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nclosed\n90\n3\n70\n1\n\
10\n20\n20\n0\n10\n30\n20\n0\n10\n20\n20\n10\n\
0\nENDSEC\n0\nEOF\n";

/// A window whose dialogs decline every prompt: a client export never asks.
fn window() -> (KetchupApp, LiveBridge, tempfile::TempDir) {
    let (app, bridge) = setup();
    (app, bridge, tempfile::tempdir().unwrap())
}

fn import(path: &Path, unit: Option<&str>) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: unit.map(str::to_owned),
    }
}

fn export(path: &Path, export_format: &str) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: export_format.to_owned(),
    }
}

#[test]
fn an_imported_stl_is_one_undo_step_in_the_declared_unit() {
    let (mut app, mut bridge, directory) = window();
    let source = directory.path().join("tetrahedron.stl");
    std::fs::write(&source, TETRAHEDRON).unwrap();
    let occurrences = app.document.current().occurrences().count();
    let undo_steps = app.undo_step_count();

    let imported = bridge
        .execute(&mut app, import(&source, Some("cm")), false)
        .unwrap();
    assert_eq!(imported["review"]["triangles"], 4);
    assert_eq!(imported["review"]["unit"], "cm");
    assert_eq!(
        app.document.current().occurrences().count(),
        occurrences + 1
    );
    assert_eq!(app.undo_step_count(), undo_steps + 1);
}

/// SEC-5 (review 2026-10-15): an attached client reads a local file into the
/// model only after the user allows that exact path, as it opens one.
#[test]
fn an_attached_client_imports_a_file_only_after_the_user_allows_its_path() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tetrahedron.stl");
    std::fs::write(&source, TETRAHEDRON).unwrap();
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(7);
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.live.consent_attached = true;
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    let before = app.document_snapshot().canonical_digest();

    assert_eq!(
        bridge.execute(&mut app, import(&source, Some("mm")), false),
        Err("import_declined")
    );
    assert_eq!(app.document_snapshot().canonical_digest(), before);
    let prompts = probe.high_risk_prompts();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].contains(&source.display().to_string()),
        "{prompts:?}"
    );

    let imported = bridge
        .execute(&mut app, import(&source, Some("mm")), false)
        .unwrap();
    assert_eq!(imported["review"]["triangles"], 4);
    assert_eq!(probe.high_risk_prompts().len(), 2);
}

#[test]
fn a_large_import_review_fits_the_answer_of_the_import_that_already_happened() {
    let (mut app, mut bridge, directory) = window();
    let source = directory.path().join("layers.dxf");
    let mut dxf = String::from("0\nSECTION\n2\nENTITIES\n");
    // Long layer names: without a bound the review alone outgrows the check below.
    for index in 0..160 {
        let layer = format!("{index:03}-{}", "layer".repeat(50));
        let x = index * 20;
        dxf.push_str(&format!(
            "0\nLWPOLYLINE\n8\n{layer}\n90\n3\n70\n1\n10\n{x}\n20\n0\n10\n{}\n20\n0\n10\n{x}\n20\n10\n",
            x + 10
        ));
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    std::fs::write(&source, dxf).unwrap();
    let undo_steps = app.undo_step_count();

    let imported = bridge
        .execute(&mut app, import(&source, Some("mm")), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let review = &imported["review"];
    assert_eq!(review["profiles"], 160);
    assert_eq!(review["layers_total"], 160);
    assert_eq!(review["layers"].as_array().unwrap().len(), 20);
    assert!(
        review["diagnostics"].as_array().unwrap().len()
            <= review["diagnostics_total"].as_u64().unwrap() as usize
    );
    let bytes = serde_json::to_vec(&imported).unwrap().len();
    assert!(bytes < MAX_RESPONSE_FRAME_BYTES / 16, "{bytes}");
    assert_eq!(app.undo_step_count(), undo_steps + 1);
}

#[test]
fn a_dxf_without_units_needs_a_unit_and_nothing_changes_until_it_has_one() {
    let (mut app, mut bridge, directory) = window();
    let source = directory.path().join("plan.dxf");
    std::fs::write(&source, UNITLESS_DXF).unwrap();
    let before = app.canonical_digest();

    assert_eq!(
        bridge.execute(&mut app, import(&source, None), false),
        Err("import_failed")
    );
    assert_eq!(app.canonical_digest(), before);

    let imported = bridge
        .execute(&mut app, import(&source, Some("mm")), false)
        .unwrap();
    assert_eq!(imported["review"]["profiles"], 1);
    assert_eq!(imported["review"]["unit_from_file"], false);
    assert_ne!(app.canonical_digest(), before);

    let target = directory.path().join("profiles.dxf");
    let exported = bridge
        .execute(&mut app, export(&target, "dxf"), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let written = std::fs::read_to_string(&target).unwrap();
    assert!(
        written.contains("LWPOLYLINE") || written.contains("LINE"),
        "{written}"
    );
    assert!(written.trim_end().ends_with("EOF"), "{written}");
    assert_eq!(
        exported["loss_report"],
        target.with_extension("dxf.loss.txt").display().to_string()
    );

    // An existing file is never replaced from outside the window.
    std::fs::write(&target, b"user drawing").unwrap();
    assert_eq!(
        bridge.execute(&mut app, export(&target, "dxf"), false),
        Err("export_failed")
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"user drawing");
}

#[test]
fn unknown_formats_and_mismatched_extensions_write_and_change_nothing() {
    let (mut app, mut bridge, directory) = window();
    let mesh = directory.path().join("part.obj");
    std::fs::write(&mesh, b"v 0 0 0\n").unwrap();
    let before = app.canonical_digest();

    assert_eq!(
        bridge.execute(&mut app, import(&mesh, None), false),
        Err("unsupported_format")
    );
    let wrong = directory.path().join("model.stl");
    assert_eq!(
        bridge.execute(&mut app, export(&wrong, "step"), false),
        Err("invalid_path")
    );
    assert_eq!(
        bridge.execute(&mut app, export(&wrong, "obj"), false),
        Err("unsupported_format")
    );
    assert!(!wrong.exists());
    assert_eq!(app.canonical_digest(), before);
}
