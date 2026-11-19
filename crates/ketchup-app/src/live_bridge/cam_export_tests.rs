//! A client writes the CAM program of the document's plan for the controller it names, told
//! what its simulation found; only replacing a file asks the user.

use super::*;
use ketchup_model::cam::{
    CamCutParameters, CamPlan, CamPlanId, CamPostprocessorDialect, CamSetup, CamStock, CamTool,
    CamToolKind, CamWorkOffset,
};
use ketchup_model::document::{
    CanonicalCommand, DefinitionId, Dimension, FeatureId, FeatureKind, OccurrenceId, Transform,
};

/// A window holding a 20 Ă— 10 Ă— 5 mm block and a facing plan for it, whose prompts answer as
/// `dialogs` says.
fn cam_window(dialogs: &ScriptedFileDialogs) -> (KetchupApp, LiveBridge, tempfile::TempDir) {
    cam_window_over(
        dialogs,
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [20.0, 10.0, 7.0],
        },
        None,
    )
}

/// The window of [`cam_window`] with its plan cutting `stock`, the block placed once and,
/// before the plan is made, tapped through its middle as `thread` (size, tap drill, thread depth) says.
fn cam_window_over(
    dialogs: &ScriptedFileDialogs,
    stock: CamStock,
    thread: Option<(&str, f64, f64)>,
) -> (KetchupApp, LiveBridge, tempfile::TempDir) {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs.clone()));
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    app.connect_exact_worker(worker).unwrap();
    let definition = DefinitionId(95);
    let profile = FeatureId(950);
    let solid = FeatureId(951);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "CAM target".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "20x10 profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "20x10x5 target".into(),
                kind: FeatureKind::extrusion(profile, Dimension::new("5", 5.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(952),
                definition_id: definition,
                name: "Block".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let mut bridge = transport::start(egui::Context::default()).unwrap();
    if let Some((size, tap_drill_mm, depth_mm)) = thread {
        super::program_edit::evaluate_exact(&mut app);
        let top = super::drill_holes::face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
        let tapped = super::drill_holes::edit(AssistantCadEditOperation::DrillHoles {
            definition_id: definition.0,
            name: "tapped hole".to_owned(),
            target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
            face_reference_id: top["reference_id"].as_str().unwrap().to_owned(),
            points_local_mm: vec![super::drill_holes::centroid(&top)],
            diameter_mm: tap_drill_mm,
            depth_mm: None,
            through: true,
            thread: Some(ketchup_assistant::sidecar::AssistantHoleThread {
                size: size.to_owned(),
                depth_mm,
                modeled_thread: None,
            }),
        });
        let applied = bridge.execute(&mut app, tapped, false).unwrap();
        assert_eq!(applied["published"], true, "{applied}");
    }
    let plan = CamPlan::new(
        &app.document.current(),
        CamPlanId(95),
        "Reviewed facing",
        stock,
        CamTool {
            number: 1,
            kind: CamToolKind::FlatEndMill,
            diameter_mm: 2.0,
            flute_length_mm: 5.0,
            overall_length_mm: 10.0,
            holder_diameter_mm: 6.0,
            holder_length_mm: 10.0,
            spindle_rpm: 10_000,
            feed_mm_per_min: 600.0,
            plunge_mm_per_min: 200.0,
            point_angle_deg: None,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 10.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        definition,
        solid,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
            plan,
        )]))
        .unwrap();
    (app, bridge, tempfile::tempdir().unwrap())
}

fn cam_export(path: &Path, export_format: &str, cam_dialect: Option<&str>) -> Request {
    cam_export_of(path, export_format, cam_dialect, None)
}

/// An export whose program runs `cam_operations`, faces of the part named by ordinal.
fn cam_export_of(
    path: &Path,
    export_format: &str,
    cam_dialect: Option<&str>,
    cam_operations: Option<Value>,
) -> Request {
    Request::Export(crate::live_bridge::interchange::ExportRequest {
        expected: None,
        path: path.display().to_string(),
        export_format: export_format.to_owned(),
        drawing_scale: None,
        image: None,
        cam_dialect: cam_dialect.map(str::to_owned),
        cam_operations: cam_operations
            .map(|operations| serde_json::from_value(operations).unwrap()),
        overwrite: false,
    })
}

/// The library's own description of the dialect `id`, as the window shows it.
fn dialect_name(id: &str) -> String {
    match CamPostprocessorDialect::library(id).unwrap() {
        CamPostprocessorDialect::GCode(dialect) => dialect.name().to_owned(),
        CamPostprocessorDialect::ControllerNeutralJson => unreachable!("{id} is G-code"),
    }
}

