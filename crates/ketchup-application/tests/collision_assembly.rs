use ketchup_application::{
    DocumentSession, SaveOptions, SessionSettings, verify_rule_program_assembly,
};
use ketchup_model::document::RuleProgramSource;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

const GEOMETRY: &str = "a=box('base',(2,2,2),at=(-100,0,0))\nb=box('insert',(2,2,2),at=(20,0,0))\nc=box('stop',(2,2,2),at=(10,0,0))\njoint(a,b,kind='motion',name='insert',motion=slide((1,0,0),-20,0))\njoint(a,c,kind='motion',name='stop',motion=slide((0,1,0),0,20))\n";
const INSERT: &str = "assembly_step('insert',start=-20,end=0)\n";
const STOP: &str = "assembly_step('stop',start=20,end=0)\n";

fn apply(session: &mut DocumentSession, source: &str) {
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "assembly.star".into(),
                source: source.into(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .unwrap();
}
fn check(session: &DocumentSession) -> serde_json::Value {
    let source = session.rule_program().unwrap();
    let evaluated =
        ketchup_program::evaluate(&source.file_name, &source.source, &source.overrides).unwrap();
    verify_rule_program_assembly(
        &session.snapshot(),
        &evaluated.model,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    )
}

#[test]
fn final_fit_is_not_a_pass_for_blocked_insertion_order() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    apply(&mut session, &format!("{GEOMETRY}{INSERT}{STOP}"));
    let before = session.snapshot().scene_query();
    let clear = check(&session);
    assert_eq!(clear["state"], "passed", "{clear}");
    assert_eq!(clear["steps_checked"], 2);
    assert_eq!(
        clear["steps"][0]["absent_parts"],
        serde_json::json!(["stop"])
    );
    assert_eq!(session.snapshot().scene_query(), before);
    apply(&mut session, &format!("{GEOMETRY}{STOP}{INSERT}"));
    assert_eq!(
        session.snapshot().scene_query(),
        before,
        "Only order changed, not final fit"
    );
    let blocked = check(&session);
    assert_eq!(blocked["state"], "failed", "{blocked}");
    let conflict = &blocked["steps"][1]["pairs"][0];
    assert_eq!(conflict["moving"]["name"], "insert");
    assert_eq!(conflict["obstacle"]["name"], "stop");
    assert!(conflict["issues"][0]["common_volume_mm3"].as_f64().unwrap() > 7.99);
    assert_eq!(session.snapshot().scene_query(), before);
    session.undo().unwrap();
    assert_eq!(check(&session)["state"], "passed");
    session.redo().unwrap();
    assert_eq!(check(&session)["state"], "failed");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("assembly.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(check(&reopened)["state"], "failed");
    assert_eq!(reopened.snapshot().scene_query(), before);
}

#[test]
fn missing_invalid_and_duplicate_paths_are_not_success_or_edit_blockers() {
    let mut session = DocumentSession::default();
    for suffix in [
        "",
        "assembly_step('absent',start=1,end=0)",
        "assembly_step('insert',start=-20,end=-1)",
        "assembly_step('insert',start=-21,end=0)",
        "assembly_step('insert',start=-20,end=0)\nassembly_step('insert',start=-10,end=0)",
    ] {
        apply(&mut session, &format!("{GEOMETRY}{suffix}"));
        let report = check(&session);
        assert_eq!(report["state"], "incomplete", "{suffix}: {report}");
        assert_eq!(report["complete"], false);
        assert!(report["steps"].as_array().unwrap().is_empty());
        assert!(!report["not_evaluated"].as_array().unwrap().is_empty());
    }
}

#[test]
fn rigid_group_checks_every_member_against_preinstalled_parts() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let source = "a=box('base',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2),at=(20,0,0))\nc=box('c',(2,2,2),at=(20,10,0))\ng=group('moving',[b,c])\njoint(a,g,kind='motion',name='insert',motion=slide((1,0,0),-20,0))\nassembly_step('insert',start=-20,end=0)\n";
    apply(&mut session, source);
    assert_eq!(check(&session)["state"], "passed");
    apply(
        &mut session,
        &format!("{source}box('blocker',(2,2,2),at=(10,10,0))"),
    );
    let report = check(&session);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["steps"][0]["pairs"][0]["moving"]["name"], "c");
    assert_eq!(
        report["steps"][0]["pairs"][0]["obstacle"]["name"],
        "blocker"
    );
}

#[test]
fn unsupported_final_contact_does_not_hide_unproven_last_interval() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    apply(
        &mut session,
        "a=box('base',(2,2,2))\nb=extrude('insert',distance=2,profile=round_corners([[2,0],[4,0],[4,2],[2,2]],0.25))\njoint(a,b,kind='motion',name='insert',motion=slide((1,0,0),0,10))\nassembly_step('insert',start=10,end=0)",
    );
    let report = check(&session);
    assert_eq!(report["state"], "incomplete", "{report}");
    let pair = &report["steps"][0]["pairs"][0];
    assert!(pair["issues"].as_array().unwrap().is_empty());
    assert!(!pair["unresolved_intervals"].as_array().unwrap().is_empty());
}
