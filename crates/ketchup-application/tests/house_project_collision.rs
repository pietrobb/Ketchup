//! The demo house (examples/programs/house-project.star, 1855 parts, 1813 timber members) has
//! no overlap in the exact collision check: every joint is cut into the receiving
//! member. Every timber member exports to BTLx as a stock box with its cuts.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_all};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const HOUSE: &str = include_str!("../../../examples/programs/house-project.star");

const BUDGET: Duration = Duration::from_secs(if cfg!(debug_assertions) { 1200 } else { 300 });

#[test]
#[ignore = "release-only: about a minute of exact geometry; run with --ignored"]
fn the_demo_house_has_no_collisions_in_the_exact_check() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "house-project.star".to_owned(),
                source: HOUSE.to_owned(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let mut report = applied.report;
    let summary = verify_rule_program_all(
        &applied.snapshot,
        &applied.model,
        &mut report,
        &ContainerData::default(),
        None,
        BUDGET,
        Arc::new(AtomicBool::new(false)),
    )
    .expect("the house has a collision summary");
    assert_eq!(summary["state"], "verified", "{summary:#}");
    assert_eq!(summary["collisions"], 0, "{summary:#}");
    assert_eq!(summary["unresolved"], 0, "{summary:#}");
}

/// Every timber member of the house is a stock box with cuts that BTLx describes,
/// and the export of the whole house passes its own collision check.
#[test]
#[ignore = "release-only: exact geometry of the whole house; run with --ignored"]
fn the_demo_house_exports_every_timber_member_to_btlx() {
    use ketchup_application::validation::fabrication_collision_validation_with_worker;
    use ketchup_manufacturing::fabrication::{
        BtlxExportOptions, BtlxProfileProcessingRequest, GeneralBomItemKind,
        project_general_fabrication,
    };
    use ketchup_model::exact_brep_graph::ExactBRepGraph;
    use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
    use ketchup_model::exact_validation::GeneralBodyParticipant;

    let mut session = DocumentSession::new(SessionSettings::default());
    let snapshot = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "house-project.star".to_owned(),
                source: HOUSE.to_owned(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap()
        .snapshot;
    let mut worker = crate::operations_support::worker();
    let occurrences = snapshot.scene_query();
    let packages = occurrences
        .iter()
        .map(|occurrence| {
            let definition = snapshot.definition(occurrence.definition_id).unwrap();
            let graph = ExactBRepGraph::from_snapshot(
                &snapshot,
                occurrence.definition_id,
                *definition.feature_ids().last().unwrap(),
            )
            .unwrap();
            std::sync::Arc::new(ExactBodyPackage::from(
                worker.evaluate_exact_brep_graph(&graph).unwrap(),
            ))
        })
        .collect::<Vec<_>>();
    let registry = ExactResultRegistry::accept(&snapshot, packages).unwrap();
    let tolerance = snapshot.tolerance();
    let participants = occurrences
        .iter()
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                occurrence.instance_path.clone(),
                tolerance,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let validation = fabrication_collision_validation_with_worker(
        &snapshot,
        &participants,
        &ContainerData::default(),
        None,
        BUDGET,
    )
    .unwrap();
    assert_eq!(validation.cases.len(), participants.len() - 1);
    let projection = project_general_fabrication(
        &snapshot,
        &registry,
        &validation.cases,
        &validation.report,
        tolerance,
    )
    .unwrap();
    assert!(
        projection.manufacturing.unresolved_sources.is_empty(),
        "{} members are not a stock box with cuts",
        projection.manufacturing.unresolved_sources.len()
    );
    let timber = projection
        .bom
        .rows
        .iter()
        .filter(|row| row.item_kind == GeneralBomItemKind::Timber)
        .map(|row| row.quantity)
        .sum::<usize>();
    // The program's own count of timber members, not a number kept by hand.
    let members = ketchup_program::evaluate("house-project.star", HOUSE, &Default::default())
        .unwrap()
        .model
        .parts
        .iter()
        .filter(|part| {
            part.attributes
                .get("classification:ketchup.fabrication-role.v1")
                .is_some_and(|role| role == "fabrication.timber-member.v1")
        })
        .count();
    assert!(members > 1800, "{members}");
    assert_eq!(timber, members);
    // the house is larger than the unscoped collision scene (512 occurrences)
    assert_eq!(
        validation.report.state,
        ketchup_model::validation::ValidationState::Passed,
        "{:?}",
        validation.report.unresolved_conditions
    );
    let btlx = projection
        .btlx_2_3_1_export_with_options(
            &snapshot,
            BtlxExportOptions {
                profile_processing_request: BtlxProfileProcessingRequest::PortableFreeContour,
            },
        )
        .unwrap();
    let xml = String::from_utf8(btlx).unwrap();
    let pieces = xml
        .split("<Part ")
        .skip(1)
        .map(|part| {
            let part = format!(" {part}");
            let start = part.find(" Count=\"").unwrap() + 8;
            part[start..start + part[start..].find('"').unwrap()]
                .parse::<usize>()
                .unwrap()
        })
        .sum::<usize>();
    assert_eq!(pieces, members);
    // Every processing lies in its blank up to the air of a through cut. A
    // cutter that is a neighbouring member's whole section (a clamp against
    // a sloped rafter, a horizontal beam notch in a rafter) is not clipped to
    // the member it cuts and reaches further out; each of those is pinned
    // with its overshoot. A processing in a wrong frame leaves the blank by a
    // large part of a member dimension or changes a pinned overshoot.
    let pinned = crate::btlx_blank::btlx_machining_overshoots(&xml)
        .into_iter()
        .filter(|(_, _, overshoot)| *overshoot > crate::btlx_blank::HOUSE_CUTTER_MARGIN_MM)
        .map(|(designation, ordinal, overshoot)| {
            format!("{designation}\t{ordinal}\t{overshoot:.1}\n")
        })
        .collect::<String>();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/house-project-btlx-overshoots.tsv");
    if ketchup_test_env::update_golden() {
        std::fs::write(&path, &pinned).unwrap();
    }
    assert_eq!(
        pinned,
        std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n"),
        "processings reaching more than {} mm out of their blank",
        crate::btlx_blank::HOUSE_CUTTER_MARGIN_MM
    );
}
