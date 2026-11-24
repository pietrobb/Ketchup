//! Supplier STEP-derived extrusion sections, including slot lips, radii and bore.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

// Integral of the line/circle wires of Motedis P20_N5_I.stp, end face #2482.
// Supplier STEP dated 2012-11-22, 1000 mm extrusion; not a rectangular envelope.
const SECTION_AREA: f64 = 181.936_180_217;

#[test]
fn slot_profile_catalogue_identifies_exact_supplier_section() {
    let source = r#"
d = slot_profile_dimensions("Motedis-20x20-I5")
if d["width_mm"] != 20 or d["height_mm"] != 20 or d["bore_mm"] != 4.3:
    fail("incorrect supplier section")
if "motedis.com/shop/products_files/" not in d["source"]:
    fail("missing supplier CAD citation")
slot_profile("bar", "Motedis-20x20-I5", 125)
"#;
    let model = ketchup_program::run("profile.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].count, 1);
    assert!(bom.hardware[0].item.contains("Motedis-20x20-I5"));
    assert_eq!(
        bom.hardware[0].attributes["length_mm"]
            .parse::<f64>()
            .unwrap(),
        125.0
    );
    assert!(bom.hardware[0].attributes["geometry_basis"].contains("supplier STEP"));
    for call in [
        "slot_profile_dimensions('generic-2020')",
        "slot_profile_dimensions('Motedis-30x30-I5')",
        "slot_profile('bar','Motedis-20x20-I5',0)",
        "slot_profile('bar','Motedis-20x20-I5',-1)",
    ] {
        assert!(
            ketchup_program::run("invalid.star", call, &BTreeMap::new()).is_err(),
            "{call}"
        );
    }
}

#[test]
fn slot_profile_exact_volume_retains_slots_and_bore_at_each_length() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for length in [25.0, 125.0, 1000.0] {
        let source = format!("slot_profile('bar','Motedis-20x20-I5',{length})");
        assert_volume(volume(&mut worker, &source, "bar"), SECTION_AREA * length);
    }
}

#[test]
fn slot_profile_components_keep_bom_and_rotated_step_after_save_open() {
    check_components("Motedis-20x20-I5", SECTION_AREA, 20.0);
}

#[test]
fn slot_profile_30_retains_supplier_voids_and_exports_components() {
    let source = r#"
d = slot_profile_dimensions("Motedis-30x30L-I6")
if d["width_mm"] != 30 or d["height_mm"] != 30 or d["bore_mm"] != 4.9:
    fail("incorrect supplier section")
if d["source"] != "https://www.motedis.com/shop/products_files/Motedis_019597.zip":
    fail("missing supplier CAD citation")
slot_profile("bar", "Motedis-30x30L-I6", 75)
"#;
    let model = ketchup_program::run("profile30.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 1);
    assert!(bom.hardware[0].item.contains("Motedis-30x30L-I6"));
    // Integral of all six wires on supplier STEP face #3143 (2026-04-22).
    // Includes four corner cavities, not only the central circular bore.
    let area = 335.186_232_766;
    {
        let _turn = crate::integration_support::file_turn();
        let mut worker = worker();
        for length in [10.0, 75.0, 300.0] {
            let source = format!("slot_profile('bar','Motedis-30x30L-I6',{length})");
            assert_volume(volume(&mut worker, &source, "bar"), area * length);
        }
    }
    check_components("Motedis-30x30L-I6", area, 30.0);
}

#[test]
fn slot_profile_40_retains_supplier_voids_and_exports_components() {
    let source = r#"
d = slot_profile_dimensions("Motedis-40x40L-I8")
if d["width_mm"] != 40 or d["height_mm"] != 40 or d["bore_mm"] != 6.8:
    fail("incorrect supplier section")
if d["source"] != "https://www.motedis.com/shop/products_files/40x40L%20I-Type%20slot%208.zip":
    fail("missing supplier CAD citation")
slot_profile("bar", "Motedis-40x40L-I8", 60)
"#;
    let model = ketchup_program::run("profile40.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 1);
    assert!(bom.hardware[0].item.contains("Motedis-40x40L-I8"));
    // Integral of all six wires on supplier STEP P40L_N8_I.stp face #5037
    // (2013-03-20): 112 outer edges, four 8-edge cavities and the bore.
    let area = 657.097_170_981;
    {
        let _turn = crate::integration_support::file_turn();
        let mut worker = worker();
        for length in [10.0, 60.0, 300.0] {
            let source = format!("slot_profile('bar','Motedis-40x40L-I8',{length})");
            assert_volume(volume(&mut worker, &source, "bar"), area * length);
        }
    }
    check_components("Motedis-40x40L-I8", area, 40.0);
}

fn check_components(designation: &str, section_area: f64, width: f64) {
    let _turn = crate::integration_support::file_turn();
    let source = r#"
p=slot_profile("bar","Motedis-20x20-I5",125,at=(10,20,30),axis=(1,0,0))
c=component("frame member",[p])
instance("second member",c,at=(0,0,50))
"#
    .replace("Motedis-20x20-I5", designation);
    let source = source.as_str();
    let model = ketchup_program::run("frame.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].count, 2);
    let mut document = session(source).unwrap();
    let before = document.snapshot();
    let roots = before.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.ketchup");
    document.save(&path, SaveOptions::default()).unwrap();
    drop(document);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
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
        exported.push((
            ExactBodyPackage::Graph(worker.evaluate_exact_brep_graph(&graph).unwrap()),
            part.transform,
        ));
    }
    assert_eq!(exported.len(), 2);
    let step = dir.path().join("frame.step");
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
    assert_volume(imported.volume_mm3, 2.0 * section_area * 125.0);
    for (actual, expected) in imported.bounds_mm.into_iter().flatten().zip([
        10.0,
        20.0 - width / 2.0,
        30.0 - width / 2.0,
        135.0,
        20.0 + width / 2.0,
        80.0 + width / 2.0,
    ]) {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
    }
}
