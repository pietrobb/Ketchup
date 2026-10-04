use ketchup_application::{
    DocumentSession, ExactMotionPair, SessionSettings, exact_motion_pair_with_worker,
    rule_program_part_name,
};
use ketchup_model::{
    assembly_joint::{AssemblyJointAxis, AssemblyJointKind},
    document::{RuleProgramSource, Snapshot, Transform},
    exact_product::ExactSnapshotPreparation,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn pair(
    source: &str,
    moving: &str,
    obstacle: &str,
    motion: AssemblyJointKind,
    from: f64,
    to: f64,
) -> (DocumentSession, ExactMotionPair) {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "motion.star".into(),
                source: source.into(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .unwrap();
    let snapshot = &applied.snapshot;
    let preparation = ExactSnapshotPreparation::new(snapshot).unwrap();
    let body = |name: &str| {
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|p| rule_program_part_name(snapshot, &p.instance_path).as_deref() == Some(name))
            .unwrap();
        let terminals = preparation
            .terminal_features(occurrence.definition_id)
            .unwrap();
        assert_eq!(terminals.len(), 1);
        let (_, producer) = terminals.into_iter().next().unwrap();
        (
            preparation
                .graph(occurrence.definition_id, producer)
                .unwrap(),
            occurrence.transform,
        )
    };
    let (moving, zero_transform) = body(moving);
    let (obstacle, obstacle_transform) = body(obstacle);
    (
        session,
        ExactMotionPair {
            moving,
            obstacle,
            zero_transform,
            obstacle_transform,
            motion,
            from,
            to,
            tolerance_mm: 0.001,
            allow_contact: false,
        },
    )
}

fn slide() -> AssemblyJointKind {
    AssemblyJointKind::Prismatic {
        axis: AssemblyJointAxis::new([1., 0., 0.], [0.; 3]),
        limits: None,
        position_mm: 0.,
    }
}
fn rotate() -> AssemblyJointKind {
    AssemblyJointKind::Revolute {
        axis: AssemblyJointAxis::new([0., 0., 1.], [0.; 3]),
        limits: None,
        position_degrees: 0.,
    }
}
fn check(pair: &ExactMotionPair) -> Value {
    exact_motion_pair_with_worker(
        pair,
        &BTreeMap::new(),
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    )
}
fn scene(snapshot: &Snapshot) -> Vec<ketchup_model::document::SceneOccurrence> {
    snapshot.scene_query()
}

#[test]
fn unsupported_envelope_stays_incomplete_despite_clear_native_samples() {
    let _turn = crate::integration_support::file_turn();
    let (_, pair) = pair(
        "extrude('moving',distance=2,profile=round_corners([[0,0],[20,0],[20,20],[0,20]],3))\nbox('obstacle',(2,2,2),at=(100,0,0))",
        "moving",
        "obstacle",
        slide(),
        0.,
        10.,
    );
    let report = check(&pair);
    assert_eq!(report["state"], "incomplete", "{report}");
    assert_eq!(report["checked_pose_count"], 3);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "uncertified_motion_envelope"
    );
}

#[test]
fn a_driven_parent_never_silently_uses_the_untransformed_child_axis() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session.apply_rule_program(RuleProgramSource {
        file_name: "parent-motion.star".into(), overrides: BTreeMap::new(),
        source: "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2))\njoint(a,b,kind='motion',name='inner',motion=slide((1,0,0),0,20))\nc=component('base',[a,b])\ni=instance('copy',c,at=(100,0,0))\nr=box('root',(2,2,2),at=(-200,0,0))\njoint(r,i,kind='motion',name='outer',motion=slide((0,1,0),0,20),position=5)".into(),
    }, false).unwrap();
    let request = ketchup_application::ProgramMotionCheck {
        name: "copy/inner".into(),
        from: 0.,
        to: 20.,
    };
    let report = ketchup_application::verify_rule_program_motion(
        &applied.snapshot,
        &applied.model,
        &request,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(report["state"], "incomplete", "{report}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "driven_parent_frame_not_supported"
    );
    assert!(report["pairs"].as_array().unwrap().is_empty());
}

#[test]
fn sliding_collision_between_clear_endpoints_preserves_document() {
    let _turn = crate::integration_support::file_turn();
    let (session, mut pair) = pair(
        "box('moving',(2,2,2))\nbox('obstacle',(2,2,2),at=(10,0,0))",
        "moving",
        "obstacle",
        slide(),
        0.,
        20.,
    );
    let before = scene(&session.snapshot());
    for position in [0., 20.] {
        pair.from = position;
        pair.to = position;
        assert_eq!(check(&pair)["state"], "passed");
    }
    pair.from = 0.;
    pair.to = 20.;
    let report = check(&pair);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["issues"][0]["position"], 10.);
    assert!(report["issues"][0]["common_volume_mm3"].as_f64().unwrap() > 7.99);
    assert_eq!(scene(&session.snapshot()), before);
}

#[test]
fn exact_clearance_proves_motion_inside_overlapping_envelopes() {
    let _turn = crate::integration_support::file_turn();
    let (_, pair) = pair(
        "a=box('obstacle',(20,20,10))\nb=box('cut',(16,16,12),at=(2,2,-1))\nsubtract(a,b)\nbox('moving',(2,2,2),at=(4,4,4))",
        "moving",
        "obstacle",
        slide(),
        0.,
        8.,
    );
    let report = check(&pair);
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["complete"], true);
    assert!(
        report["checked_pose_count"].as_u64().unwrap() > 3,
        "requires interval subdivision: {report}"
    );
    assert!(
        report["verified_intervals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["clearance_lower_bound_mm"].as_f64().unwrap() > 0.)
    );
}

