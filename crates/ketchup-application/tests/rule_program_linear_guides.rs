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

fn carriage_volume(length: f64) -> f64 {
    // Table 3.79: W 27 x (H 13 - H1 3) less the rail envelope 12 x (8 - 3); fixture drills 7 mm.
    sized_carriage_volume([27.0, 13.0, 3.0, 12.0, 8.0], length)
}

/// [W, H, H1, W_R, H_R] of table 3.79/3.81; four M3 tap drills 7 mm deep.
fn sized_carriage_volume([w, h, h1, rail_w, rail_h]: [f64; 5], length: f64) -> f64 {
    (w * (h - h1) - rail_w * (rail_h - h1)) * length - 4.0 * PI * 1.25 * 1.25 * 7.0
}

#[test]
fn mgn_catalogue_covers_sizes_9_and_15_with_their_rails() {
    let source = r#"
for size, block, rail in [
    ("MGN09C", (10, 2, 20, 28.9, 15, 10, "M3", 3, 1860, 2550, 0.02), (9, 6.5, 20, 3.5, 6, 3.5, 30, 1200, 5, 15)),
    ("MGN09H", (10, 2, 20, 39.9, 15, 16, "M3", 3, 2550, 4020, 0.03), (9, 6.5, 20, 3.5, 6, 3.5, 30, 1200, 5, 15)),
    ("MGN15C", (16, 4, 32, 42.1, 25, 20, "M3", 4, 4610, 5590, 0.06), (15, 10, 40, 3.5, 6, 4.5, 52, 2000, 6, 34)),
    ("MGN15H", (16, 4, 32, 58.8, 25, 25, "M3", 4, 6370, 9110, 0.09), (15, 10, 40, 3.5, 6, 4.5, 52, 2000, 6, 34)),
]:
    d = linear_carriage_dimensions(size)
    got = (d["height_mm"], d["clearance_mm"], d["width_mm"], d["length_mm"], d["hole_spacing_x_mm"],
           d["hole_spacing_z_mm"], d["mount_thread"], d["mount_thread_length_mm"], d["dynamic_load_n"],
           d["static_load_n"], d["mass_kg"])
    if got != block or "3.79" not in d["source"]:
        fail("wrong block %s: %r" % (size, got))
    r = linear_rail_dimensions(d["rail"])
    got = (r["width_mm"], r["height_mm"], r["pitch_mm"], r["hole_diameter_mm"], r["counterbore_diameter_mm"],
           r["counterbore_depth_mm"], r["min_length_mm"], r["max_length_mm"], r["min_end_mm"], r["max_end_mm"])
    if got != rail or r["section"] != None or "3.81" not in r["source"]:
        fail("wrong rail %s: %r" % (d["rail"], got))
def box(w, h):  # synthetic envelope for hole layout only, NOT a raceway profile
    return [["bottom", (-w, 0), (w, 0)], ["right", (w, 0), (w, h)], ["top", (w, h), (-w, h)], ["left", (-w, h), (-w, 0)]]
linear_rail("rail15", "MGNR15R", 100, first_hole=10, section=box(7.5, 10), section_source="synthetic test section")
linear_rail("rail9", "MGNR09R", 100, first_hole=10, section=box(4.5, 6.5), section_source="synthetic test section")
"#;
    let model = ketchup_program::run("mgn-sizes.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    for (part, pitch, count, height) in [(0, 40.0, 3, 10.0), (1, 20.0, 5, 6.5)] {
        let holes = model.parts[part].holes().collect::<Vec<_>>();
        assert_eq!(holes.len(), 2 * count);
        for (index, pair) in holes.chunks_exact(2).enumerate() {
            assert_eq!(pair[0].entry_mm, [0.0, height, 10.0 + pitch * index as f64]);
            assert_eq!(pair[0].depth_mm, height);
        }
    }
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (designation, envelope, length) in [
        ("MGN09C", [20.0, 10.0, 2.0, 9.0, 6.5], 28.9),
        ("MGN09H", [20.0, 10.0, 2.0, 9.0, 6.5], 39.9),
        ("MGN15C", [32.0, 16.0, 4.0, 15.0, 10.0], 42.1),
        ("MGN15H", [32.0, 16.0, 4.0, 15.0, 10.0], 58.8),
    ] {
        assert_volume(
            volume(
                &mut worker,
                &format!("p = linear_carriage('block', '{designation}', mount_drill_depth=7)"),
                "block",
            ),
            sized_carriage_volume(envelope, length),
        );
    }
}

