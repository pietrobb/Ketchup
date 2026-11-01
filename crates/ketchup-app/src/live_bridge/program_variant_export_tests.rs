//! #2477 step 3: every variant's cut list and takeoff, written to new files
//! from a worker thread while the window's document stays as it is.
use super::super::tests::setup;
use super::*;

const CABINET: &str = "W = param(\"width\", 600)\nH = param(\"height\", 720)\n\
box(\"left\", (18, 400, H), material = \"oak\")\n\
box(\"right\", (18, 400, H), at = (W - 18, 0, 0), material = \"oak\")\n\
box(\"top\", (W - 36, 400, 18), at = (18, 0, H - 18), material = \"oak\")\n";

fn run(bridge: &mut LiveBridge, app: &mut KetchupApp, request: Request) -> Value {
    bridge
        .execute(app, request, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()))
}

/// The cabinet with variants 600 (600 × 720) and 1000 (1000 × 900), left at 800 × 720.
fn cabinet_with_variants() -> (KetchupApp, LiveBridge) {
    let (mut app, mut bridge) = setup();
    let apply = Request::ApplyProgram {
        expected: None,
        source: CABINET.to_owned(),
        overrides: None,
        file_name: Some("cabinet.star".into()),
        replace_document: true,
    };
    run(&mut bridge, &mut app, apply);
    for (name, width, height) in [("600", 600., 720.), ("1000", 1000., 900.), ("", 800., 720.)] {
        let params = BTreeMap::from([("width".to_owned(), width), ("height".to_owned(), height)]);
        let expected = app.live_bridge_stamp();
        run(
            &mut bridge,
            &mut app,
            Request::SetProgramParams { expected, params },
        );
        if !name.is_empty() {
            let (expected, name) = (app.live_bridge_stamp(), name.to_owned());
            run(
                &mut bridge,
                &mut app,
                Request::SaveProgramVariant { expected, name },
            );
        }
    }
    (app, bridge)
}

fn export(app: &KetchupApp, path: &Path) -> Request {
    Request::ExportProgramVariants {
        expected: app.live_bridge_stamp(),
        path: path.display().to_string(),
        decimal: CsvDecimal::Point,
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .replace('\u{feff}', "")
}

#[test]
fn every_variant_is_exported_from_a_worker_and_the_document_stays_as_it_is() {
    let (mut app, mut bridge) = cabinet_with_variants();
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("cabinet.csv");
    let (stamp, undo) = (app.live_bridge_stamp(), app.undo_step_count());
    let program = app.document.current_rule_program().cloned();

    let context = egui::Context::default();
    let (reply, response) = mpsc::sync_channel(1);
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    let request = export(&app, &target);
    bridge.start_queued_variant_export(&app, &context, queued, request);
    // The UI thread only started the job; the worker evaluates and writes.
    assert!(bridge.program_check_job.is_some());
    let deadline = Instant::now() + Duration::from_secs(30);
    let answer = loop {
        bridge.poll_program_check_job(&mut app, &context);
        if let Ok(answer) = response.try_recv() {
            break answer;
        }
        assert!(Instant::now() < deadline, "the export did not finish");
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert!(answer.ok, "{answer:?}");
    let result = answer.result.unwrap();
    assert_eq!(result["exported"], 2, "{result}");
    assert_eq!(result["failed"], 0, "{result}");
    assert_eq!(result["variants"][0]["name"], "1000", "{result}");
    assert_eq!(result["variants"][0]["params"]["height"], 900.0, "{result}");
    assert_eq!(result["variants"][1]["parts"], 3, "{result}");

    // Height and the top's length follow each variant, not the window's 800 × 720.
    // The test window speaks English: decimal point, columns split on commas.
    let cut_list = read(&target);
    let lines = cut_list.lines().collect::<Vec<_>>();
    assert_eq!(
        lines,
        [
            "variant,position,parts,material,length_mm,width_mm,thickness_mm,count",
            "1000,1,top,oak,964,400,18,1",
            "1000,2,\"left, right\",oak,900,400,18,2",
            "600,1,\"left, right\",oak,720,400,18,2",
            "600,2,top,oak,564,400,18,1",
        ],
        "{cut_list}"
    );
    let takeoff = read(&folder.path().join("cabinet.takeoff.csv"));
    assert!(
        takeoff.starts_with("variant,category,material,width_mm,thickness_mm,count,"),
        "{takeoff}"
    );
    let totals = takeoff
        .lines()
        .filter(|line| line.contains(",TOTAL,oak,"))
        .map(|line| line.split(',').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(totals, ["1000", "600"], "{takeoff}");

    assert_eq!(
        app.live_bridge_stamp(),
        stamp,
        "the export publishes nothing"
    );
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(app.document.current_rule_program().cloned(), program);
}

/// UI-9 (review 2026-10-17): a client names how CSV numbers are written
/// (`decimal`), a point unless it asks; the window's language plays no part.
#[test]
fn a_variant_export_writes_numbers_as_the_request_names_them() {
    let (mut app, mut bridge) = cabinet_with_variants();
    app.set_ui_language(crate::language::UiLanguage::Slovak);
    let folder = tempfile::tempdir().unwrap();
    for (decimal, name, header) in [
        (None, "plain.csv", "variant,position,"),
        (Some("comma"), "comma.csv", "variant;position;"),
        (Some("point"), "point.csv", "variant,position,"),
    ] {
        let path = folder.path().join(name);
        let mut request = json!({
            "method": "export_program_variants",
            "expected": app.live_bridge_stamp(),
            "path": path,
        });
        if let Some(decimal) = decimal {
            request["decimal"] = json!(decimal);
        }
        let request: Request = serde_json::from_value(request).unwrap();
        run(&mut bridge, &mut app, request);
        let cut_list = read(&path);
        assert!(cut_list.starts_with(header), "{decimal:?}\n{cut_list}");
    }
}

#[test]
fn an_export_never_replaces_a_file_and_needs_variants() {
    let (mut app, mut bridge) = cabinet_with_variants();
    let folder = tempfile::tempdir().unwrap();
    let takeoff = folder.path().join("cabinet.takeoff.xlsx");
    std::fs::write(&takeoff, b"user file").unwrap();
    let target = folder.path().join("cabinet.xlsx");
    let request = export(&app, &target);
    assert_eq!(
        bridge.execute(&mut app, request, false),
        Err("export_target_exists")
    );
    assert_eq!(std::fs::read(&takeoff).unwrap(), b"user file");
    assert!(!target.exists(), "neither table is written");

    for path in [
        folder.path().join("cabinet.pdf"),
        PathBuf::from("cabinet.csv"),
    ] {
        let request = export(&app, &path);
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err("invalid_path")
        );
    }

    let written = folder.path().join("both.xlsx");
    let request = export(&app, &written);
    let result = run(&mut bridge, &mut app, request);
    assert_eq!(result["exported"], 2, "{result}");
    assert!(
        std::fs::read(&written).unwrap().starts_with(b"PK"),
        "an XLSX workbook"
    );
    assert!(folder.path().join("both.takeoff.xlsx").exists());

    for name in ["1000", "600"] {
        let (expected, name) = (app.live_bridge_stamp(), name.to_owned());
        run(
            &mut bridge,
            &mut app,
            Request::DeleteProgramVariant { expected, name },
        );
    }
    let request = export(&app, &folder.path().join("none.csv"));
    assert_eq!(
        bridge.execute(&mut app, request, false),
        Err("invalid_params")
    );
    let reason = take_error_details().unwrap().to_string();
    assert!(reason.contains("save_variant"), "{reason}");
}
