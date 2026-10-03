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

fn apply(session: &mut DocumentSession, source: &str) {
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "contact.star".into(),
                source: source.into(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
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
fn edit(session: &mut DocumentSession, command: CanonicalCommand) {
    let proposal = session
        .plan_commands(CommandBatch::new(vec![command]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
}
const SOURCE: &str = "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(30,0,0))\njoint(a,b,kind='fixed',name='physical',max_gap=2)";

#[test]
fn physical_joint_survives_detachment_repair_history_reopen_and_endpoint_deletion() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let bad = check(&session.snapshot(), false);
    assert_eq!(bad["complete"], true, "{bad:#}");
    assert_eq!(bad["issue_count"], 1, "{bad:#}");
    let issue = &bad["issues"][0];
    assert_eq!(issue["kind"], "joint_without_contact");
    assert_eq!(issue["names"], serde_json::json!(["a", "b"]));
    assert_eq!(issue["distance_mm"], 20.);
    assert_eq!(issue["max_gap_mm"], 2.);
    assert_eq!(
        issue["origin_mm"],
        serde_json::json!([[0., 0., 0.], [30., 0., 0.]])
    );
    let snapshot = session.snapshot();
    let b = snapshot
        .occurrences()
        .find(|part| part.name() == "b")
        .unwrap()
        .id();
    edit(
        &mut session,
        CanonicalCommand::SetOccurrenceTransform {
            id: b,
            transform: Transform::from_translation(12., 0., 0.).unwrap(),
        },
    );
    assert!(session.rule_program().is_none());
    let repaired = check(&session.snapshot(), false);
    assert_eq!(repaired["state"], "passed", "{repaired:#}");
    assert_eq!(repaired["group_connectivity"]["checked_joint_count"], 1);
    session.undo().unwrap();
    assert_eq!(check(&session.snapshot(), false)["issue_count"], 1);
    session.redo().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("physical.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let mut session = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert!(session.rule_program().is_none());
    assert_eq!(session.snapshot().contact_joints().len(), 1);
    assert_eq!(check(&session.snapshot(), false)["state"], "passed");
    edit(
        &mut session,
        CanonicalCommand::SetOccurrenceTransform {
            id: b,
            transform: Transform::from_translation(30., 0., 0.).unwrap(),
        },
    );
    assert_eq!(
        check(&session.snapshot(), false)["issues"][0]["distance_mm"],
        20.
    );
    edit(&mut session, CanonicalCommand::DeleteOccurrence { id: b });
    assert!(session.snapshot().contact_joints().is_empty());
    assert_eq!(check(&session.snapshot(), false)["state"], "passed");
    session.undo().unwrap();
    assert_eq!(check(&session.snapshot(), false)["issue_count"], 1);
}

#[test]
fn nested_rotated_joints_use_exact_distance_and_program_removal_clears_declarations() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let source = "a=extrude('a',profile=[(0,0),(100,0),(0,100)],distance=20)\nb=box('b',(20,20,20),at=(70,70,0))\njoint(a,b,kind='fixed',name='physical')\nc=component('c',[a,b])\ninstance('copy',c,at=(300,0,0),x=(0,1,0))";
    apply(&mut session, source);
    assert_eq!(session.snapshot().contact_joints().len(), 2);
    let report = check(&session.snapshot(), false);
    assert_eq!(report["complete"], true, "{report:#}");
    let joints = report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "joint_without_contact")
        .collect::<Vec<_>>();
    assert_eq!(joints.len(), 2, "{report:#}");
    for issue in &joints {
        assert!((issue["distance_mm"].as_f64().unwrap() - 40. / 2f64.sqrt()).abs() < 1e-6);
        assert_ne!(issue["instance_paths"][0], issue["instance_paths"][1]);
        assert_eq!(
            issue["instance_paths"][0]["root"],
            issue["instance_paths"][1]["root"]
        );
    }
    assert!(
        joints
            .iter()
            .any(|i| i["names"] == serde_json::json!(["copy/a", "copy/b"]))
    );
    apply(
        &mut session,
        &source.replace("joint(a,b,kind='fixed',name='physical')\n", ""),
    );
    assert!(session.snapshot().contact_joints().is_empty());
    assert!(
        check(&session.snapshot(), false)["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["kind"] != "joint_without_contact")
    );
}

#[test]
fn kinematic_joints_do_not_require_contact_and_missing_measurement_is_not_passed() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let snapshot = session.snapshot();
    for report in [
        check(&snapshot, true),
        assistant_validation_context(
            &snapshot,
            &ExactResultRegistry::default(),
            &AssistantValidationSelection::only(&["group_connectivity"]),
        ),
    ] {
        assert_eq!(report["complete"], false, "{report:#}");
        assert_eq!(report["state"], "not_evaluated");
        assert_eq!(report["issue_count"], 0);
    }
    apply(
        &mut session,
        "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(300,0,0))\njoint(a,b,kind='motion',motion=slide((1,0,0),0,100),position=20)",
    );
    assert!(session.snapshot().contact_joints().is_empty());
    let report = check(&session.snapshot(), false);
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["group_connectivity"]["checked_joint_count"], 0);
}
