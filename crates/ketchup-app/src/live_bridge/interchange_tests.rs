//! An AI client brings files into the window and writes the model out over the bridge.

use super::*;
use crate::live_bridge::drawings::ExportDrawingsRequest;

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
        as_underlay: false,
        revision_of: None,
        code_page: None,
    }
}

fn export(path: &Path, export_format: &str) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: export_format.to_owned(),
        drawing_scale: None,
    }
}

/// The entities of the ENTITIES section of a DXF text, each as its type and its
/// trimmed group pairs.
fn dxf_entities(dxf: &str) -> Vec<(String, Vec<(i32, String)>)> {
    let lines = dxf.lines().map(str::trim).collect::<Vec<_>>();
    let pairs = lines
        .chunks(2)
        .map(|pair| (pair[0].parse::<i32>().unwrap(), pair[1].to_owned()))
        .collect::<Vec<_>>();
    let start = pairs
        .windows(2)
        .position(|window| {
            window[0] == (0, "SECTION".into()) && window[1] == (2, "ENTITIES".into())
        })
        .expect("an ENTITIES section");
    let mut entities: Vec<(String, Vec<(i32, String)>)> = Vec::new();
    for (code, value) in &pairs[start + 2..] {
        match (code, value.as_str()) {
            (0, "ENDSEC") => break,
            (0, kind) => entities.push((kind.to_owned(), Vec::new())),
            _ => entities.last_mut().unwrap().1.push((*code, value.clone())),
        }
    }
    entities
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

/// The client is a local process with the user's rights that can read the file
/// itself, so an attached client imports without a prompt in the window.
#[test]
fn an_attached_client_imports_a_file_without_a_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tetrahedron.stl");
    std::fs::write(&source, TETRAHEDRON).unwrap();
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.live.consent_attached = true;
    let mut bridge = transport::start(egui::Context::default()).unwrap();

    let imported = bridge
        .execute(&mut app, import(&source, Some("mm")), false)
        .unwrap();
    assert_eq!(imported["review"]["triangles"], 4);
    assert!(probe.high_risk_prompts().is_empty());
}

