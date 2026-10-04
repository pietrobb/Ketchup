use ketchup_application::{
    DocumentSession, SaveOptions, SessionSettings, verify_rule_program_tool_access,
};
use ketchup_model::document::RuleProgramSource;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

const SOURCE: &str = "box('work',(20,20,2),at=(-10,-10,-4))\nt=box('holder',(4,4,8),at=(-2,-2,0),tool=True)\ntool_access('approach',envelope=t,motion=slide((0,0,1),0,20),start=20)\n";
fn apply(session: &mut DocumentSession, text: &str) {
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "access.star".into(),
                source: text.into(),
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
    verify_rule_program_tool_access(
        &session.snapshot(),
        &evaluated.model,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    )
}

#[test]
fn auxiliary_tool_checks_volume_not_axis_without_adding_parts() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    apply(&mut session, SOURCE);
    let before = session.snapshot().scene_query();
    let clear = check(&session);
    assert_eq!(clear["state"], "passed", "{clear}");
    assert_eq!(session.snapshot().scene_query(), before);
    assert_eq!(before.len(), 1, "auxiliary holder is not physical");
    assert!(
        clear["checks"][0]["pairs"][0]["verified_intervals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["clearance_lower_bound_mm"].as_f64().unwrap() > 0.0)
    );
    // The z axis x=y=0 remains clear, but the housing hits the off-axis obstruction.
    apply(
        &mut session,
        &format!("{SOURCE}box('lip',(2,4,2),at=(1,-2,14))"),
    );
    let blocked = check(&session);
    assert_eq!(blocked["state"], "failed", "{blocked}");
    assert_eq!(blocked["checks"][0]["pairs"][0]["obstacle"]["name"], "lip");
    assert!(
        blocked["checks"][0]["pairs"][0]["issues"][0]["common_volume_mm3"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    session.undo().unwrap();
    assert_eq!(check(&session)["state"], "passed");
    session.redo().unwrap();
    assert_eq!(check(&session)["state"], "failed");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(check(&reopened)["state"], "failed");
    assert_eq!(reopened.snapshot().scene_query().len(), 2);
}

#[test]
fn narrow_aperture_and_rotated_approach_use_solid_clearance() {
    let _turn = crate::integration_support::file_turn();
    for rotate in [false, true] {
        for (width, expected) in [(2, "passed"), (6, "failed")] {
            let mut session = DocumentSession::default();
            let rotation = if rotate {
                "rotate(t,axis=(0,1,0),angle=90,pivot=(0,0,0))\nrotate(a,axis=(0,1,0),angle=90,pivot=(0,0,0))\nrotate(b,axis=(0,1,0),angle=90,pivot=(0,0,0))\n"
            } else {
                ""
            };
            let axis = if rotate { "(1,0,0)" } else { "(0,0,1)" };
            apply(
                &mut session,
                &format!(
                    "a=box('left',(10,20,2),at=(-12,-10,10))\nb=box('right',(10,20,2),at=(2,-10,10))\nt=box('tool',({width},2,4),at=(-{width}/2,-1,0),tool=True)\n{rotation}tool_access('passage',envelope=t,motion=slide({axis},0,20),start=20)"
                ),
            );
            let report = check(&session);
            assert_eq!(
                report["state"], expected,
                "rotated={rotate} width={width}: {report}"
            );
        }
    }
}

#[test]
fn undeclared_missing_and_invalid_approaches_remain_incomplete() {
    let mut session = DocumentSession::default();
    for declaration in [
        "",
        "tool_access('missing')",
        "tool_access('physical',envelope='work',motion=slide((0,0,1),0,20),start=20)",
        "tool_access('missing-path',envelope=t)",
        "tool_access('outside',envelope=t,motion=slide((0,0,1),0,20),start=30)",
        "tool_access('wrong-end',envelope=t,motion=slide((0,0,1),0,20),start=20,end=1)",
    ] {
        apply(
            &mut session,
            &format!(
                "box('work',(2,2,2))\nt=box('tool',(2,2,2),at=(0,0,10),tool=True)\n{declaration}"
            ),
        );
        let report = check(&session);
        assert_eq!(report["state"], "incomplete", "{declaration}: {report}");
        assert_eq!(report["complete"], false);
    }
}

#[test]
fn composite_envelope_and_rotating_approach_check_the_complete_volume() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let composite = "box('block',(2,2,2),at=(4,0,10))\nt=box('tool',(2,2,4),tool=True)\nw=box('wing',(6,2,2),at=(0,0,2),tool=True)\nunion(t,w)\ntool_access('composite',envelope=t,motion=slide((0,0,1),0,20),start=20)";
    apply(&mut session, composite);
    let report = check(&session);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(session.snapshot().scene_query().len(), 1);
    for (at, expected) in [("(-2,10,0)", "failed"), ("(100,100,100)", "passed")] {
        apply(
            &mut session,
            &format!(
                "box('obstacle',(2,2,2),at={at})\nt=box('tool',(2,2,2),at=(10,0,0),tool=True)\ntool_access('turn',envelope=t,motion=rotate_motion((0,0,1),0,360),start=360)"
            ),
        );
        let report = check(&session);
        assert_eq!(report["state"], expected, "{report}");
    }
}

#[test]
fn cancelled_access_is_not_success() {
    let session = DocumentSession::default();
    let evaluated = ketchup_program::evaluate("access.star", SOURCE, &BTreeMap::new()).unwrap();
    let report = verify_rule_program_tool_access(
        &session.snapshot(),
        &evaluated.model,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(true)),
    );
    assert_eq!(report["state"], "incomplete");
}
