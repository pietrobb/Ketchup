//! Two parts of one bought assembly that overlap where its file places them
//! are a contact in that assembly, not a collision; moved apart from where the
//! file puts them, they collide as any two parts do.
use super::*;
use crate::exact::{ExactPair, ExactShapes};
use crate::validate::{PURCHASED_ASSEMBLY_CONTACT, validate_with};
use crate::{OverlapStatus, relations_with};
use ketchup_model::document::BodyKind;
use ketchup_model::import::ImportLengthUnit;
use std::collections::BTreeMap;

/// A hinge and its mounting plate from one file, both placed with the
/// file's frame at (100, 0, 500); a spare plate 10 mm higher.
const KIT: &str = "purchased('hinge', 'kit.step', '1', 'Test', part = 'Hinge', at = (100, 0, 500))\n\
    purchased('plate', 'kit.step', '2', 'Test', part = 'Plate', at = (100, 0, 500))\n\
    purchased('loose', 'kit.step', '2', 'Test', part = 'Spare plate', at = (100, 0, 510))\n";

fn evidence(bounds_mm: [[f64; 3]; 2]) -> StepImportEvidence {
    StepImportEvidence {
        source_sha256: [7; 32],
        source_byte_len: 4_096,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 2_200.0,
        volume_mm3: 6_000.0,
        bounds_mm,
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    }
}