/// Review 2026-11-19 MF-807: no operation cuts a thread. A client's program names them in
/// `lost`, without a prompt (R1); the window's release prompt names them to the user.
#[test]
fn the_cam_program_names_the_threads_of_its_part_it_does_not_cut() {
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval(7);
    let (mut app, mut bridge, directory) = cam_window_over(
        &dialogs,
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [20.0, 10.0, 7.0],
        },
        Some(("M4", 3.3, 4.0)),
    );
    let target = directory.path().join("facing.ngc");
    let exported = bridge
        .execute(&mut app, cam_export(&target, "cam_nc", None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(
        exported["lost"],
        json!(["CAM target: thread M4×0.7 depth 4"]),
        "{exported}"
    );
    assert!(dialogs.high_risk_prompts().is_empty());

    let first = CamPostprocessorDialect::library_ids().unwrap()[0].clone();
    let written = app
        .export_current_cam_program_to(
            &directory.path().join("window.ngc"),
            CamPostprocessorDialect::library(&first).unwrap(),
            Vec::new(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(written.lost, ["CAM target: thread M4×0.7 depth 4"]);
    let prompts = dialogs.high_risk_prompts();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("M4×0.7 depth 4"), "{}", prompts[0]);
}

/// Owner decisions R1 and R3 (review 2026-11-19 og): a new program is written without a
/// prompt, and the reply names the controller, what the simulation found and what it does
/// not check.
#[test]
fn a_client_writes_the_cam_program_for_the_controller_it_names_without_a_prompt() {
    let dialogs = ScriptedFileDialogs::new();
    let (mut app, mut bridge, directory) = cam_window(&dialogs);
    let before = app.canonical_digest();

    let target = directory.path().join("facing.ngc");
    let exported = bridge
        .execute(
            &mut app,
            cam_export(&target, "cam_nc", Some("linuxcnc")),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["export_format"], "cam_nc");
    assert_eq!(exported["path"], target.display().to_string());
    let program = std::fs::read_to_string(&target).unwrap();
    let lines = program.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"%"), "{program}");
    assert!(program.contains("G43 H1"), "{program}");
    assert_eq!(lines[lines.len() - 2], "M2", "{program}");
    // The client is told which controller the program is for and what the simulation found:
    // facing the 7 mm stock to the 5 mm block removes 20 x 10 x 2 mm3 and gouges nothing.
    let machining = &exported["machining"];
    assert_eq!(machining["dialect"], dialect_name("linuxcnc"), "{exported}");
    assert!((machining["removed_stock_mm3"].as_f64().unwrap() - 400.0).abs() < 0.5);
    assert!(machining["gouge_mm3"].as_f64().unwrap() < 1.0e-5);
    assert_eq!(
        machining["not_checked"].as_array().unwrap().len(),
        crate::app::Machining::NOT_CHECKED.len()
    );
    assert!(dialogs.high_risk_prompts().is_empty());
    assert_eq!(app.canonical_digest(), before);

    // Without a dialect the program is the library's first one.
    let first = CamPostprocessorDialect::library_ids().unwrap()[0].clone();
    let default_target = directory.path().join("facing.nc");
    let exported = bridge
        .execute(&mut app, cam_export(&default_target, "cam_nc", None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["machining"]["dialect"], dialect_name(&first));

    // Without overwrite an existing file is kept.
    assert_eq!(
        bridge.execute(
            &mut app,
            cam_export(&target, "cam_nc", Some("linuxcnc")),
            false
        ),
        Err("export_failed")
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), program);
    assert!(dialogs.high_risk_prompts().is_empty());
}

/// Owner decision R1 (review 2026-11-19 og-6): with overwrite the program replaces an
/// existing file once the user agrees to the replacement, the only prompt; a declined
/// replacement, or a file changed while the user was asked, keeps the file.
#[test]
fn a_client_program_replaces_an_existing_file_only_once_the_user_agrees() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("facing.ngc");
    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval_after_write(7, &target, b"saved meanwhile".to_vec())
        .queue_high_risk_approval(7);
    let (mut app, mut bridge, _unused) = cam_window(&dialogs);
    let over = || {
        let mut request = cam_export(&target, "cam_nc", Some("linuxcnc"));
        if let Request::Export(crate::live_bridge::interchange::ExportRequest {
            overwrite, ..
        }) = &mut request
        {
            *overwrite = true;
        }
        request
    };

    // The replacement declined: the user's file is kept.
    std::fs::write(&target, b"user program").unwrap();
    assert_eq!(
        bridge.execute(&mut app, over(), false),
        Err("export_declined")
    );
    assert_eq!(dialogs.high_risk_prompts().len(), 1);
    assert_eq!(std::fs::read(&target).unwrap(), b"user program");

    // Changed while the user was asked: not replaced.
    assert_eq!(
        bridge.execute(&mut app, over(), false),
        Err("export_failed")
    );
    assert_eq!(dialogs.high_risk_prompts().len(), 2);
    assert_eq!(std::fs::read(&target).unwrap(), b"saved meanwhile");

    // Agreed: the program replaces it.
    bridge
        .execute(&mut app, over(), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(dialogs.high_risk_prompts().len(), 3);
    let program = std::fs::read_to_string(&target).unwrap();
    assert!(program.contains("G43 H1"), "{program}");
}

