//! A client writes the CAM program of the document's plan for the controller it names, as a
//! manufacturing release the user confirms in the window.

use super::*;
use ketchup_model::cam::{
    CamCutParameters, CamPlan, CamPlanId, CamPostprocessorDialect, CamSetup, CamStock, CamTool,
    CamToolKind, CamWorkOffset,
};
use ketchup_model::document::{CanonicalCommand, DefinitionId, Dimension, FeatureId, FeatureKind};

/// A window holding a 20 × 10 × 5 mm block and a facing plan for it, whose prompts answer as
/// `dialogs` says.
fn cam_window(dialogs: &ScriptedFileDialogs) -> (KetchupApp, LiveBridge, tempfile::TempDir) {
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
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &app.document.current(),
        CamPlanId(95),
        "Reviewed facing",
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [20.0, 10.0, 7.0],
        },
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
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: export_format.to_owned(),
        drawing_scale: None,
        image: None,
        cam_dialect: cam_dialect.map(str::to_owned),
        overwrite: false,
    }
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
