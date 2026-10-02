//! Outcome regressions for CR-09 and CR-12.
use ketchup_application::{
    DocumentSession, SessionSettings, rewrite_rule_program_push_pull, verify_rule_program_exact,
};
use ketchup_model::{document::RuleProgramSource, persistence::ContainerData};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn source(expression: &str, bounds: &str) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "review.star".to_owned(),
        source: format!(
            "H = param(\"H\", 10.0{bounds})\np = extrude(\"part\", profile=[(0,0),(20,0),(0,20)], distance={expression})\nother = box(\"other\", (20,20,H), at=(100,0,0))\n"
        ),
        overrides: Default::default(),
    }
}

fn check_height(source: &RuleProgramSource, expected: f64) -> ketchup_program::ProgramModel {
    let result = ketchup_program::evaluate(&source.file_name, &source.source, &source.overrides)
        .expect("rewritten program evaluates");
    let part = result.model.part("part").expect("part");
    let face = part.face_frame("end").expect("end face");
    assert!(
        (face.origin_mm[2] - expected).abs() < 1.0e-7,
        "face at {:?}, expected {expected}",
        face.origin_mm
    );
    result.model
}

#[test]
fn linear_driver_reaches_requested_face_without_an_extra_operation() {
    let original = source("H*2", "");
    let rewritten =
        rewrite_rule_program_push_pull(&original, "part", "end", 21.0).expect("rewrite");
    let model = check_height(&rewritten, 41.0);
    assert_eq!(model.part("part").expect("part").face_offsets().count(), 0);
    assert!((rewritten.overrides["H"] - 20.5).abs() < 1.0e-7);
}

#[test]
fn quadratic_driver_falls_back_locally_to_exactly_121_mm() {
    let original = source("H*H", "");
    let rewritten =
        rewrite_rule_program_push_pull(&original, "part", "end", 21.0).expect("rewrite");
    let model = check_height(&rewritten, 121.0);
    assert_eq!(rewritten.overrides, original.overrides);
    assert_eq!(
        model.part("other").expect("other").local_bounds().1[2],
        10.0
    );
    assert_eq!(model.part("part").expect("part").face_offsets().count(), 1);
    let again =
        rewrite_rule_program_push_pull(&rewritten, "part", "end", -5.0).expect("second rewrite");
    check_height(&again, 116.0);
}

#[test]
fn clamp_piecewise_and_parameter_limits_use_a_safe_local_fallback() {
    for (expression, bounds, expected) in [
        ("min(H,11)", "", 31.0),
        ("max(H,12)", "", 33.0),
        ("H if H < 11 else H*2", "", 31.0),
        ("H", ", min=9, max=11", 31.0),
    ] {
        let original = source(expression, bounds);
        let rewritten =
            rewrite_rule_program_push_pull(&original, "part", "end", 21.0).expect("rewrite");
        let model = check_height(&rewritten, expected);
        assert_eq!(rewritten.overrides, original.overrides, "{expression}");
        assert_eq!(
            model.part("other").expect("other").local_bounds().1[2],
            10.0
        );
    }
}

#[test]
fn revolve_lower_wall_collision_is_confirmed_by_exact_solids_after_placement() {
    let _turn = crate::integration_support::file_turn();
    for placement in [
        "",
        "rotate(p, axis=(1,2,3), angle=37, pivot=(0,0,0))\nrotate(b, axis=(1,2,3), angle=37, pivot=(0,0,0))\nmove(p, by=(100,200,30))\nmove(b, by=(100,200,30))\n",
    ] {
        let mut session = DocumentSession::new(SessionSettings::default());
        let applied = session.apply_rule_program(RuleProgramSource {
            file_name: "review.star".to_owned(),
            source: format!("p = revolve(\"tube\", profile=[(0,10),(100,10),(100,20),(0,20)], axis=[(0,0),(1,0)])\nb = box(\"inside_wall\", (5,2,2), at=(40,-19,-1))\n{placement}"),
            overrides: Default::default(),
        }, false).expect("apply");
        let mut report = applied.report;
        let summary = verify_rule_program_exact(
            &applied.snapshot,
            &applied.model,
            &mut report,
            &ContainerData::default(),
            None,
            Duration::from_secs(120),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("exact candidates retained");
        assert_eq!(summary["state"], "verified", "{summary}");
        assert_eq!(summary["collisions"], 1, "{summary}");
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == "collision" && issue.parts.iter().any(|p| p == "tube")),
            "{:?}",
            report.issues
        );
    }
}