#[test]
fn an_unknown_dialect_or_one_given_to_another_format_is_refused_before_anything_runs() {
    let dialogs = ScriptedFileDialogs::new();
    let (mut app, mut bridge, directory) = cam_window(&dialogs);
    let target = directory.path().join("facing.nc");
    assert_eq!(
        bridge.execute(
            &mut app,
            cam_export(&target, "cam_nc", Some("no_such_controller")),
            false
        ),
        Err("invalid_params")
    );
    let step = directory.path().join("model.step");
    assert_eq!(
        bridge.execute(&mut app, cam_export(&step, "step", Some("linuxcnc")), false),
        Err("invalid_params")
    );
    assert!(!target.exists() && !step.exists());
    assert!(dialogs.high_risk_prompts().is_empty());
}

#[test]
fn a_window_without_a_cam_plan_refuses_the_program() {
    let (mut app, mut bridge) = setup();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("facing.nc");
    assert_eq!(
        bridge.execute(&mut app, cam_export(&target, "cam_nc", None), false),
        Err("export_refused")
    );
    assert!(!target.exists());
}

/// The ordinal `query kind=faces` gives the lowest face of the block.
fn bottom_face(app: &mut KetchupApp, bridge: &mut LiveBridge) -> u64 {
    super::program_edit::evaluate_exact(app);
    let page = bridge
        .execute(
            app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(json!({"kind": "faces", "limit": 50})).unwrap(),
            },
            false,
        )
        .unwrap();
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["definition_id"] == 95)
        .min_by(|left, right| {
            let z = |row: &Value| row["geometry"]["centroid_mm"][2].as_f64().unwrap();
            z(left).total_cmp(&z(right))
        })
        .unwrap_or_else(|| panic!("no face of the block in {page}"))["ordinal"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_client_profiles_a_part_from_a_face_it_names_and_the_window_reads_the_outline() {
    let dialogs = ScriptedFileDialogs::new();
    let (mut app, mut bridge, directory) = cam_window_over(
        &dialogs,
        CamStock {
            minimum_mm: [-3.0, -3.0, 0.0],
            maximum_mm: [23.0, 13.0, 5.0],
        },
        None,
    );
    let bottom = bottom_face(&mut app, &mut bridge);
    let before = app.canonical_digest();

    // The client names the bottom face; Ketchup reads its outline from the exact part and
    // cuts one tool radius outside it, from the stock top down to the face.
    let target = directory.path().join("profile.gcode");
    let exported = bridge
        .execute(
            &mut app,
            cam_export_of(
                &target,
                "cam_nc",
                Some("grbl"),
                Some(json!([{"type": "face_contour", "face_ordinal": bottom, "side": "outside"}])),
            ),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let program = std::fs::read_to_string(&target).unwrap();
    assert!(program.contains("X-1"), "{program}");
    let band_mm2 = 60.0 * 2.0 + std::f64::consts::PI * 4.0;
    let removed = exported["machining"]["removed_stock_mm3"].as_f64().unwrap();
    assert!((removed - band_mm2 * 5.0).abs() < 0.5, "{exported}");
    assert_eq!(app.canonical_digest(), before);

    // A flat face is no hole and a face the part does not have is refused before any prompt;
    // a malformed operation, or operations given to another format, before anything runs.
    for (operations, code) in [
        (
            json!([{"type": "face_drill", "face_ordinal": bottom}]),
            "export_failed",
        ),
        (
            json!([{"type": "face_pocket", "face_ordinal": 9_999}]),
            "export_failed",
        ),
        (json!([]), "invalid_params"),
    ] {
        let refused = directory.path().join("refused.nc");
        let request = cam_export_of(&refused, "cam_nc", None, Some(operations.clone()));
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err(code),
            "{operations}"
        );
        assert!(!refused.exists());
    }
    // A malformed operation is no request at all.
    assert!(
        serde_json::from_value::<Vec<crate::live_bridge::interchange::CamFaceOperationRequest>>(
            json!([
                {"type": "face_contour", "face_ordinal": bottom, "side": "diagonal"}
            ])
        )
        .is_err()
    );
    let step = directory.path().join("model.step");
    assert_eq!(
        bridge.execute(
            &mut app,
            cam_export_of(
                &step,
                "step",
                None,
                Some(json!([{"type": "face_pocket", "face_ordinal": bottom}]))
            ),
            false
        ),
        Err("invalid_params")
    );
    assert!(dialogs.high_risk_prompts().is_empty());
}
