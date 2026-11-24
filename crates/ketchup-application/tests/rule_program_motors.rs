//! Supplier-dimensioned motor interfaces, with explicit limits on housing detail.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::{collections::BTreeMap, f64::consts::PI, sync::atomic::AtomicBool};

fn expected_volume() -> f64 {
    // Drawing: housing maximum and nominal pilot/shaft; fixture supplies a 7.5 mm drill depth.
    let flat_segment = 2.5_f64.powi(2) * (2.0_f64 / 2.5).acos() - 2.0 * 1.5;
    42.3 * 42.3 * 48.0 - 4.0 * PI * 1.25 * 1.25 * 7.5
        + PI * 11.0 * 11.0 * 2.0
        + PI * 2.5 * 2.5 * 22.0
        - flat_segment * 15.0
}

#[test]
fn motor_catalogue_preserves_supplier_dimensions_and_unknowns() {
    let source = r#"
d = stepper_motor_dimensions("17HS19-2004S1")
if (d["frame_max_mm"], d["body_length_max_mm"], d["mount_pitch_mm"]) != (42.3, 48, 31):
    fail("wrong mounting dimensions")
if (d["pilot_diameter_mm"], d["pilot_height_mm"], d["shaft_length_mm"]) != (22, 2, 24):
    fail("wrong pilot or shaft")
if (d["shaft_diameter_mm"], d["shaft_flat_thickness_mm"], d["shaft_flat_length_mm"]) != (5, 4.5, 15):
    fail("wrong D shaft")
if d["housing_corner_mm"] != None or d["mass_kg"] != None:
    fail("unpublished dimensions must remain unspecified")
if "17HS19-2004S1.pdf" not in d["source"]:
    fail("supplier drawing missing")
stepper_motor("motor", "17HS19-2004S1", mount_drill_depth=7.5)
"#;
    let model = ketchup_program::run("motor.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let holes = model.parts[0].holes().collect::<Vec<_>>();
    assert_eq!(holes.len(), 4);
    let mut entries = holes
        .iter()
        .map(|hole| {
            assert_eq!(hole.depth_mm, 7.5);
            assert_eq!(hole.diameter_mm, 2.5);
            assert!(!hole.through);
            hole.entry_mm
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        entries,
        [
            [-15.5, -15.5, 48.0],
            [-15.5, 15.5, 48.0],
            [15.5, -15.5, 48.0],
            [15.5, 15.5, 48.0]
        ]
    );
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].item, "STEPPERONLINE 17HS19-2004S1");
    assert_eq!(bom.hardware[0].count, 1);
    let attributes = &bom.hardware[0].attributes;
    assert!(attributes["geometry_basis"].contains("housing envelope"));
    assert!(attributes["geometry_basis"].contains("cosmetic"));
    assert!(attributes["mass_basis"].contains("not specified"));
    assert!(!attributes.contains_key("mass_kg"));
    assert_eq!(attributes["mount_drill_depth_mm"], "7.5");
    assert!(attributes["mount_drill_depth_basis"].contains("caller-supplied"));
    for (argument, reason) in [
        ("", "mount_drill_depth is not specified"),
        (", mount_drill_depth=4.5", "runout"),
    ] {
        let error = ketchup_program::run(
            "motor-depth.star",
            &format!("stepper_motor(\"motor\", \"17HS19-2004S1\"{argument})"),
            &BTreeMap::new(),
        )
        .expect_err("an unknown or unsafe drill depth must not be invented or accepted")
        .to_string();
        assert!(error.contains(reason), "{error}");
    }
    for designation in ["NEMA17", "NEMA23", "unknown"] {
        let error = ketchup_program::run(
            "unknown.star",
            &format!("stepper_motor(\"motor\", \"{designation}\")"),
            &BTreeMap::new(),
        )
        .expect_err("a frame size does not specify a supplier motor")
        .to_string();
        assert!(error.contains("not specified"), "{error}");
    }
}

fn nema23_volume() -> f64 {
    // The fixture supplies 5 mm clearance-hole depth, not published by the supplier.
    let radius = 6.35_f64 / 2.0;
    let flat = 5.85 - radius;
    let segment =
        radius * radius * (flat / radius).acos() - flat * (radius * radius - flat * flat).sqrt();
    57.3 * 57.3 * 56.0 - 4.0 * PI * 2.6 * 2.6 * 5.0
        + PI * 19.05 * 19.05 * 1.6
        + PI * radius * radius * (21.0 - 1.6)
        - segment * 15.0
}

