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
        as_underlay: false,
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
            Request::ExportDrawings {
                expected: None,
                path: path.display().to_string(),
                sheet_id: None,
                format: format.map(str::to_owned),
                title_block: BTreeMap::new(),
                plan_only,
                drawing_scale,
            }
        };

    let target = directory.path().join("plan.pdf");
    let written = bridge
        .execute(&mut app, drawings(&target, true, Some(250.0), None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(written["scale"], "1:250");
    assert_eq!(written["format"], "A3");
    assert_eq!(written["views"].as_array().unwrap().len(), 1);
    assert_eq!(written["views"][0]["view"], "plan");
    assert_eq!(written["annotations"]["texts"], 2);
    assert_eq!(written["annotations"]["dimensions"], 1);
    assert!(std::fs::read(&target).unwrap().starts_with(b"%PDF"));
    let sheet = app
        .project_drawings_job_for(crate::app::project_drawings::SheetRequest {
            plan_only: true,
            scale: Some(250),
        })
        .unwrap()
        .render()
        .unwrap()
        .0;
    let texts = sheet.page.texts().collect::<Vec<_>>();
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
    if let Request::ExportDrawings { sheet_id, .. } = &mut sheet {
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
            Request::ExportDrawings {
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
            },
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
