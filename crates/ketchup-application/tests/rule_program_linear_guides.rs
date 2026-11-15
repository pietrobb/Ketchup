//! Catalogue mounting holes on a caller-supplied section, never invented raceways.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::{collections::BTreeMap, f64::consts::PI, sync::atomic::AtomicBool};

// Synthetic notched section for testing, NOT a claimed HIWIN raceway profile.
const SECTION: &str = r#"
section = [
 ["bottom", (-6,0), (6,0)], ["right_lower", (6,0), (6,3)],
 ["right_notch_a", (6,3), (5,4)], ["right_notch_b", (5,4), (6,5)],
 ["right_upper", (6,5), (6,8)], ["top", (6,8), (-6,8)],
 ["left_upper", (-6,8), (-6,5)], ["left_notch_a", (-6,5), (-5,4)],
 ["left_notch_b", (-5,4), (-6,3)], ["left_lower", (-6,3), (-6,0)]
]
"#;

fn rail_source(length: f64, first_hole: f64) -> String {
    format!(
        "{SECTION}\np = linear_rail(\"rail\", \"MGNR12R\", {length}, first_hole={first_hole}, section=section, section_source=\"synthetic test section, not supplier geometry\")"
    )
}

fn expected_volume(length: f64, holes: f64) -> f64 {
    94.0 * length
        - holes * PI * (1.75_f64.powi(2) * 8.0 + (3.0_f64.powi(2) - 1.75_f64.powi(2)) * 4.5)
}

#[test]
fn linear_rail_uses_catalogue_holes_and_preserves_unknown_raceways() {
    let source = format!(
        r#"
{SECTION}
d = linear_rail_dimensions("MGNR12R")
if (d["width_mm"], d["height_mm"], d["pitch_mm"]) != (12,8,25):
    fail("wrong rail dimensions")
if (d["hole_diameter_mm"], d["counterbore_diameter_mm"], d["counterbore_depth_mm"]) != (3.5,6,4.5):
    fail("wrong mounting holes")
if d["section"] != None:
    fail("raceway section is not dimensioned")
if "Hiwin_MG.pdf" not in d["source"] or "3.81" not in d["source"]:
    fail("missing catalogue citation")
linear_rail("rail", "MGNR12R", 100, first_hole=10, section=section, section_source="synthetic test section")
"#
    );
    let model = ketchup_program::run("rail.star", &source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let holes = model.parts[0].holes().collect::<Vec<_>>();
    assert_eq!(holes.len(), 8);
    for (index, pair) in holes.chunks_exact(2).enumerate() {
        let station = 10.0 + 25.0 * index as f64;
        assert_eq!(pair[0].entry_mm, [0.0, 8.0, station]);
        assert_eq!(pair[1].entry_mm, pair[0].entry_mm);
        assert_eq!(pair[0].inward, [0.0, -1.0, 0.0]);
        assert_eq!(pair[0].diameter_mm, 3.5);
        assert_eq!(pair[0].depth_mm, 8.0);
        assert!(pair[0].through);
        assert_eq!(pair[1].diameter_mm, 6.0);
        assert_eq!(pair[1].depth_mm, 4.5);
        assert!(!pair[1].through);
    }
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware[0].count, 1);
    assert!(bom.hardware[0].item.contains("MGNR12R"));
    assert_eq!(
        bom.hardware[0].attributes["length_mm"]
            .parse::<f64>()
            .unwrap(),
        100.0
    );
    assert_eq!(
        bom.hardware[0].attributes["section_source"],
        "synthetic test section"
    );
    assert!(bom.hardware[0].attributes["geometry_basis"].contains("caller-supplied"));
    for (call, reason) in [
        (
            "linear_rail('r','MGNR12R',100,first_hole=10)",
            "section is not specified",
        ),
        (
            "linear_rail('r','MGNR12R',100,first_hole=10,section=section)",
            "section_source",
        ),
        ("linear_rail_dimensions('MGNR99R')", "not specified"),
        (
            "linear_rail('r','MGNR12R',100,first_hole=10,section=section,section_source='  ')",
            "section_source",
        ),
        (
            "linear_rail('r','MGNR12R',100,first_hole=10,section=section,section_source='test',top_face='bottom')",
            "top_face",
        ),
        (
            "linear_rail('r','MGNR12R',100,first_hole=10,section=[(-5,0),(5,0),(5,8),(-5,8)],section_source='test')",
            "section must span",
        ),
        (
            "linear_rail('r','MGNR12R',100,section=section,section_source='test')",
            "first_hole",
        ),
        (
            "linear_rail('r','MGNR12R',100,first_hole=4,section=section,section_source='test')",
            "end distances",
        ),
        (
            "linear_rail('r','MGNR12R',110,first_hole=10,section=section,section_source='test')",
            "end distances",
        ),
        (
            "linear_rail('r','MGNR12R',20,first_hole=10,section=section,section_source='test')",
            "length",
        ),
        (
            "linear_rail('r','MGNR12R',2001,first_hole=10,section=section,section_source='test')",
            "length",
        ),
    ] {
        let error = ketchup_program::run(
            "invalid-rail.star",
            &format!("{SECTION}\n{call}"),
            &BTreeMap::new(),
        )
        .expect_err("missing dimensions or out-of-catalogue layout must fail")
        .to_string();
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn linear_rail_exact_section_and_holes_follow_cut_length() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (length, first, count) in [(100.0, 10.0, 4.0), (70.0, 10.0, 3.0), (35.0, 5.0, 2.0)] {
        assert_volume(
            volume(&mut worker, &rail_source(length, first), "rail"),
            expected_volume(length, count),
        );
    }
}

#[test]
fn linear_rail_components_keep_bom_and_exact_step_after_save_open() {
    let _turn = crate::integration_support::file_turn();
    let source = format!(
        "{SECTION}\np=linear_rail('rail','MGNR12R',100,first_hole=10,section=section,section_source='synthetic test section',at=(10,20,30),axis=(1,0,0))\nc=component('guide',[p])\ninstance('second guide',c,at=(0,0,50))"
    );
    let model = ketchup_program::run("rails.star", &source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].count, 2);
    let mut document = session(&source).unwrap();
    let before = document.snapshot();
    let roots = before.occurrences().collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].definition_id(), roots[1].definition_id());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rails.ketchup");
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
    let step = dir.path().join("rails.step");
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
    assert_volume(imported.volume_mm3, 2.0 * expected_volume(100.0, 4.0));
    for (actual, expected) in imported
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([10.0, 12.0, 24.0, 110.0, 20.0, 86.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
    }
}