#[test]
fn nema23_has_supplier_clearance_mounts_not_invented_threads() {
    let _turn = crate::integration_support::file_turn();
    let source = r#"
d = stepper_motor_dimensions("23HS22-2804S")
if (d["frame_max_mm"], d["body_length_max_mm"], d["mount_pitch_mm"]) != (57.3, 56, 47.14):
    fail("wrong NEMA23 housing or mounting pitch")
if (d["pilot_diameter_mm"], d["pilot_height_mm"], d["shaft_length_mm"]) != (38.1, 1.6, 21):
    fail("wrong NEMA23 pilot or shaft")
if (d["shaft_diameter_mm"], d["shaft_flat_thickness_mm"], d["shaft_flat_length_mm"]) != (6.35, 5.85, 15):
    fail("wrong NEMA23 D shaft")
if d["mount_hole_diameter_mm"] != 5.2 or d["mount_thread"] != None:
    fail("clearance holes are not tapped holes")
if d["mount_drill_depth_mm"] != None or d["mass_kg"] != None or d["housing_corner_mm"] != None:
    fail("unpublished details must remain unspecified")
if "23HS22-2804S.pdf" not in d["source"]:
    fail("supplier drawing missing")
stepper_motor("motor", "23HS22-2804S", mount_drill_depth=5)
"#;
    let model = ketchup_program::run("nema23.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let holes = model.parts[0].holes().collect::<Vec<_>>();
    assert_eq!(holes.len(), 4);
    let mut entries = holes
        .iter()
        .map(|hole| {
            assert_eq!(hole.diameter_mm, 5.2);
            assert_eq!(hole.depth_mm, 5.0);
            assert!(!hole.through);
            hole.entry_mm
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        entries,
        [
            [-23.57, -23.57, 56.0],
            [-23.57, 23.57, 56.0],
            [23.57, -23.57, 56.0],
            [23.57, 23.57, 56.0],
        ]
    );
    assert_eq!(
        session(source).unwrap().snapshot().hole_threads().count(),
        0
    );
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware[0].item, "STEPPERONLINE 23HS22-2804S");
    assert!(bom.hardware[0].attributes["geometry_basis"].contains("clearance"));
    assert!(!bom.hardware[0].attributes["geometry_basis"].contains("M3"));
    assert!(bom.hardware[0].attributes["mount_drill_depth_basis"].contains("caller-supplied"));
    assert_volume(volume(&mut worker(), source, "motor"), nema23_volume());
    let error = ketchup_program::run(
        "missing-depth.star",
        "stepper_motor(\"motor\", \"23HS22-2804S\")",
        &BTreeMap::new(),
    )
    .expect_err("clearance-hole termination is not dimensioned")
    .to_string();
    assert!(
        error.contains("mount_drill_depth is not specified"),
        "{error}"
    );
}

#[test]
fn motor_has_exact_pilot_d_shaft_and_four_blind_tapped_mounts() {
    let _turn = crate::integration_support::file_turn();
    let source = "stepper_motor(\"motor\", \"17HS19-2004S1\", mount_drill_depth=7.5)";
    assert_volume(volume(&mut worker(), source, "motor"), expected_volume());
    let session = session(source).unwrap();
    let snapshot = session.snapshot();
    let labels = snapshot
        .hole_threads()
        .map(|(_, thread)| thread.label())
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["M3×0.5 depth 4.5"; 4]);
}

#[test]
fn motor_components_keep_bom_placement_and_step_after_save_open() {
    let _turn = crate::integration_support::file_turn();
    for (designation, depth, expected, bounds) in [
        (
            "17HS19-2004S1",
            7.5,
            expected_volume(),
            [10.0, -1.15, 8.85, 82.0, 41.15, 151.15],
        ),
        (
            "23HS22-2804S",
            5.0,
            nema23_volume(),
            [10.0, -8.65, 1.35, 87.0, 48.65, 158.65],
        ),
    ] {
        motor_component_roundtrip(designation, depth, expected, bounds);
    }
}

fn motor_component_roundtrip(designation: &str, depth: f64, expected: f64, bounds: [f64; 6]) {
    let source = &format!(
        r#"
p = stepper_motor("motor", "{designation}", at=(10,20,30), axis=(1,0,0), mount_drill_depth={depth})
c = component("drive", [p])
instance("second drive", c, at=(0,0,100))
"#
    );
    let model = ketchup_program::run("motors.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].count, 2);
    assert_eq!(
        bom.hardware[0].attributes["source"],
        model.parts[0].attributes["source"]
    );
    let mut document = session(source).unwrap();
    let before = document.snapshot();
    let roots = before.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("motors.ketchup");
    document.save(&path, SaveOptions::default()).unwrap();
    drop(document);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.scene_query(), before.scene_query());
    assert_eq!(
        snapshot.hole_threads().collect::<Vec<_>>(),
        before.hole_threads().collect::<Vec<_>>()
    );
    let mut worker = worker();
    let mut exported = Vec::new();
    for part in snapshot.scene_query() {
        let definition = snapshot.definition(part.definition_id).unwrap();
        let Some(&tip) = definition.feature_ids().last() else {
            continue;
        };
        let graph = ExactBRepGraph::from_snapshot(&snapshot, part.definition_id, tip).unwrap();
        let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
        exported.push((ExactBodyPackage::Graph(package), part.transform));
    }
    assert_eq!(exported.len(), 2);
    let step = directory.path().join("motors.step");
    let bytes = worker
        .export_current_model_step(&snapshot, &exported, &step)
        .unwrap();
    let imported = worker
        .inspect_step_import_with_cancellation(
            &step,
            &ketchup_model::graph::sha256_hex(&bytes),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(imported.solid_count, 2);
    assert_volume(imported.volume_mm3, 2.0 * expected);
    for (actual, expected) in imported.bounds_mm.into_iter().flatten().zip(bounds) {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
    }
}
