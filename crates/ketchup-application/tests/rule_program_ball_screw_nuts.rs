//! Sourced DIN 69051 flange nut envelopes, not invented raceway geometry.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::atomic::AtomicBool;

struct Nut {
    screw: f64,
    body: f64,
    length: f64,
    flats: f64,
    flange: f64,
    flange_thickness: f64,
    hole: f64,
    holes: f64,
}

/// Flange circle cut by the two flats, minus mounting holes, plus the body, minus the bore.
fn nut_volume(n: &Nut) -> f64 {
    let r = n.flange / 2.0;
    let h = n.flats / 2.0;
    let segment = r * r * (h / r).acos() - h * (r * r - h * h).sqrt();
    let flange = PI * r * r - 2.0 * segment - n.holes * PI * (n.hole / 2.0).powi(2);
    flange * n.flange_thickness + PI * (n.body / 2.0).powi(2) * (n.length - n.flange_thickness)
        - PI * (n.screw / 2.0).powi(2) * n.length
}

// HIWIN Italy ground ballscrew catalogue, table 6.7 p.51.
const R16: Nut = Nut {
    screw: 16.0,
    body: 28.0,
    length: 40.0,
    flats: 40.0,
    flange: 48.0,
    flange_thickness: 10.0,
    hole: 5.5,
    holes: 6.0,
};
const R20: Nut = Nut {
    screw: 20.0,
    body: 36.0,
    length: 40.0,
    flats: 44.0,
    flange: 58.0,
    flange_thickness: 10.0,
    hole: 6.6,
    holes: 6.0,
};
const R38: Nut = Nut {
    screw: 38.0,
    body: 63.0,
    length: 80.0,
    flats: 70.0,
    flange: 93.0,
    flange_thickness: 14.0,
    hole: 9.0,
    holes: 8.0,
};

#[test]
fn ball_screw_nuts_have_catalogue_flange_body_and_published_hole_patterns() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (designation, nut) in [
        ("R16-5T3-FSI", R16),
        ("R20-5K4-FSC", R20),
        ("R38-10K5-FSC", R38),
    ] {
        let program = format!("ball_screw_nut(\"nut\", \"{designation}\")");
        assert_volume(volume(&mut worker, &program, "nut"), nut_volume(&nut));
    }
}

#[test]
fn ball_screw_nut_catalogue_does_not_invent_unspecified_dimensions() {
    let source = r#"
d = ball_screw_nut_dimensions("R20-5K4-FSC")
if (d["screw_mm"], d["lead_mm"], d["body_mm"], d["length_mm"]) != (20, 5, 36, 40):
    fail("wrong catalogue dimensions")
if d["raceway"] != None or d["mass_kg"] != None:
    fail("the raceway and mass must stay unspecified")
if "hiwin.it/" not in d["source"] or "table 6.7" not in d["source"]:
    fail("catalogue source missing")
if (d["hole_pattern"], ball_screw_nut_dimensions("R38-10K5-FSC")["hole_pattern"]) != (1, 2):
    fail("printed hole pattern types missing")
ball_screw_nut("nut", "R20-5K4-FSC")
"#;
    let model = ketchup_program::run("nuts.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    assert_eq!(model.parts.len(), 1);
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].item, "HIWIN R20-5K4-FSC");
    let attributes = &model.parts[0].attributes;
    assert_eq!(attributes["standard"], "DIN 69051-5 form B");
    assert_eq!(attributes["mount_holes"], "type 1: 6 x 6.6 on 47");
    assert!(
        attributes["geometry_basis"].contains("bore at nominal screw diameter"),
        "{}",
        attributes["geometry_basis"]
    );
    let error = ketchup_program::run(
        "unknown-nut.star",
        "ball_screw_nut(\"nut\", \"R12-4K3-FSC\")",
        &BTreeMap::new(),
    )
    .expect_err("an unlisted nut must not be extrapolated")
    .to_string();
    assert!(error.contains("not specified"), "{error}");
    assert!(error.contains("R12-4K3-FSC"), "{error}");
}

#[test]
fn ball_screw_nut_components_keep_placement_bom_and_exact_step_after_save_open() {
    let _turn = crate::integration_support::file_turn();
    let source = r#"
p = ball_screw_nut("nut", "R16-5T3-FSI", at=(10,20,30), axis=(1,0,0))
c = component("nut unit", [p])
instance("second nut", c, at=(0,0,100))
"#;
    let model = ketchup_program::run("nuts.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].item, "HIWIN R16-5T3-FSI");
    assert_eq!(bom.hardware[0].count, 2);
    let mut session = session(source).unwrap();
    let before = session.snapshot();
    let roots = before.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    let directory = tempfile::tempdir().unwrap();
    let native_path = directory.path().join("nuts.ketchup");
    session.save(&native_path, SaveOptions::default()).unwrap();
    drop(session);
    let reopened = DocumentSession::open(&native_path, SessionSettings::default()).unwrap();
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.scene_query(), before.scene_query());

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
    let step_path = directory.path().join("nuts.step");
    let bytes = worker
        .export_current_model_step(&snapshot, &exported, &step_path)
        .unwrap();
    let imported = worker
        .inspect_step_import_with_cancellation(
            &step_path,
            &ketchup_model::graph::sha256_hex(&bytes),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(imported.solid_count, 2);
    assert_volume(imported.volume_mm3, 2.0 * nut_volume(&R16));
    // Axis along +x from x=10; local x (flats 40 apart) turns onto world z, the round
    // 48 mm flange onto world y; the instance repeats it 100 mm up.
    for (actual, expected) in imported
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([10.0, -4.0, 10.0, 50.0, 44.0, 150.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
    }
}
