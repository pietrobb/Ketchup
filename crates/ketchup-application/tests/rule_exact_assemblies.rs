//! Exact reports distinguish leaves even when they share a component root.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_exact};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;
use ketchup_program::{COLLISION_UNVERIFIED, RelationKind, Report};
use std::collections::BTreeSet;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const PARTS: &str = r#"
t = extrude("t", profile=[(0,0),(100,0),(0,100)], distance=20)
c = box("clear", (20,20,20), at=(70,70,0))
"#;

fn checked(source: String, cancelled: bool) -> (Report, serde_json::Value) {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "exact-assemblies.star".into(),
                source,
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let mut report = applied.report;
    let summary = verify_rule_program_exact(
        &applied.snapshot,
        &applied.model,
        &mut report,
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        Arc::new(AtomicBool::new(cancelled)),
    )
    .expect("non-box pairs require exact validation");
    (report, summary)
}

fn collision_pairs(report: &Report) -> BTreeSet<Vec<String>> {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .map(|issue| {
            let mut parts = issue.parts.clone();
            parts.sort();
            parts
        })
        .collect()
}

fn expected_pairs(pairs: &[&[&str]]) -> BTreeSet<Vec<String>> {
    pairs
        .iter()
        .map(|pair| {
            let mut names = pair
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>();
            names.sort();
            names
        })
        .collect()
}

fn assert_verified(report: &Report, summary: &serde_json::Value) {
    assert_eq!(summary["state"], "verified", "{summary}");
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED),
        "{report:#?}"
    );
}

#[test]
fn disconnected_shared_groups_use_native_solids_and_cancellation_stays_unverified() {
    let _turn = crate::integration_support::file_turn();
    let source =
        format!("{PARTS}\nk=component('k',[t,c])\ninstance('copy',k,at=(300,0,0),x=(0,1,0))");
    let (report, summary) = checked(source.clone(), false);
    assert_verified(&report, &summary);
    let disconnected = report
        .issues
        .iter()
        .filter(|i| i.kind == "disconnected_group")
        .collect::<Vec<_>>();
    assert_eq!(disconnected.len(), 2, "{report:?}");
    assert!(disconnected.iter().any(|i| i.parts == ["clear", "t"]));
    assert!(
        disconnected
            .iter()
            .any(|i| i.parts == ["copy/clear", "copy/t"])
    );
    assert!(
        !report
            .issues
            .iter()
            .any(|i| i.kind == "group_contact_unverified")
    );
    let (fixed, summary) = checked(source.replace("(70,70,0)", "(0,0,20)"), false);
    assert_verified(&fixed, &summary);
    assert!(
        !fixed
            .issues
            .iter()
            .any(|i| matches!(i.kind, "disconnected_group" | "group_contact_unverified")),
        "{fixed:?}"
    );
    let (cancelled, summary) = checked(source, true);
    assert_eq!(summary["state"], "incomplete", "{summary}");
    assert_eq!(summary["unverified_groups"], 2, "{summary}");
    assert!(
        cancelled
            .issues
            .iter()
            .any(|i| i.kind == "group_contact_unverified")
    );
    assert!(
        !cancelled
            .issues
            .iter()
            .any(|i| i.kind == "disconnected_group")
    );
}

#[test]
fn shared_boolean_rebuild_tracks_member_and_tool_placement_in_every_instance() {
    let _turn = crate::integration_support::file_turn();
    let source = |offset: f64, cut: f64| RuleProgramSource {
        file_name: "shared-boolean.star".into(),
        source: format!(
            "p=box(\"p\",(100,60,40),at=({offset},0,0))\nt=box(\"tool\",(20,60,10),at=({},0,30),tool=True)\nsubtract(p,t)\na=box(\"left\",(1,1,1),at=({},5,35))\nb=box(\"middle\",(1,1,1),at=({},5,35))\nc=component(\"assembly\",[p,a,b])\ninstance(\"copy\",c,at=(400,500,0),x=(0,1,0))\n",
            offset + cut,
            offset + 5.0,
            offset + 45.0
        ),
        overrides: Default::default(),
    };
    let verify = |applied: &ketchup_application::RuleProgramApplyResult| {
        let exact = ketchup_application::validation::assistant_validation_context_with_worker(
            &applied.snapshot,
            &ketchup_model::exact_product::ExactResultRegistry::default(),
            &ketchup_application::AssistantValidationSelection::only(&["collision"]),
            &ContainerData::default(),
            None,
            Duration::from_secs(60),
        );
        assert_eq!(exact["complete"], true, "{exact}");
        assert_eq!(exact["issue_count"], 2, "{exact}");
        exact["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| {
                assert_eq!(issue["evidence_class"], "exact", "{issue}");
                let mut names = vec![
                    issue["left_name"].as_str().unwrap().to_owned(),
                    issue["right_name"].as_str().unwrap().to_owned(),
                ];
                names.sort();
                names
            })
            .collect::<BTreeSet<_>>()
    };
    let mut session = DocumentSession::new(SessionSettings::default());
    let initial = session
        .apply_rule_program(source(10.0, 0.0), false)
        .unwrap();
    assert_eq!(verify(&initial), expected_pairs(&[&["p", "middle"]]));
    let moved = session
        .apply_rule_program(source(30.0, 40.0), false)
        .unwrap();
    assert_eq!(verify(&moved), expected_pairs(&[&["p", "left"]]));
    for member in initial.snapshot.local_occurrences() {
        let updated = moved.snapshot.local_occurrence(member.key()).unwrap();
        if member.name() == "p" {
            assert_ne!(updated.definition_id(), member.definition_id());
            assert!(moved.snapshot.definition(member.definition_id()).is_none());
        } else {
            assert_eq!(updated.definition_id(), member.definition_id());
        }
    }
    let mut fresh = DocumentSession::new(SessionSettings::default());
    assert_eq!(
        verify(&fresh.apply_rule_program(source(30.0, 40.0), false).unwrap()),
        verify(&moved)
    );
    assert_eq!(
        session.undo().unwrap().scene_query(),
        initial.snapshot.scene_query()
    );
    assert_eq!(
        session.redo().unwrap().scene_query(),
        moved.snapshot.scene_query()
    );
    let tool_only = session
        .apply_rule_program(source(30.0, 0.0), false)
        .unwrap();
    assert_eq!(verify(&tool_only), expected_pairs(&[&["p", "middle"]]));
}

