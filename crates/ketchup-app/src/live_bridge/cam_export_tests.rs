//! A client writes the CAM program of the document's plan for the controller it names, as a
//! manufacturing release the user confirms in the window.

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
    )
}

/// The window of [`cam_window`] with its plan cutting `stock`, the block placed once.
fn cam_window_over(
    dialogs: &ScriptedFileDialogs,
    stock: CamStock,
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
    let bridge = transport::start(egui::Context::default()).unwrap();
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

#[test]
fn a_client_writes_the_cam_program_for_the_controller_it_names_once_the_user_confirms() {
    let dialogs = ScriptedFileDialogs::new()
        .queue_high_risk_approval(7)
        .queue_high_risk_approval(7)
        .queue_refused_high_risk();
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
    // The user saw which controller the program is for before it was written.
    let prompts = dialogs.high_risk_prompts();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].contains(&dialect_name("linuxcnc")),
        "{}",
        prompts[0]
    );
    assert_eq!(app.canonical_digest(), before);

    // Without a dialect the program is the library's first one.
    let first = CamPostprocessorDialect::library_ids().unwrap()[0].clone();
    let default_target = directory.path().join("facing.nc");
    bridge
        .execute(&mut app, cam_export(&default_target, "cam_nc", None), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert!(dialogs.high_risk_prompts()[1].contains(&dialect_name(&first)));

    // An existing file is never replaced.
    assert_eq!(
        bridge.execute(
            &mut app,
            cam_export(&target, "cam_nc", Some("linuxcnc")),
            false
        ),
        Err("export_failed")
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), program);

    // A declined release writes nothing.
    let declined = directory.path().join("declined.nc");
    assert_eq!(
        bridge.execute(
            &mut app,
            cam_export(&declined, "cam_nc", Some("fanuc")),
            false
        ),
        Err("export_declined")
    );
    assert!(!declined.exists());
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

/// The stock the confirmation says the program removes, in mmÂł.
fn removed_mm3(prompt: &str) -> f64 {
    let after = &prompt[prompt
        .find("removes ")
        .unwrap_or_else(|| panic!("{prompt}"))
        + 8..];
    after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("{prompt}"))
}

#[test]
fn a_client_profiles_a_part_from_a_face_it_names_and_the_window_reads_the_outline() {
    let dialogs = ScriptedFileDialogs::new().queue_high_risk_approval(7);
    let (mut app, mut bridge, directory) = cam_window_over(
        &dialogs,
        CamStock {
            minimum_mm: [-3.0, -3.0, 0.0],
            maximum_mm: [23.0, 13.0, 5.0],
        },
    );
    let bottom = bottom_face(&mut app, &mut bridge);
    let before = app.canonical_digest();

    // The client names the bottom face; Ketchup reads its outline from the exact part and
    // cuts one tool radius outside it, from the stock top down to the face.
    let target = directory.path().join("profile.gcode");
    bridge
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
    let prompts = dialogs.high_risk_prompts();
    let band_mm2 = 60.0 * 2.0 + std::f64::consts::PI * 4.0;
    assert!(
        (removed_mm3(&prompts[0]) - band_mm2 * 5.0).abs() < 0.5,
        "{}",
        prompts[0]
    );
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
    assert_eq!(dialogs.high_risk_prompts().len(), 1);
}