/// Ploopy STEP fix B: a STEP part that cannot be read is left out, the rest
/// imported, and the answer names the part and the solid left out of another
/// with their source entities.
#[test]
fn an_imported_step_names_the_parts_and_solids_it_left_out() {
    let (mut app, mut bridge, _directory) = window();
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    app.connect_exact_worker(worker).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-partly-invalid-assembly.step");
    let definitions = app.document.current().definitions().count();

    let imported = bridge
        .execute(&mut app, import(&source, None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let review = &imported["review"];
    assert_eq!(review["refused_total"], 3, "{review}");
    let refused = review["refused"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(refused[0].starts_with("Crossed prism: ") && refused[0].contains("#1077"));
    assert!(refused[1].starts_with("Wire frame: "), "{}", refused[1]);
    assert!(
        refused[2].starts_with("Block and figure eight: solid 2 (entity #1756) fails"),
        "{}",
        refused[2]
    );
    assert_eq!(review["solids"], 4);
    // Review 2026-11-01 EX-100: the answer and the window's status line say
    // the import is partial, not only the review a client may not read.
    assert_eq!(review["complete"], false, "{review}");
    assert_eq!(
        review["left_out"],
        json!({"parts": 2, "solids": 1, "placements": 3}),
        "{review}"
    );
    let status = app.action_digest();
    assert!(
        status.contains("2 parts and 1 solids") && status.contains("3 placements"),
        "{status}"
    );
    // ... and the model summary keeps saying it after the answer is gone.
    let summary = bridge.query.summary(&app.document.current());
    let partial = &summary["partial_imports"];
    assert_eq!(partial.as_array().map(Vec::len), Some(1), "{summary}");
    assert_eq!(partial[0]["format"], "STEP");
    assert_eq!(
        partial[0]["source"]["text"],
        "self-authored-partly-invalid-assembly.step"
    );
    assert_eq!(partial[0]["refused_total"], 3);
    let summarized = partial[0]["refused"].as_array().unwrap();
    assert_eq!(summarized.len(), refused.len());
    for (row, line) in summarized.iter().zip(&refused) {
        assert_eq!(row["count"], 1);
        assert!(line.starts_with(row["subject"]["text"].as_str().unwrap()));
    }
    assert_eq!(
        app.document.current().definitions().count(),
        definitions + 3
    );
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

/// Review 2026-11-08 (class of CD-72): a model the format cannot carry, and a
/// declined consent, were `export_failed` like a full disk, whose hint is
/// "export to another path". Each now has the code whose hint repairs it.
#[test]
fn an_export_refusal_is_named_by_what_repairs_it() {
    use crate::export_bundle::ExportError;
    use ketchup_manufacturing::fabrication::GeneralFabricationError;
    use ketchup_rejection::{Rejection, RejectionPhase};
    let code = crate::live_bridge::interchange::export_code;
    assert_eq!(
        code(&ExportError::NothingToExport {
            subject: "weldment"
        }),
        "export_refused"
    );
    assert_eq!(
        code(&ExportError::failed(
            GeneralFabricationError::PartGeometryUnavailable { part: "top".into() }
        )),
        "export_refused"
    );
    assert_eq!(
        code(&ExportError::from(Rejection::new(
            "side_effect.declined",
            RejectionPhase::Validation
        ))),
        "export_declined"
    );
    assert_eq!(
        code(&ExportError::Destination {
            requirement: "a .btlx path"
        }),
        "export_failed"
    );
    assert_eq!(code(&ExportError::ChangedDuringConsent), "export_failed");

    // Over the bridge: an empty window has no weldment to write.
    let (mut app, mut bridge, directory) = window();
    let target = directory.path().join("frame.btlx");
    assert_eq!(
        bridge.execute(&mut app, export(&target, "btlx"), false),
        Err("export_refused")
    );
    assert!(!target.exists());
    // Review 2026-11-08 §5.2: nor any shop file to send to the workshop.
    let package = directory.path().join("frame.zip");
    assert_eq!(
        bridge.execute(&mut app, export(&package, "workshop_package"), false),
        Err("export_refused")
    );
    assert!(!package.exists());
}

/// Měchenice 2026-11-02 MECH-9: `file export dxf/dwg` failed whole on one
/// upright pipe. The plan is written without it and the answer names it.
#[test]
fn a_plan_export_writes_the_flat_parts_and_names_the_part_it_left_out() {
    let (mut app, mut bridge, directory) = window();
    bridge
        .execute(
            &mut app,
            super::program_access::apply_source(
                "box('floor', (3000, 40, 5))\n\
                 pipe = extrude('pipe', profile=[(-50, -50), (50, -50), (50, 50), (-50, 50)], distance=700, at=(1000, 1000, 0))\n\
                 rotate(pipe, axis=(1, 0, 0), angle=90, pivot=(1000, 1000, 0))\n",
            ),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    for format in ["dxf", "dwg"] {
        let target = directory.path().join(format!("plan.{format}"));
        let exported = bridge
            .execute(&mut app, export(&target, format), false)
            .unwrap_or_else(|code| panic!("{format} {code}: {}", app.action_digest()));
        assert_eq!(exported["omitted_parts"], json!(["pipe"]), "{exported}");
        assert!(target.is_file());
        // The window's status line names it too, for a user exporting by hand.
        let status = app.action_digest();
        assert!(status.contains("pipe") && status.contains("XY"), "{status}");
        let report =
            std::fs::read_to_string(target.with_extension(format!("{format}.loss.txt"))).unwrap();
        assert!(
            report.lines().any(|line| line == "omitted_parts=pipe"),
            "{report}"
        );
    }
}

/// Review 2026-10-17 MCP-1: the answer guessed `<path>.<extension>.loss.txt`,
/// but a `.stp` export writes `x.step.loss.txt`, so a client was told nothing
/// was lost. The answer names the files the exporter wrote.
#[test]
fn a_stp_export_answers_with_the_loss_report_it_wrote() {
    let (mut app, mut bridge, directory) = window();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let target = directory.path().join("x.stp");
    let exported = bridge
        .execute(&mut app, export(&target, "step"), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let report = target.with_extension("step.loss.txt");
    assert!(report.is_file());
    assert_eq!(exported["loss_report"], report.display().to_string());
    assert_eq!(exported["companions"], json!([]));
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

/// A client exports the plan a program drew at the scale it names: the label is
/// 3.5 mm tall on paper, so 70 mm in the file at 1:20, and 350 mm at the default 1:100.
#[test]
fn a_plan_exports_its_annotations_at_the_drawing_scale_the_client_names() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "text('S-01', 'S-01', at=(1000, 2000), height_mm=3.5, tags='plan')\n\
                 dim('d', (0, 0), (1000, 0), offset_mm=-500, tags='plan')\n"
            .into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let height = |path: &Path| {
        let written = std::fs::read_to_string(path).unwrap();
        let lines = written.lines().collect::<Vec<_>>();
        // The label, after the dimension's own text in BLOCKS.
        let entities = lines.iter().position(|line| *line == "ENTITIES").unwrap();
        let text = entities
            + lines[entities..]
                .iter()
                .position(|line| *line == "TEXT")
                .unwrap();
        let at = lines[text..].iter().position(|line| *line == "40").unwrap();
        lines[text + at + 1].parse::<f64>().unwrap()
    };

    let scaled = directory.path().join("plan-20.dxf");
    let mut request = export(&scaled, "dxf");
    if let Request::Export { drawing_scale, .. } = &mut request {
        *drawing_scale = Some(20.0);
    }
    bridge
        .execute(&mut app, request, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(height(&scaled), 70.0);
    let report = std::fs::read_to_string(scaled.with_extension("dxf.loss.txt")).unwrap();
    assert!(report.contains("annotation_dimension_count=1"), "{report}");

    let default = directory.path().join("plan.dxf");
    bridge
        .execute(&mut app, export(&default, "dxf"), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(height(&default), 350.0);

    // The same plan as DWG keeps the dimension a DIMENSION and both on the tag's layer.
    let dwg = directory.path().join("plan-20.dwg");
    let mut request = export(&dwg, "dwg");
    if let Request::Export { drawing_scale, .. } = &mut request {
        *drawing_scale = Some(20.0);
    }
    bridge
        .execute(&mut app, request, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert!(dwg.with_extension("dwg.loss.txt").is_file());
    let read = ketchup_dwg::dwg_to_dxf(&std::fs::read(&dwg).unwrap(), 64 << 20).unwrap();
    let entities = dxf_entities(&String::from_utf8(read.dxf).unwrap());
    let group = |pairs: &[(i32, String)], code: i32| {
        pairs
            .iter()
            .find(|(found, _)| *found == code)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("no group {code} in {pairs:?}"))
    };
    let number = |pairs: &[(i32, String)], code: i32| group(pairs, code).parse::<f64>().unwrap();
    let of = |kind: &str| {
        entities
            .iter()
            .filter(|(found, _)| found == kind)
            .map(|(_, pairs)| pairs.as_slice())
            .collect::<Vec<_>>()
    };
    let (dimensions, texts) = (of("DIMENSION"), of("TEXT"));
    assert_eq!((dimensions.len(), texts.len()), (1, 1), "{entities:?}");
    let measured = (number(dimensions[0], 14) - number(dimensions[0], 13))
        .hypot(number(dimensions[0], 24) - number(dimensions[0], 23));
    assert!((measured - 1000.0).abs() <= 0.5, "{measured}");
    assert_eq!(
        [
            group(dimensions[0], 8),
            group(texts[0], 8),
            group(texts[0], 1)
        ],
        ["plan", "plan", "S-01"]
    );
    assert_eq!(number(texts[0], 40), 70.0);

    // A scale only a plan has, and only from 1:1.
    for (format, path, scale) in [("stl", "plan.stl", 20.0), ("dxf", "tiny.dxf", 0.5)] {
        let target = directory.path().join(path);
        let mut request = export(&target, format);
        if let Request::Export { drawing_scale, .. } = &mut request {
            *drawing_scale = Some(scale);
        }
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err("invalid_params")
        );
        assert!(!target.exists());
    }
}

/// A Czech project for the building office gets Czech sheets in a window of
/// another language (Měchenice MECH-5): the client names the sheet language
/// once and the document keeps it, `auto` gives the sheets back the window's
/// language, and a language without sheet texts is refused by name.
#[test]
fn the_sheets_are_titled_in_the_language_the_client_names_and_the_document_keeps_it() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "text('S-01', 'S-01', at=(0, 0), height_mm=3.5, tags='plan')\n".into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let drawings = |path: &Path, language: Option<&str>| {
        let mut request = json!({
            "method": "export_drawings",
            "path": path.display().to_string(),
            "plan_only": true,
            "drawing_scale": 250,
        });
        if let Some(language) = language {
            request["sheet_language"] = json!(language);
        }
        serde_json::from_value::<Request>(request).unwrap()
    };
    let texts = |app: &KetchupApp| {
        let set = app
            .project_drawings_job_for(crate::app::project_drawings::SheetRequest {
                plan_only: true,
                scale: Some(250),
            })
            .unwrap()
            .render(
                crate::app::project_drawings::SetFileFormat::Pdf,
                &AtomicBool::new(false),
            )
            .unwrap()
            .0;
        set.sheets[0]
            .page
            .texts()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let stored_language = |app: &KetchupApp| {
        serde_json::to_value(app.stored_sheet_settings().unwrap()).unwrap()["language"].clone()
    };
    let shows = |texts: &[String], wanted: &str| texts.iter().any(|text| text.contains(wanted));

    let czech = directory.path().join("czech.pdf");
    let written = bridge
        .execute(&mut app, drawings(&czech, Some("cs")), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["sheet_language"], "cs", "{written}");
    assert_eq!(stored_language(&app), "cs");
    let window_plan = app.catalog.text("drawings-sheet-plan");
    let czech_texts = texts(&app);
    for wanted in [
        "Půdorys",
        "řez ve výšce",
        "Měřítko",
        "Datum",
        "Projektová dokumentace",
    ] {
        assert!(
            shows(&czech_texts, wanted),
            "{wanted} not in {czech_texts:?}"
        );
    }
    assert!(!shows(&czech_texts, &window_plan), "{czech_texts:?}");

    // The document keeps the language for the next export.
    let again = directory.path().join("again.pdf");
    let written = bridge
        .execute(&mut app, drawings(&again, None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["sheet_language"], "cs", "{written}");

    // A language with no sheet texts is refused and changes nothing.
    let german = directory.path().join("german.pdf");
    assert_eq!(
        bridge.execute(&mut app, drawings(&german, Some("de")), false),
        Err("invalid_params")
    );
    assert!(!german.exists());
    assert_eq!(stored_language(&app), "cs");

    // auto: the window's language again, and the document keeps no language.
    let auto = directory.path().join("auto.pdf");
    let written = bridge
        .execute(&mut app, drawings(&auto, Some("auto")), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["sheet_language"], "auto", "{written}");
    assert_eq!(stored_language(&app), Value::Null);
    let window_texts = texts(&app);
    assert!(shows(&window_texts, &window_plan), "{window_texts:?}");
    assert!(!shows(&window_texts, "Půdorys"), "{window_texts:?}");
}

/// Review 2026-11-04 r19 P3: the height of ±0,000 could be set in the drawings
/// window only. A client names it in whole millimetres, the document keeps it
/// for the next export, `auto` finds it from the model again and says which
/// height it found (review 2026-11-06 P3), and a value that is no height is
/// refused and changes nothing.
#[test]
fn a_client_sets_the_height_of_zero_and_the_document_keeps_it() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "text('S-01', 'S-01', at=(0, 0), height_mm=3.5, tags='plan')\n\
                 box('slab', (6000, 3000, 200), at=(0, 0, 450))\n"
            .into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let drawings = |name: &str, zero: Option<Value>| {
        let mut request = json!({
            "method": "export_drawings",
            "path": directory.path().join(name).display().to_string(),
            "plan_only": true,
            "drawing_scale": 250,
        });
        if let Some(zero) = zero {
            request["zero_level_mm"] = zero;
        }
        serde_json::from_value::<Request>(request).unwrap()
    };
    let stored = |app: &KetchupApp| app.stored_sheet_settings().unwrap().zero_level_mm;

    let written = bridge
        .execute(&mut app, drawings("set.pdf", Some(json!(2800))), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["zero_level_mm"], 2800, "{written}");
    assert_eq!(written["zero_level_used_mm"], 2800.0, "{written}");
    assert_eq!(stored(&app), Some(2800));
    let written = bridge
        .execute(&mut app, drawings("again.pdf", None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["zero_level_mm"], 2800, "{written}");

    for refused in [json!(12.5), json!("high"), json!(3_000_000_000_i64)] {
        assert_eq!(
            bridge.execute(&mut app, drawings("refused.pdf", Some(refused)), false),
            Err("invalid_params")
        );
        assert!(!directory.path().join("refused.pdf").exists());
        assert_eq!(stored(&app), Some(2800));
    }

    let written = bridge
        .execute(&mut app, drawings("auto.pdf", Some(json!("auto"))), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["zero_level_mm"], "auto", "{written}");
    // The top of the slab, the ground floor.
    assert_eq!(written["zero_level_used_mm"], 650.0, "{written}");
    assert_eq!(stored(&app), None);
}

/// Review 2026-11-08 MF-241: the storeys were only ever found from the faces
/// of the model. A client gives the table of storeys and the altitude of
/// ±0,000; the document keeps them, the plans are cut above its floors and
/// the answer says where the storeys came from.
#[test]
fn a_client_gives_the_table_of_storeys_and_the_document_keeps_it() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('slab', (6000, 3000, 200))\n\
                 box('lining', (6000, 3000, 12), at=(0, 0, 2700))\n\
                 box('joists', (6000, 3000, 220), at=(0, 0, 2712))\n\
                 box('osb', (6000, 3000, 22), at=(0, 0, 2932))\n\
                 box('wall', (6000, 200, 2500), at=(0, 0, 200))\n\
                 box('upper wall', (6000, 200, 2500), at=(0, 0, 2954))\n"
            .into(),
        overrides: None,
        file_name: Some("tiny.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let drawings = |name: &str, storeys: Option<Value>, altitude: Option<Value>| {
        let mut request = json!({
            "method": "export_drawings",
            "path": directory.path().join(name).display().to_string(),
            "plan_only": true,
            "drawing_scale": 250,
        });
        if let Some(storeys) = storeys {
            request["storeys"] = storeys;
        }
        if let Some(altitude) = altitude {
            request["zero_altitude_mm"] = altitude;
        }
        serde_json::from_value::<Request>(request).unwrap()
    };
    let table = json!([
        {"finished_floor_mm": 200, "structural_mm": 200},
        {"finished_floor_mm": 2954, "structural_mm": 2932},
    ]);
    let written = bridge
        .execute(
            &mut app,
            drawings("set.pdf", Some(table.clone()), Some(json!(245_300))),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["storeys_from"], "table", "{written}");
    assert_eq!(written["storeys"], table, "{written}");
    assert_eq!(written["zero_altitude_mm"], 245_300, "{written}");
    assert_eq!(written["zero_level_used_mm"], 200.0, "{written}");
    assert_eq!(written["sheet_count"], 2, "{written}");
    let settings = app.stored_sheet_settings().unwrap();
    assert_eq!(settings.storeys.len(), 2);
    assert_eq!(settings.zero_altitude_mm, Some(245_300));

    for refused in [
        json!([]),
        json!([{"finished_floor_mm": 1.5, "structural_mm": 0}]),
        json!("high"),
    ] {
        assert_eq!(
            bridge.execute(
                &mut app,
                drawings("refused.pdf", Some(refused), None),
                false
            ),
            Err("invalid_params")
        );
        assert!(!directory.path().join("refused.pdf").exists());
    }
    assert_eq!(app.stored_sheet_settings().unwrap().storeys.len(), 2);

    // Review 2026-11-09 MF-300: 0/3500 against the floors at 200 and 2954
    // printed +3,300 and +3,500; the row the model contradicts is named.
    let contradicted = json!([
        {"finished_floor_mm": 200, "structural_mm": 200},
        {"finished_floor_mm": 3500, "structural_mm": 3500},
    ]);
    assert_eq!(
        bridge.execute(
            &mut app,
            drawings("contradicted.pdf", Some(contradicted), None),
            false
        ),
        Err("storey_table_contradicts_model")
    );
    assert!(!directory.path().join("contradicted.pdf").exists());
    assert_eq!(
        app.stored_sheet_settings().unwrap().storeys[1].finished_floor_mm,
        2954,
        "the document keeps its table"
    );

    let written = bridge
        .execute(
            &mut app,
            drawings("auto.pdf", Some(json!("auto")), Some(json!("none"))),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["storeys_from"], "model", "{written}");
    assert_eq!(written["storeys"], "auto", "{written}");
    assert_eq!(written["zero_altitude_mm"], "none", "{written}");
    assert!(app.stored_sheet_settings().unwrap().storeys.is_empty());
}

/// A client writes the plan alone as a PDF at the scale it names: the sheet holds
/// one view, the texts and the measured value of the dimension, and says what it drew.
#[test]
fn a_plan_of_its_own_is_drawn_as_a_pdf_at_the_scale_the_client_names() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "text('S-01', 'S-01', at=(0, 0), height_mm=3.5, tags='plan')\n\
                 text('S-02', 'S-02', at=(5000, 0), height_mm=3.5, tags='plan')\n\
                 dim('d', (0, 0), (5000, 0), offset_mm=-1000, tags='plan')\n"
            .into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let drawings =
        |path: &Path, plan_only: bool, drawing_scale: Option<f64>, format: Option<&str>| {
            Request::ExportDrawings(ExportDrawingsRequest {
                expected: None,
                path: path.display().to_string(),
                sheet_id: None,
                format: format.map(str::to_owned),
                title_block: BTreeMap::new(),
                plan_only,
                drawing_scale,
                sheet_language: None,
                replace: false,
                take_over: false,
                zero_level_mm: None,
                storeys: None,
                zero_altitude_mm: None,
                north_deg: None,
                revisions: None,
            })
        };

    let target = directory.path().join("plan.pdf");
    let written = bridge
        .execute(&mut app, drawings(&target, true, Some(250.0), None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["scale"], "1:250");
    assert_eq!(written["sheet_count"], 1, "{written}");
    assert_eq!(written["sheets"][0]["format"], "A3");
    assert_eq!(written["sheets"][0]["views"].as_array().unwrap().len(), 1);
    assert_eq!(written["sheets"][0]["views"][0]["view"], "plan");
    assert_eq!(written["annotations"]["texts"], 2);
    assert_eq!(written["annotations"]["dimensions"], 1);
    assert!(std::fs::read(&target).unwrap().starts_with(b"%PDF"));
    let set = app
        .project_drawings_job_for(crate::app::project_drawings::SheetRequest {
            plan_only: true,
            scale: Some(250),
        })
        .unwrap()
        .render(
            crate::app::project_drawings::SetFileFormat::Pdf,
            &AtomicBool::new(false),
        )
        .unwrap()
        .0;
    let texts = set.sheets[0].page.texts().collect::<Vec<_>>();
    for wanted in ["S-01", "S-02", "5000"] {
        assert!(texts.contains(&wanted), "{wanted} not in {texts:?}");
    }

    // Hidden, the annotations leave nothing to draw.
    let visibility = |visible: bool| Request::TagVisibility {
        expected: None,
        name: "plan".into(),
        visible,
    };
    bridge
        .execute(&mut app, visibility(false), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let hidden = directory.path().join("hidden.pdf");
    assert_eq!(
        bridge.execute(&mut app, drawings(&hidden, true, Some(250.0), None), false),
        Err("drawings_unavailable")
    );
    bridge
        .execute(&mut app, visibility(true), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));

    // Too fine for the format, a scale that is no whole 1:N, and a drawing sheet
    // with a plan option: nothing is written.
    for (path, scale, format, code) in [
        ("fine.pdf", 1.0, Some("A3"), "drawings_do_not_fit"),
        ("half.pdf", 2.5, None, "invalid_params"),
    ] {
        let target = directory.path().join(path);
        assert_eq!(
            bridge.execute(
                &mut app,
                drawings(&target, true, Some(scale), format),
                false
            ),
            Err(code)
        );
        assert!(!target.exists());
    }
    let mut sheet = drawings(&directory.path().join("sheet.dxf"), true, None, None);
    if let Request::ExportDrawings(ExportDrawingsRequest { sheet_id, .. }) = &mut sheet {
        *sheet_id = Some(1);
    }
    assert_eq!(
        bridge.execute(&mut app, sheet, false),
        Err("invalid_params")
    );
}

/// A letter the sheet's typeface lacks prints as "?": the client and the
/// window's digest name every such letter, from the plan and the title block,
/// so a sheet does not go to production with question marks nobody was told of.
#[test]
fn a_sheet_names_the_letters_it_prints_as_question_marks() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "text('S-01', 'S-01 漢', at=(0, 0), height_mm=3.5, tags='plan')\n".into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let target = directory.path().join("plan.pdf");
    let written = bridge
        .execute(
            &mut app,
            Request::ExportDrawings(ExportDrawingsRequest {
                expected: None,
                path: target.display().to_string(),
                sheet_id: None,
                format: None,
                title_block: BTreeMap::from([(
                    ketchup_manufacturing::title_block::TitleField::Project,
                    "Dom 東".to_owned(),
                )]),
                plan_only: true,
                drawing_scale: Some(250.0),
                sheet_language: None,
                replace: false,
                take_over: false,
                zero_level_mm: None,
                storeys: None,
                zero_altitude_mm: None,
                north_deg: None,
                revisions: None,
            }),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["printed_as_question_mark"], json!(["東", "漢"]));

    let window_sheet = directory.path().join("window.pdf");
    assert!(
        app.export_project_drawings_to(&window_sheet),
        "{}",
        app.action_digest()
    );
    let digest = app.action_digest();
    assert!(digest.contains("東") && digest.contains("漢"), "{digest}");
}

/// Review 2026-11-08 §5.2: north kept in the sheet settings gives each plan its
/// north arrow and names each elevation, in the sheet language, by the side of
/// the world it shows.
#[test]
fn north_in_the_sheet_settings_names_the_elevations_in_the_sheet_language() {
    use ketchup_manufacturing::project_drawings::{CompassSide, ProjectView};
    use ketchup_manufacturing::title_block::SheetLanguage;
    let (mut app, mut bridge) = setup();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('house', (6000, 3000, 2500))\n".into(),
        overrides: None,
        file_name: Some("house.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let mut settings = app.stored_sheet_settings().unwrap();
    settings.language = Some(SheetLanguage::Slovak);
    // North along -x: the long front, facing -y, faces west.
    settings.north_tenths_deg = Some(900);
    app.store_sheet_settings(settings);
    let set = app
        .project_drawings_job_for(crate::app::project_drawings::SheetRequest::default())
        .unwrap()
        .render(
            crate::app::project_drawings::SetFileFormat::Pdf,
            &AtomicBool::new(false),
        )
        .unwrap()
        .0;
    let views = set
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.views)
        .collect::<Vec<_>>();
    let front = views
        .iter()
        .find(|view| view.view == ProjectView::Front)
        .unwrap();
    assert_eq!(front.facing, Some(CompassSide::West));
    assert_eq!(front.title, "Pohľad západný");
    let plan = views
        .iter()
        .find(|view| view.view == ProjectView::Plan)
        .unwrap();
    assert!(plan.north_arrow, "{views:?}");
    assert!(
        set.sheets[0].page.texts().any(|text| text == "S"),
        "the Slovak north letter"
    );
}

/// Review 2026-11-08 §5.2: north could be set in the drawings window only. A
/// client names it in degrees counter-clockwise from the model's +y, the
/// document keeps it, `none` takes it away, and a value that is no direction is
/// refused and changes nothing.
#[test]
fn a_client_sets_north_and_the_document_keeps_it() {
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('house', (6000, 3000, 2500))\n".into(),
        overrides: None,
        file_name: Some("house.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let drawings = |name: &str, north: Option<Value>| {
        let mut request = json!({
            "method": "export_drawings",
            "path": directory.path().join(name).display().to_string(),
            "drawing_scale": 250,
        });
        if let Some(north) = north {
            request["north_deg"] = north;
        }
        serde_json::from_value::<Request>(request).unwrap()
    };
    let stored = |app: &KetchupApp| app.stored_sheet_settings().unwrap().north_tenths_deg;
    let front_facing = |written: &Value| {
        written["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|sheet| sheet["views"].as_array().unwrap())
            .find(|view| view["view"] == "front")
            .map(|view| view["facing"].clone())
    };

    let written = bridge
        .execute(&mut app, drawings("set.pdf", Some(json!(90))), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["north_deg"], 90.0, "{written}");
    assert_eq!(stored(&app), Some(900));
    assert_eq!(front_facing(&written), Some(json!("west")), "{written}");
    let written = bridge
        .execute(&mut app, drawings("again.pdf", None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["north_deg"], 90.0, "{written}");

    for refused in [json!(360.5), json!(-400), json!("up"), json!([0, 1])] {
        assert_eq!(
            bridge.execute(&mut app, drawings("refused.pdf", Some(refused)), false),
            Err("invalid_params")
        );
        assert!(!directory.path().join("refused.pdf").exists());
        assert_eq!(stored(&app), Some(900));
    }

    let written = bridge
        .execute(&mut app, drawings("none.pdf", Some(json!("none"))), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["north_deg"], "none", "{written}");
    assert_eq!(stored(&app), None);
    assert_eq!(front_facing(&written), Some(Value::Null), "{written}");
}

/// Review 2026-11-08 §5.3: a drawing set had no revisions. A client gives
/// them, oldest first; the document keeps them, every sheet lists them beside
/// its title block under the sheet language's headings, and more than the
/// table holds refuse the export and keep the document's.
#[test]
fn a_client_gives_the_revisions_every_sheet_lists() {
    use ketchup_manufacturing::title_block::SheetLanguage;
    let (mut app, mut bridge, directory) = window();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('house', (6000, 3000, 2500))\n".into(),
        overrides: None,
        file_name: Some("house.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let drawings = |name: &str, revisions: Value| {
        serde_json::from_value::<Request>(json!({
            "method": "export_drawings",
            "path": directory.path().join(name).display().to_string(),
            "drawing_scale": 250,
            "sheet_language": "cs",
            "revisions": revisions,
        }))
        .unwrap()
    };
    let revision = |index: &str, change: &str| json!({"index": index, "date": "8. 11. 2026", "change": change});
    let given = json!([
        revision("A", "Vydání pro stavební povolení"),
        revision("B", "Zvětšené okno v obývacím pokoji"),
    ]);
    let written = bridge
        .execute(&mut app, drawings("set.pdf", given.clone()), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["revisions"], given, "{written}");
    let stored = |app: &KetchupApp| app.stored_sheet_settings().unwrap().revisions;
    assert_eq!(stored(&app).len(), 2);
    assert_eq!(
        app.stored_sheet_settings().unwrap().language,
        Some(SheetLanguage::Czech)
    );
    let set = app
        .project_drawings_job_for(crate::app::project_drawings::SheetRequest::default())
        .unwrap()
        .render(
            crate::app::project_drawings::SetFileFormat::Pdf,
            &AtomicBool::new(false),
        )
        .unwrap()
        .0;
    for sheet in &set.sheets {
        let texts = sheet.page.texts().collect::<Vec<_>>();
        for text in ["Popis změny", "Datum", "Zvětšené okno v obývacím pokoji"] {
            assert!(
                texts.contains(&text),
                "{text} on sheet {}: {texts:?}",
                sheet.number
            );
        }
    }

    let crowded = Value::Array(
        (1..=12)
            .map(|index| revision(&index.to_string(), "Změna"))
            .collect(),
    );
    assert_eq!(
        bridge.execute(&mut app, drawings("crowded.pdf", crowded), false),
        Err("revisions_do_not_fit")
    );
    assert!(!directory.path().join("crowded.pdf").exists());
    assert_eq!(stored(&app).len(), 2, "the document keeps its revisions");
}