#[test]
fn grouped_profiles_keep_exact_collision_and_empty_corner_answers() {
    let _turn = crate::integration_support::file_turn();
    let (report, summary) = checked(
        format!(
            "{PARTS}\nh = box(\"hit\", (20,20,20), at=(10,10,0))\ngroup(\"outer\", [group(\"inner\", [t,c]), h])\n"
        ),
        false,
    );
    assert_verified(&report, &summary);
    assert_eq!(collision_pairs(&report), expected_pairs(&[&["t", "hit"]]));
    assert_eq!(summary["collisions"], 1);
    assert_eq!(summary["cleared"], 1);
}

#[test]
fn shared_component_leaves_have_distinct_exact_pairs_in_each_rotated_instance() {
    let _turn = crate::integration_support::file_turn();
    let (report, summary) = checked(
        format!(
            "{PARTS}\nh = box(\"hit\", (20,20,20), at=(10,10,0))\na = component(\"assembly\", [group(\"inner\", [t,c]), h])\ninstance(\"copy\", a, at=(400,500,0), x=(0,1,0))\n"
        ),
        false,
    );
    assert_verified(&report, &summary);
    assert_eq!(
        collision_pairs(&report),
        expected_pairs(&[&["t", "hit"], &["copy/t", "copy/hit"]])
    );
    assert_eq!(summary["collisions"], 2);
    assert_eq!(summary["cleared"], 2);
}

#[test]
fn external_contact_and_overlap_belong_to_the_correct_shared_leaf() {
    let _turn = crate::integration_support::file_turn();
    let (report, summary) = checked(
        format!(
            "{PARTS}\na = component(\"assembly\", [t,c])\ni = instance(\"copy\", a, at=(400,500,0), x=(0,1,0))\np = box(\"probe\", (20,20,20), at=(360,510,0))\ns = box(\"supported\", (20,20,20), at=(360,510,20))\ngroup(\"all\", [a,i,p,s])\n"
        ),
        false,
    );
    assert_verified(&report, &summary);
    assert_eq!(
        collision_pairs(&report),
        expected_pairs(&[&["copy/t", "probe"]])
    );
    let contact = report
        .relations
        .iter()
        .find(|relation| {
            let names = relation
                .parts
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            names == BTreeSet::from(["copy/t", "supported"])
        })
        .expect("exact contact with the transformed triangle");
    assert_eq!(contact.kind, RelationKind::Contact, "{contact:?}");
    assert_eq!(contact.area_mm2, Some(400.0), "{contact:?}");
    assert!(!contact.approx);
}

#[test]
fn cancelled_component_checks_do_not_clear_unmeasured_leaf_overlaps() {
    let _turn = crate::integration_support::file_turn();
    let (report, summary) = checked(
        format!(
            "{PARTS}\na = component(\"assembly\", [t,c])\ninstance(\"copy\", a, at=(400,0,0))\n"
        ),
        true,
    );
    assert_eq!(summary["state"], "incomplete", "{summary}");
    assert_eq!(summary["cleared"], 0);
    assert_eq!(
        report
            .issues
            .iter()
            .filter(|issue| issue.kind == COLLISION_UNVERIFIED)
            .count(),
        2
    );
    assert!(collision_pairs(&report).is_empty());
}
