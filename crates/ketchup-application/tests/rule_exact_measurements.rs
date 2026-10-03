//! Clearance expectations use the machined solids, including separated pairs.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_exact};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

fn checked(source: String) -> (ketchup_program::Report, serde_json::Value) {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "clearance.star".into(),
                source,
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let mut report = applied.report;
    let exact = verify_rule_program_exact(
        &applied.snapshot,
        &applied.model,
        &mut report,
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        Arc::new(AtomicBool::new(false)),
    )
    .expect("declared clearance must be measured");
    (report, exact)
}

#[test]
fn modular_cabinet_exact_check_measures_all_eight_openings() {
    let _turn = crate::integration_support::file_turn();
    let started = std::time::Instant::now();
    let (report, exact) =
        checked(include_str!("../../../examples/programs/modular-doweled-cabinet.star").into());
    eprintln!("88-part native check {:?}: {exact}", started.elapsed());
    assert_eq!(report.bom.total_parts, 88);
    assert_eq!(exact["state"], "verified", "{exact}");
    assert_eq!(exact["collisions"], 0, "{exact}");
    assert_eq!(exact["measurements_total"], 8, "{exact}");
    for measurement in exact["measurements"].as_array().unwrap() {
        assert_eq!(measurement["state"], "verified", "{measurement}");
        let right_lower = measurement["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part == "lower 3 bottom");
        assert_eq!(
            measurement["distance_mm"],
            if right_lower { 650.0 } else { 400.0 },
            "{measurement}"
        );
    }
    assert!(report.issues.is_empty(), "{:?}", report.issues);
}

#[test]
fn parallel_graph_ranges_keep_collisions_and_distance_failures_in_the_second_range() {
    let _turn = crate::integration_support::file_turn();
    let (report, exact) = checked("for i in range(20):\n    p=box('outer '+str(i),(100,100,18),at=(i*200,0,0))\n    hole(p,'z+',at=(50,50),diameter=20,depth=6 if i==15 else 12)\n    q=box('inner '+str(i),(4,4,4),at=(i*200+48,48,10))\n    expect_gap(p,q,4)\n".into());
    assert_eq!(exact["state"], "verified", "{exact}");
    assert_eq!(exact["collisions"], 1, "{exact}");
    assert_eq!(exact["measurements_total"], 20, "{exact}");
    for measurement in exact["measurements"].as_array().unwrap() {
        let shallow = measurement["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "outer 15");
        assert_eq!(
            measurement["distance_mm"],
            if shallow { 0.0 } else { 4.0 },
            "{measurement}"
        );
    }
    let failed: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "expectation_failed")
        .collect();
    assert_eq!(failed.len(), 1, "{:?}", report.issues);
    assert!(failed[0].parts.iter().any(|name| name == "outer 15"));
}

#[test]
fn a_body_inside_a_hole_or_pocket_has_four_mm_clearance() {
    let _turn = crate::integration_support::file_turn();
    for machining in [
        "hole(p, 'z+', at=(50,50), diameter=20, depth=12)",
        "pocket(p, 'z+', rect=(40,40,60,60), depth=12)",
        "tool=box('tool',(20,20,12),at=(40,40,6),tool=True)\nsubtract(p,tool)",
    ] {
        let (report, exact) = checked(format!(
            "p=box('p',(100,100,18))\n{machining}\n\
             q=box('q',(4,4,4),at=(48,48,10))\nexpect_gap(p,q,4)\n"
        ));
        assert_eq!(exact["state"], "verified", "{exact}");
        assert!(
            !report.issues.iter().any(|issue| matches!(
                issue.kind,
                "expectation_failed"
                    | "expectation_unverified"
                    | "collision"
                    | "collision_unverified"
            )),
            "{machining}: {report:?}"
        );
        let relation = report.relations.iter().find(|r| r.gap_mm == Some(4.0));
        assert!(relation.is_some(), "{report:?}");
    }
}

#[test]
fn separated_parts_keep_the_650_mm_measurement_after_unrelated_drilling_is_removed() {
    let _turn = crate::integration_support::file_turn();
    for machining in ["hole(p,'z-',at=(50,50),diameter=8,depth=12)", ""] {
        for required in [650, 649] {
            let (report, exact) = checked(format!(
                "p=box('bottom',(100,100,18))\n{machining}\n\
                 q=box('shelf',(100,100,18),at=(0,0,668))\n\
                 expect_gap(p,q,{required})\n"
            ));
            assert_eq!(exact["state"], "verified", "{exact}");
            assert_eq!(exact["distance_measurements_state"], "verified", "{exact}");
            assert_eq!(exact["measurements"][0]["distance_mm"], 650.0, "{exact}");
            assert!(
                !report
                    .issues
                    .iter()
                    .any(|i| i.kind == "expectation_unverified"),
                "{report:?}"
            );
            let failed = report
                .issues
                .iter()
                .find(|i| i.kind == "expectation_failed");
            assert_eq!(failed.is_some(), required != 650, "{report:?}");
            if let Some(failed) = failed {
                assert!(failed.message.contains("measured 650"), "{failed:?}");
            }
        }
    }
}

#[test]
fn a_shallow_groove_reports_collision_instead_of_inheriting_the_deep_groove_clearance() {
    let _turn = crate::integration_support::file_turn();
    for depth in [12, 6] {
        let (report, exact) = checked(format!(
            "p=box('p',(100,100,18))\n\
             pocket(p,'z+',rect=(40,-1,60,101),depth={depth})\n\
             q=box('q',(4,4,4),at=(48,48,10))\nexpect_gap(p,q,4)\n"
        ));
        assert_eq!(exact["state"], "verified", "{exact}");
        assert_eq!(
            exact["measurements"][0]["distance_mm"],
            if depth == 12 { 4.0 } else { 0.0 },
            "{exact}"
        );
        assert_eq!(
            exact["collisions"],
            if depth == 12 { 0 } else { 1 },
            "{exact}"
        );
        assert_eq!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == "expectation_failed"),
            depth == 6,
            "{report:?}"
        );
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.kind.ends_with("_unverified")),
            "{report:?}"
        );
    }
}