#[test]
fn full_rotation_detects_off_midpoint_collision_and_clear_rotation() {
    let _turn = crate::integration_support::file_turn();
    let (_, pair) = pair(
        "box('moving',(2,2,2),at=(10,0,0))\nbox('obstacle',(2,2,2),at=(-2,10,0))",
        "moving",
        "obstacle",
        rotate(),
        0.,
        360.,
    );
    let report = check(&pair);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["issues"][0]["position"], 90.);
    let mut clear = pair;
    clear.obstacle_transform = Transform::from_translation(100., 100., 0.).unwrap();
    assert_eq!(check(&clear)["state"], "passed");
}

#[test]
fn assembly_contact_requires_a_whole_translation_envelope_not_clear_samples() {
    let _turn = crate::integration_support::file_turn();
    let (_, mut pair) = pair(
        "box('moving',(2,2,2),at=(2,0,0))\nbox('obstacle',(2,2,2))",
        "moving",
        "obstacle",
        slide(),
        10.,
        0.,
    );
    assert_eq!(check(&pair)["state"], "incomplete");
    pair.allow_contact = true;
    let contact = check(&pair);
    assert_eq!(contact["state"], "passed", "{contact}");
    assert!(
        contact["verified_intervals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["method"] == "translation_swept_hull_with_native_contact_tolerance")
    );
    // Endpoints and midpoint do not penetrate, but the intervening path crosses the obstacle.
    pair.to = -10.;
    let blocked = check(&pair);
    assert_eq!(blocked["state"], "failed", "{blocked}");
    assert!(blocked["issues"][0]["common_volume_mm3"].as_f64().unwrap() > 0.);
    pair.to = 0.;
    pair.from = -10.;
    pair.motion = rotate();
    assert_eq!(
        check(&pair)["state"],
        "incomplete",
        "Rotating endpoint hulls do not enclose an arc"
    );
}

#[test]
fn tangential_motion_exhausts_budget_without_claiming_pass() {
    let _turn = crate::integration_support::file_turn();
    let (_, pair) = pair(
        "box('moving',(2,2,2))\nbox('obstacle',(30,2,2),at=(0,2,0))",
        "moving",
        "obstacle",
        slide(),
        0.,
        20.,
    );
    let report = check(&pair);
    assert_eq!(report["state"], "incomplete", "{report}");
    assert_eq!(report["complete"], false);
    assert_eq!(report["checked_pose_count"], 256);
    assert!(
        !report["unresolved_intervals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancellation_deadline_invalid_input_and_missing_worker_never_pass() {
    let _turn = crate::integration_support::file_turn();
    let (_, mut pair) = pair(
        "box('moving',(2,2,2))\nbox('obstacle',(2,2,2),at=(100,0,0))",
        "moving",
        "obstacle",
        slide(),
        0.,
        20.,
    );
    for (timeout, cancelled) in [(Duration::ZERO, false), (Duration::from_secs(30), true)] {
        let report = exact_motion_pair_with_worker(
            &pair,
            &BTreeMap::new(),
            None,
            timeout,
            Arc::new(AtomicBool::new(cancelled)),
        );
        assert_eq!(report["state"], "incomplete", "{report}");
        assert_eq!(report["checked_pose_count"], 0);
    }
    let report = exact_motion_pair_with_worker(
        &pair,
        &BTreeMap::new(),
        Some("missing-motion-worker.exe".into()),
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(report["state"], "incomplete", "{report}");
    assert!(report["not_evaluated"][0]["cause"].as_str().is_some());
    pair.from = f64::NAN;
    assert_eq!(check(&pair)["state"], "incomplete");
}

#[test]
fn declared_motion_reports_transformed_nested_obstacle_and_removes_current_pose() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session.apply_rule_program(RuleProgramSource {
        file_name: "nested-motion.star".into(), overrides: BTreeMap::new(),
        source: "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2))\nc=component('base',[a,b])\ni=instance('copy',c,at=(100,0,0),x=(0,1,0))\njoint(a,i,kind='motion',motion=slide((1,0,0),0,20),position=3)\nbox('obstacle',(2,2,2),at=(108,0,0))".into(),
    }, false).unwrap();
    let motion = &applied.model.motions[0];
    let request = ketchup_application::ProgramMotionCheck {
        name: motion.name.clone(),
        from: 0.,
        to: 20.,
    };
    let before = session.snapshot().scene_query();
    let report = ketchup_application::verify_rule_program_motion(
        &applied.snapshot,
        &applied.model,
        &request,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(report["state"], "failed", "{report}");
    let hit = report["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["state"] == "failed")
        .unwrap();
    assert_eq!(hit["moving"]["name"], "copy/b");
    assert_eq!(hit["obstacle"]["name"], "obstacle");
    assert!(
        !hit["moving"]["instance_path"]["steps"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(hit["issues"][0]["position"], 10.);
    assert_eq!(session.snapshot().scene_query(), before);
    let invalid = ketchup_application::ProgramMotionCheck { to: 21., ..request };
    let report = ketchup_application::verify_rule_program_motion(
        &applied.snapshot,
        &applied.model,
        &invalid,
        None,
        Duration::from_secs(30),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(report["state"], "incomplete", "{report}");
    assert!(report["pairs"].as_array().unwrap().is_empty());
}
