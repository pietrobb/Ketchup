//! Slot profiles: catalogue outer size on a caller-supplied section, never shipped supplier CAD.
use crate::operations_support::{assert_volume, session, volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::{exact_brep_graph::ExactBRepGraph, exact_product::ExactBodyPackage};
use std::{collections::BTreeMap, f64::consts::PI, sync::atomic::AtomicBool};

const BORE_RADIUS: f64 = 2.0;

/// Synthetic notched square with a round bore for testing, NOT a claimed Motedis section.
fn section(width: f64) -> String {
    let h = width / 2.0;
    let n = h / 4.0;
    format!(
        r#"
section = [
 ["bottom", (-{h},-{h}), ({h},-{h})], ["right_lower", ({h},-{h}), ({h},-{n})],
 ["right_notch_a", ({h},-{n}), ({h}-{n},0)], ["right_notch_b", ({h}-{n},0), ({h},{n})],
 ["right_upper", ({h},{n}), ({h},{h})], ["top", ({h},{h}), (-{h},{h})],
 ["left", (-{h},{h}), (-{h},-{h})]
]
cavities = [[["bore", ({r},0), (-{r},0), {{"center": (0,0)}}], ["bore_back", (-{r},0), ({r},0), {{"center": (0,0)}}]]]
source = "synthetic test section, not supplier geometry"
"#,
        r = BORE_RADIUS
    )
}

fn area(width: f64) -> f64 {
    let n = width / 8.0;
    width * width - n * n - PI * BORE_RADIUS * BORE_RADIUS
}

fn profile(designation: &str, width: f64, length: f64, placement: &str) -> String {
    format!(
        "{}\np = slot_profile(\"bar\", \"{designation}\", {length}, section=section, section_source=source, cavities=cavities{placement})",
        section(width)
    )
}

#[test]
fn slot_profile_keeps_catalogue_size_and_refuses_a_section_it_cannot_publish() {
    let source = format!(
        r#"
d = slot_profile_dimensions("Motedis-20x20-I5")
if d["width_mm"] != 20 or d["height_mm"] != 20:
    fail("incorrect catalogue size")
if d["section"] != None or d["bore_mm"] != None:
    fail("Motedis terms forbid publishing the CAD section and its bore")
if d["source"] != "https://www.motedis.com/en/Aluminium-Profile-20x20-I-Typ-slot-5":
    fail("missing product page citation")
{}
"#,
        profile("Motedis-20x20-I5", 20.0, 125.0, "")
    );
    let model = ketchup_program::run("profile.star", &source, &BTreeMap::new())
        .unwrap()
        .0
        .model;
    let bom = ketchup_program::bom::bom(&model);
    assert!(bom.cut_list.is_empty());
    assert_eq!(bom.hardware.len(), 1);
    assert_eq!(bom.hardware[0].count, 1);
    assert!(bom.hardware[0].item.contains("Motedis-20x20-I5"));
    let attributes = &bom.hardware[0].attributes;
    assert_eq!(attributes["length_mm"].parse::<f64>().unwrap(), 125.0);
    assert!(attributes["geometry_basis"].contains("caller-supplied section"));
    assert!(attributes["section_source"].contains("synthetic test section"));
    let synthetic = section(20.0);
    let wide = section(30.0);
    for call in [
        "slot_profile_dimensions('generic-2020')".to_owned(),
        "slot_profile_dimensions('Motedis-30x30-I5')".to_owned(),
        "slot_profile('bar','Motedis-20x20-I5',125)".to_owned(),
        format!(
            "{synthetic}\nslot_profile('bar','Motedis-20x20-I5',125,section=section,cavities=cavities)"
        ),
        format!(
            "{synthetic}\nslot_profile('bar','Motedis-20x20-I5',125,section=section,section_source=' ',cavities=cavities)"
        ),
        format!(
            "{synthetic}\nslot_profile('bar','Motedis-20x20-I5',125,section=section,section_source=source)"
        ),
        format!(
            "{wide}\nslot_profile('bar','Motedis-20x20-I5',125,section=section,section_source=source,cavities=cavities)"
        ),
        format!(
            "{synthetic}\nslot_profile('bar','Motedis-20x20-I5',0,section=section,section_source=source,cavities=cavities)"
        ),
        format!(
            "{synthetic}\nslot_profile('bar','Motedis-20x20-I5',-1,section=section,section_source=source,cavities=cavities)"
        ),
    ] {
        assert!(
            ketchup_program::run("invalid.star", &call, &BTreeMap::new()).is_err(),
            "{call}"
        );
    }
}

#[test]
fn slot_profile_exact_volume_keeps_the_supplied_section_and_cavities_at_each_length() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (designation, width) in [
        ("Motedis-20x20-I5", 20.0),
        ("Motedis-30x30L-I6", 30.0),
        ("Motedis-40x40L-I8", 40.0),
    ] {
        for length in [25.0, 300.0] {
            let source = profile(designation, width, length, "");
            assert_volume(volume(&mut worker, &source, "bar"), area(width) * length);
        }
    }
}

#[test]
fn slot_profile_components_keep_bom_and_rotated_step_after_save_open() {
    check_components("Motedis-20x20-I5", 20.0);
    check_components("Motedis-30x30L-I6", 30.0);
    check_components("Motedis-40x40L-I8", 40.0);
}

fn check_components(designation: &str, width: f64) {
    let _turn = crate::integration_support::file_turn();
    let source = format!(
        "{}\nc=component(\"frame member\",[p])\ninstance(\"second member\",c,at=(0,0,50))\n",
        profile(designation, width, 125.0, ",at=(10,20,30),axis=(1,0,0)")
    );
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
    assert_volume(imported.volume_mm3, 2.0 * area(width) * 125.0);
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
