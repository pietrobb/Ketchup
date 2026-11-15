//! Sourced bearing mounting envelopes, not invented rolling-element geometry.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::atomic::AtomicBool;

#[test]
fn bearing_envelopes_have_catalogue_bore_and_exact_annular_volume() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (designation, bore, outside, width) in [
        ("6000", 10.0_f64, 26.0_f64, 8.0),
        ("6001", 12.0, 28.0, 8.0),
        ("6004", 20.0, 42.0, 12.0),
        ("6201", 12.0, 32.0, 10.0),
        ("6204", 20.0, 47.0, 14.0),
    ] {
        let program = format!("bearing_envelope(\"bearing\", \"{designation}\")");
        assert_volume(
            volume(&mut worker, &program, "bearing"),
            PI * (outside.powi(2) - bore.powi(2)) / 4.0 * width,
        );
    }
}

#[test]
fn bearing_catalogue_does_not_invent_unspecified_dimensions() {
    let source = r#"
d = bearing_dimensions("6204")
if (d["bore_mm"], d["outside_mm"], d["width_mm"]) != (20, 47, 14):
    fail("wrong catalogue dimensions")
if d["ball_diameter_mm"] != None or d["ball_count"] != None:
    fail("rolling elements must stay unspecified")
if "skf.com/" not in d["source"] or "productid-6204" not in d["source"]:
    fail("catalogue source missing")
bearing_envelope("bearing", "6204")
"#;
    let model = ketchup_program::run("bearings.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    assert_eq!(model.parts.len(), 1);
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].item, "SKF 6204");
    assert_eq!(bom.hardware[0].count, 1);
    assert_eq!(
        bom.hardware[0].attributes["source"],
        model.parts[0].attributes["source"]
    );
    let attributes = &model.parts[0].attributes;
    assert_eq!(attributes["designation"], "SKF 6204");
    assert_eq!(
        attributes["geometry_basis"],
        "mounting envelope only; internal geometry not specified"
    );
    assert_eq!(
        attributes["mass_basis"],
        "not specified; envelope volume is not bearing material volume"
    );
    let error = ketchup_program::run(
        "unknown-bearing.star",
        "bearing_envelope(\"unknown\", \"6299\")",
        &BTreeMap::new(),
    )
    .expect_err("an unlisted designation must not be extrapolated")
    .to_string();
    assert!(error.contains("not specified"), "{error}");
    assert!(error.contains("6299"), "{error}");
}

#[test]
fn bearing_components_keep_placement_bom_and_exact_step_after_save_open() {
    let _turn = crate::integration_support::file_turn();
    let source = r#"
p = bearing_envelope("bearing", "6201", at=(10,20,30), axis=(1,0,0))
c = component("bearing unit", [p])
instance("second bearing", c, at=(0,0,100))
"#;
    let model = ketchup_program::run("bearings.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].item, "SKF 6201");
    assert_eq!(bom.hardware[0].count, 2);
    let mut session = session(source).unwrap();
    let before = session.snapshot();
    let roots = before.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    let directory = tempfile::tempdir().unwrap();
    let native_path = directory.path().join("bearings.ketchup");
    session.save(&native_path, SaveOptions::default()).unwrap();
    drop(session);
    let reopened = DocumentSession::open(&native_path, SessionSettings::default()).unwrap();
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.scene_query(), before.scene_query());
    assert_eq!(
        snapshot.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
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
    let step_path = directory.path().join("bearings.step");
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
    assert_volume(
        imported.volume_mm3,
        2.0 * PI * (32.0_f64.powi(2) - 12.0_f64.powi(2)) / 4.0 * 10.0,
    );
    for (actual, expected) in imported
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([10.0, 4.0, 14.0, 20.0, 36.0, 146.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
    }
}
