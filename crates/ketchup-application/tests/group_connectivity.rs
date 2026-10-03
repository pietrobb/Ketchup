use ketchup_application::validation::{
    assistant_validation_context, assistant_validation_context_with_worker_cancellation,
};
use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionSettings,
};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, RuleProgramSource, Snapshot, Transform,
};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::persistence::ContainerData;
use serde_json::Value;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

fn session(source: &str) -> DocumentSession {
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "connectivity.star".into(),
                source: source.into(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    session
}

fn check(snapshot: &Snapshot, cancelled: bool) -> Value {
    assistant_validation_context_with_worker_cancellation(
        snapshot,
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["group_connectivity"]),
        &ContainerData::default(),
        None,
        Duration::from_secs(60),
        Arc::new(AtomicBool::new(cancelled)),
    )
}

#[test]
fn typed_repair_detaches_program_but_group_contact_survives_history_and_reopen() {
    let _turn = crate::integration_support::file_turn();
    let mut session = session(
        "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(30,0,0))\ngroup('g',[a,b],grounded=True)\nbox('outside',(20,10,10),at=(10,0,0))",
    );
    let before = session.snapshot();
    let disconnected = check(&before, false);
    assert_eq!(disconnected["complete"], true, "{disconnected:#}");
    assert_eq!(disconnected["state"], "failed", "{disconnected:#}");
    assert_eq!(disconnected["issue_count"], 1);
    assert_eq!(disconnected["collision"]["state"], "skipped");
    let issue = &disconnected["group_connectivity"]["issues"][0];
    assert_eq!(issue["group"]["name"], "g");
    assert_eq!(issue["names"], serde_json::json!(["a", "b"]));
    assert_eq!(issue["code"], "group.disconnected");
    assert_eq!(issue["islands"].as_array().unwrap().len(), 2);
    assert!(!issue.to_string().contains("outside"));
    let id = before
        .occurrences()
        .find(|part| part.name() == "b")
        .unwrap()
        .id();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id,
                transform: Transform::from_translation(10., 0., 0.).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert!(session.rule_program().is_none());
    let repaired = check(&session.snapshot(), false);
    assert_eq!(repaired["state"], "passed", "{repaired:#}");
    assert_eq!(repaired["issue_count"], 0);
    assert_eq!(repaired["complete"], true);
    session.undo().unwrap();
    assert_eq!(check(&session.snapshot(), false)["issue_count"], 1);
    session.redo().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("detached.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert!(reopened.rule_program().is_none());
    let report = check(&reopened.snapshot(), false);
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true);
}

#[test]
fn native_nested_groups_use_full_paths_and_actual_solids_not_overlapping_boxes() {
    let _turn = crate::integration_support::file_turn();
    let session = session(
        "t=extrude('t',profile=[(0,0),(100,0),(0,100)],distance=20)\nc=box('clear',(20,20,20),at=(70,70,0))\ng=group('g',[t,c])\nk=component('k',[g],grounded=True)\ninstance('copy',k,at=(300,0,0),x=(0,1,0))",
    );
    let report = check(&session.snapshot(), false);
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["state"], "failed");
    assert_eq!(report["group_connectivity"]["issue_count"], 4, "{report:#}");
    for issue in report["group_connectivity"]["issues"].as_array().unwrap() {
        assert_eq!(issue["islands"].as_array().unwrap().len(), 2);
        let leaves = issue["islands"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|island| island.as_array().unwrap())
            .collect::<Vec<_>>();
        let roots = leaves
            .iter()
            .map(|leaf| leaf["instance_path"]["root"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(roots[0], roots[1]);
        assert_ne!(leaves[0]["instance_path"], leaves[1]["instance_path"]);
        assert!(
            leaves
                .iter()
                .all(|leaf| leaf["origin_mm"].as_array().unwrap().len() == 3)
        );
    }
    assert!(report.to_string().contains("copy/clear"));
    assert_eq!(
        report["group_connectivity"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|issue| issue["group"]["name"] == "g")
            .count(),
        2
    );
}

#[test]
fn cancelled_or_non_native_contact_is_incomplete_not_disconnected_or_passed() {
    let _turn = crate::integration_support::file_turn();
    let session =
        session("a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(10,0,0))\ngroup('g',[a,b])");
    let snapshot = session.snapshot();
    for report in [
        check(&snapshot, true),
        assistant_validation_context(
            &snapshot,
            &ExactResultRegistry::default(),
            &AssistantValidationSelection::only(&["group_connectivity"]),
        ),
    ] {
        assert_eq!(report["state"], "not_evaluated", "{report:#}");
        assert_eq!(report["complete"], false);
        assert_eq!(report["issue_count"], 0);
        assert_eq!(report["group_connectivity"]["complete"], false);
        assert_eq!(report["collision"]["state"], "skipped");
        assert!(!report["not_evaluated"].as_array().unwrap().is_empty());
    }
}