#[test]
fn linear_carriage_uses_catalogue_block_dimensions_and_preserves_unknowns() {
    let source = r#"
d = linear_carriage_dimensions("MGN12H")
if (d["height_mm"], d["clearance_mm"], d["width_mm"], d["length_mm"]) != (13, 3, 27, 45.4):
    fail("wrong block envelope")
if (d["hole_spacing_x_mm"], d["hole_spacing_z_mm"], d["mount_thread"], d["mount_thread_length_mm"]) != (20, 20, "M3", 3.5):
    fail("wrong mounting pattern")
if (d["rail"], d["dynamic_load_n"], d["static_load_n"], d["mass_kg"]) != ("MGNR12R", 3720, 5880, 0.05):
    fail("wrong rail, ratings or weight")
if d["raceway_section"] != None or d["mount_drill_depth_mm"] != None:
    fail("undimensioned details must remain unspecified")
if "Hiwin_MG.pdf" not in d["source"] or "3.79" not in d["source"]:
    fail("missing catalogue citation")
if linear_carriage_dimensions("MGN12C")["length_mm"] != 34.7 or linear_carriage_dimensions("MGN12C")["hole_spacing_z_mm"] != 15:
    fail("wrong short block")
linear_carriage("block", "MGN12H", at=(0,0,20), mount_drill_depth=7)
"#;
    let model = ketchup_program::run("carriage.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let holes = model.parts[0].holes().collect::<Vec<_>>();
    assert_eq!(holes.len(), 4);
    let mut entries = holes
        .iter()
        .map(|hole| {
            assert_eq!(hole.diameter_mm, 2.5);
            assert_eq!(hole.depth_mm, 7.0);
            assert_eq!(hole.inward, [0.0, -1.0, 0.0]);
            assert!(!hole.through);
            hole.entry_mm.map(|value| (value * 1.0e6).round() / 1.0e6)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Part-local: B 20 across, C 20 along about the block centre L/2 = 22.7.
    assert_eq!(
        entries,
        [
            [-10.0, 13.0, 12.7],
            [-10.0, 13.0, 32.7],
            [10.0, 13.0, 12.7],
            [10.0, 13.0, 32.7]
        ]
    );
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware[0].item, "HIWIN MGN12H");
    let attributes = &bom.hardware[0].attributes;
    assert!(attributes["geometry_basis"].contains("envelope"));
    assert!(attributes["geometry_basis"].contains("raceways"));
    assert_eq!(attributes["rail"], "MGNR12R");
    assert_eq!(attributes["catalogue_mass_kg"], "0.05");
    assert!(attributes["mount_drill_depth_basis"].contains("caller-supplied"));
    for (call, reason) in [
        (
            "linear_carriage('b','MGN12H')",
            "mount_drill_depth is not specified",
        ),
        (
            "linear_carriage('b','MGN12H',mount_drill_depth=4.5)",
            "runout",
        ),
        (
            "linear_carriage('b','MGN07H',mount_drill_depth=7)",
            "not specified",
        ),
        ("linear_carriage_dimensions('MGN12')", "not specified"),
    ] {
        let error = ketchup_program::run("invalid-carriage.star", call, &BTreeMap::new())
            .expect_err("unknown blocks or unpublished depths must fail")
            .to_string();
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn linear_carriage_exact_envelope_clears_the_rail() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (designation, length) in [("MGN12C", 34.7), ("MGN12H", 45.4)] {
        assert_volume(
            volume(
                &mut worker,
                &format!("p = linear_carriage('block', '{designation}', mount_drill_depth=7)"),
                "block",
            ),
            carriage_volume(length),
        );
    }
}

#[test]
fn rail_and_carriage_component_keeps_bom_and_exact_step_after_save_open() {
    let _turn = crate::integration_support::file_turn();
    let source = format!(
        "{SECTION}\nr=linear_rail('rail','MGNR12R',100,first_hole=10,section=section,section_source='synthetic test section')\nb=linear_carriage('block','MGN12H',at=(0,0,20),mount_drill_depth=7)\nc=component('guide',[r,b])\ninstance('second guide',c,at=(0,0,150))"
    );
    let model = ketchup_program::run("guide.star", &source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert_eq!(bom.hardware.len(), 2);
    assert!(bom.hardware.iter().all(|row| row.count == 2));
    let mut document = session(&source).unwrap();
    let before = document.snapshot();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guide.ketchup");
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
        exported.push((
            ExactBodyPackage::Graph(worker.evaluate_exact_brep_graph(&graph).unwrap()),
            part.transform,
        ));
    }
    assert_eq!(exported.len(), 4);
    let step = dir.path().join("guide.step");
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
    assert_eq!(imported.solid_count, 4);
    assert_volume(
        imported.volume_mm3,
        2.0 * (expected_volume(100.0, 4.0) + carriage_volume(45.4)),
    );
    for (actual, expected) in imported
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([-13.5, 0.0, 0.0, 13.5, 13.0, 250.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "STEP bound {actual} != {expected}"
        );
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