/// The kit as the host reads it: the hinge turned half a turn about z and
/// moved 10 mm along x by the assembly, each plate 5 mm along y; every body
/// 40 x 40 x 20 mm about its own origin, so all three overlap.
fn kit() -> ProgramModel {
    let mut model = crate::evaluate("kit.star", KIT, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    let half_turn = Transform::from_matrix([
        -1.0, 0.0, 0.0, 10.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    for (index, part) in model.imported_parts_mut().enumerate() {
        let placement = if part.name == "hinge" {
            half_turn
        } else {
            Transform::from_translation(0.0, 5.0, 0.0).unwrap()
        };
        ProgramImport::record(
            part,
            ImportedBody {
                file_name: "kit.step".to_owned(),
                part_index: Some(u32::try_from(index).unwrap()),
                evidence: evidence([[-20.0, -20.0, -10.0], [20.0, 20.0, 10.0]]),
                placement: Some(placement),
            },
        );
    }
    model
}

/// The exact kernel's answer for the kit: every two of its solids overlap.
fn overlapping(model: &ProgramModel) -> ExactShapes {
    let names: Vec<String> = model.parts.iter().map(|part| part.name.clone()).collect();
    let mut exact = ExactShapes::default();
    exact.checked_against(names.clone(), names.clone());
    let collision = ExactPair {
        common_volume_mm3: 55.8,
        common_thickness_mm: 2.0,
        ..ExactPair::default()
    };
    for (index, a) in names.iter().enumerate() {
        for b in &names[index + 1..] {
            exact.insert(a, b, collision);
        }
    }
    exact
}

fn pairs<'a>(issues: &'a [crate::Issue], kind: &str) -> Vec<Vec<&'a str>> {
    let mut found: Vec<Vec<&str>> = issues
        .iter()
        .filter(|issue| issue.kind == kind)
        .map(|issue| {
            let mut pair: Vec<&str> = issue.parts.iter().map(String::as_str).collect();
            pair.sort_unstable();
            pair
        })
        .collect();
    found.sort();
    found
}

#[test]
fn parts_where_their_assembly_file_places_them_are_a_contact_not_a_collision() {
    let model = kit();
    let part = |name: &str| model.part(name).unwrap();
    assert!(same_assembly_placement(part("hinge"), part("plate")));
    // The spare plate's file frame lies 10 mm higher.
    assert!(!same_assembly_placement(part("hinge"), part("loose")));
    assert!(!same_assembly_placement(part("plate"), part("loose")));

    let exact = overlapping(&model);
    let issues = validate_with(&model, &exact);
    assert_eq!(
        pairs(&issues, PURCHASED_ASSEMBLY_CONTACT),
        [["hinge", "plate"]],
        "{issues:#?}"
    );
    assert_eq!(
        pairs(&issues, "collision"),
        [["hinge", "loose"], ["loose", "plate"]],
        "{issues:#?}"
    );
    let contact = issues
        .iter()
        .find(|issue| issue.kind == PURCHASED_ASSEMBLY_CONTACT)
        .unwrap();
    assert_eq!(contact.severity, crate::Severity::Warning);
    assert!(contact.message.contains("55.8 mm³"), "{}", contact.message);

    let relations = relations_with(&model, &issues, &exact);
    let status = |a: &str, b: &str| {
        relations
            .iter()
            .find(|relation| relation.parts == [a.to_owned(), b.to_owned()])
            .and_then(|relation| relation.status)
    };
    assert_eq!(
        status("hinge", "plate"),
        Some(OverlapStatus::PurchasedAssembly)
    );
    assert_eq!(status("hinge", "loose"), Some(OverlapStatus::Collision));
}

#[test]
fn one_part_of_a_file_twice_or_a_whole_file_is_no_assembly_contact() {
    let mut model = kit();
    // The spare plate named as the plate is: the same part placed twice.
    let mut twin = model.part("plate").unwrap().clone();
    twin.name = "twin".to_owned();
    let plate = model.part("plate").unwrap();
    assert!(!same_assembly_placement(plate, &twin));
    // A whole file has no assembly placement to compare.
    if let ProgramPartBody::Imported(import) = &mut model.parts[0].body {
        import.part = None;
    }
    assert!(!same_assembly_placement(&model.parts[0], &model.parts[1]));
}

/// A beam on two posts with a steel block 100 x 80 x 100 mm on it, 200 mm
/// along, read from a STEP file (review 2026-11-24 R1124og-STEP-902/904).
const BLOCK_ON_BEAM: &str = "load_path(only = ['frame'])\n\
    material_weight('steel', kg_m3 = 7850, source = 'test')\n\
    weight_scope(['heavy'])\n\
    box('post a', (100, 80, 1000), material = 'C24', grounded = True)\n\
    box('post b', (100, 80, 1000), at = (2900, 0, 0), material = 'C24', grounded = True)\n\
    box('beam', (3000, 80, 200), at = (0, 0, 1000), material = 'C24', tags = ['frame'])\n\
    imported('block', 'block.step', at = (200, 0, 1200), material = 'steel', tags = ['heavy'])\n";

fn block_on_beam() -> ProgramModel {
    crate::evaluate("block.star", BLOCK_ON_BEAM, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model
}

fn beam_load(report: &crate::loads::LoadReport) -> &crate::loads::MemberLoad {
    report
        .members
        .iter()
        .find(|member| member.part == "beam")
        .unwrap_or_else(|| panic!("{report:#?}"))
}

#[test]
fn a_bought_part_not_read_yet_is_unchecked_not_weightless_or_floating() {
    let model = block_on_beam();
    let report = crate::loads::loads(&model);
    let beam = beam_load(&report);
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.contains("block") && reason.contains("not read")),
        "{beam:#?}"
    );
    assert!(
        report
            .unassigned
            .get("block")
            .is_some_and(|reason| reason.contains("block.step")),
        "{report:#?}"
    );
    let issues = crate::validate::validate(&model);
    assert!(
        !issues
            .iter()
            .any(|issue| issue.kind == "floating_part" && issue.parts == ["block"]),
        "{issues:#?}"
    );
}

#[test]
fn a_bought_part_weighs_its_own_volume_not_its_box() {
    let mut model = block_on_beam();
    let block = model.imported_parts_mut().next().unwrap();
    ProgramImport::record(
        block,
        ImportedBody {
            file_name: "block.step".to_owned(),
            part_index: None,
            evidence: StepImportEvidence {
                volume_mm3: 200_000.0,
                ..evidence([[0.0; 3], [100.0, 80.0, 100.0]])
            },
            placement: None,
        },
    );
    // The statics finds no face of a read body to stand on and says so with
    // the load it leaves out: 200 000 mm³ of steel is 15.4 N, where its box
    // of 800 000 mm³ would be 61.6 N.
    let report = crate::loads::loads(&model);
    let beam = beam_load(&report);
    assert!(
        beam.missing
            .iter()
            .any(|reason| reason.starts_with("block ") && reason.contains("its 0.02 kN")),
        "{beam:#?}"
    );
}
