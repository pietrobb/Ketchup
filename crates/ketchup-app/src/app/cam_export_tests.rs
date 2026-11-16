//! The File menu's CAM export machines the face the user selected: they choose the
//! operation and the controller, simulate it exactly and write the program.
use crate::dialogs::ScriptedFileDialogs;
use crate::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use ketchup_model::cam::{
    CamCutParameters, CamPlan, CamPlanId, CamPostprocessorDialect, CamSetup, CamStock, CamTool,
    CamToolKind, CamWorkOffset,
};
use ketchup_model::exact_product::ExactBodyPackage;

type Shell = egui_kittest::Harness<'static, KetchupApp>;

const PART: DefinitionId = DefinitionId(60);
const PLATE: FeatureId = FeatureId(602);
const POCKET: FeatureId = FeatureId(604);
const PLACED: OccurrenceId = OccurrenceId(605);

/// A 40 × 30 × 10 mm plate with a 20 × 10 mm pocket 4 mm deep, placed once, and a plan
/// that cuts it with a 2 mm end mill out of stock the size of the plate.
fn plate_with_a_pocket(dialogs: &ScriptedFileDialogs) -> KetchupApp {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs.clone()));
    let rectangle = |id, low: [f64; 2], high: [f64; 2]| CanonicalCommand::CreateFeature {
        id,
        definition_id: PART,
        name: format!("profile {}", id.0),
        kind: FeatureKind::polygon(&[low, [high[0], low[1]], high, [low[0], high[1]]]),
    };
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: PART,
                name: "Pocketed plate".into(),
            },
            rectangle(FeatureId(601), [0.0, 0.0], [40.0, 30.0]),
            CanonicalCommand::CreateFeature {
                id: PLATE,
                definition_id: PART,
                name: "Plate".into(),
                kind: FeatureKind::extrusion(FeatureId(601), Dimension::new("10", 10.0).unwrap()),
            },
            rectangle(FeatureId(603), [10.0, 10.0], [30.0, 20.0]),
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: PART,
                name: "Pocket".into(),
                kind: FeatureKind::pocket(PLATE, FeatureId(603), Dimension::new("4", 4.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: PLACED,
                definition_id: PART,
                name: "Plate".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &app.document.current(),
        CamPlanId(60),
        "Pocket",
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [40.0, 30.0, 10.0],
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
            safe_height_mm: 15.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        PART,
        POCKET,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
            plan,
        )]))
        .unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    app
}

/// The face of the plate whose plane faces up at `z_mm`, the first if several do.
fn face_up_at(app: &KetchupApp, z_mm: f64) -> u32 {
    let snapshot = app.document.current();
    let package = app
        .exact
        .topology_results
        .get_render(&snapshot, PART)
        .unwrap();
    let ExactBodyPackage::Graph(graph) = package.as_ref() else {
        panic!("the plate is an exact graph");
    };
    graph
        .face_evidence
        .iter()
        .find(|face| {
            face.surface_kind == "plane"
                && face.unit_normal[2] > 0.5
                && (face.centroid_mm[2] - z_mm).abs() < 1e-6
        })
        .unwrap_or_else(|| panic!("no face up at z {z_mm}"))
        .face_ordinal
}

fn select_face(app: &mut KetchupApp, ordinal: u32) {
    assert!(app.select_topological_locator(TopologicalPickLocator {
        instance_path: InstancePath::root(PLACED),
        producer_feature_id: POCKET,
        kind: TopologicalElementKind::Face,
        ordinal,
    }));
}

fn shell(app: KetchupApp) -> Shell {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    harness
}

fn choose(harness: &mut Shell, combo_key: &str, option: &str) {
    let combo = harness.state().catalog.text(combo_key);
    harness
        .get_by_role_and_label(Role::ComboBox, &combo)
        .click();
    harness.run_steps(2);
    harness.get_by_label(option).click();
    harness.run_steps(2);
}

fn click(harness: &mut Shell, key: &str) {
    let label = harness.state().catalog.text(key);
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(2);
}

/// The X, Y, Z every motion of `program` reaches, the axes it leaves out kept from before.
fn positions(program: &str) -> Vec<[f64; 3]> {
    let mut at = [f64::NAN; 3];
    let mut reached = Vec::new();
    for line in program.lines() {
        let words = line.split_whitespace().collect::<Vec<_>>();
        if !words
            .iter()
            .any(|word| ["G0", "G1", "G2", "G3"].contains(word))
        {
            continue;
        }
        for word in words {
            if let Some(axis) = "XYZ".find(word.get(..1).unwrap_or_default())
                && let Ok(value) = word[1..].parse()
            {
                at[axis] = value;
            }
        }
        reached.push(at);
    }
    reached
}

#[test]
fn the_window_pockets_the_selected_floor_for_the_chosen_controller() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("pocket.gcode");
    let dialogs = ScriptedFileDialogs::new().queue_export(&output);
    let mut app = plate_with_a_pocket(&dialogs);
    let floor = face_up_at(&app, 6.0);
    select_face(&mut app, floor);
    let before = app.canonical_digest();
    let mut harness = shell(app);
    harness
        .state_mut()
        .dispatch_command(AppCommand::ReviewCamExport);
    harness.run_steps(2);

    // A flat floor is no hole: drilling it is refused and nothing can be written.
    let drill = harness.state().catalog.text("cam-operation-drill");
    choose(&mut harness, "cam-review-operation", &drill);
    click(&mut harness, "cam-review-preview");
    let dialog = harness.state().reviews.cam_export_dialog.as_ref().unwrap();
    assert!(dialog.review.is_none(), "{}", harness.state().digest);

    let pocket = harness.state().catalog.text("cam-operation-pocket");
    choose(&mut harness, "cam-review-operation", &pocket);
    let grbl = match CamPostprocessorDialect::library("grbl").unwrap() {
        CamPostprocessorDialect::GCode(dialect) => dialect.name().to_owned(),
        CamPostprocessorDialect::ControllerNeutralJson => unreachable!(),
    };
    choose(&mut harness, "cam-review-dialect", &grbl);
    click(&mut harness, "cam-review-preview");
    let review = harness
        .state()
        .reviews
        .cam_export_dialog
        .as_ref()
        .and_then(|dialog| dialog.review.as_ref())
        .unwrap_or_else(|| panic!("{}", harness.state().digest));
    // The pocket less the four corners a 1 mm radius cutter leaves, 4 mm deep.
    let corners_mm2 = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4);
    let expected_mm3 = (20.0 * 10.0 - corners_mm2) * 4.0;
    assert!(
        (review.removed_stock_mm3 - expected_mm3).abs() < 0.5,
        "{} != {expected_mm3}",
        review.removed_stock_mm3
    );
    assert!(review.gouge_mm3.abs() < 1e-6, "{}", review.gouge_mm3);

    click(&mut harness, "cam-review-confirm-export");
    let program = std::fs::read_to_string(&output)
        .unwrap_or_else(|error| panic!("{error}: {}", harness.state().digest));
    let cutting = positions(&program)
        .into_iter()
        .filter(|at| at[2] < 10.0 - 1e-6)
        .collect::<Vec<_>>();
    assert!(!cutting.is_empty(), "{program}");
    for [x, y, _] in &cutting {
        assert!(
            (11.0 - 1e-6..=29.0 + 1e-6).contains(x) && (11.0 - 1e-6..=19.0 + 1e-6).contains(y),
            "cut outside the pocket at {x}, {y}:\n{program}"
        );
    }
    let deepest = cutting.iter().map(|at| at[2]).fold(f64::INFINITY, f64::min);
    assert!((deepest - 6.0).abs() < 1e-6, "{deepest}\n{program}");
    assert_eq!(harness.state().canonical_digest(), before);
}
