//! Outcome regressions for CR-09 and CR-12.
use ketchup_application::{
    DocumentSession, PartChange, SessionSettings, rewrite_rule_program_parts,
    verify_rule_program_exact,
};
use ketchup_model::{document::RuleProgramSource, persistence::ContainerData};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn source(expression: &str, bounds: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "review.star".to_owned(),
        source: format!(
            "H = param(\"H\", 10.0{bounds})\np = extrude(\"part\", profile=[(0,0),(20,0),(0,20)], distance={expression})\nother = box(\"other\", (20,20,H), at=(100,0,0))\n"
        ),
        overrides: Default::default(),
    }
}

fn push_pull(source: &RuleProgramSource, distance: f64) -> RuleProgramSource {
    let face = "end".to_owned();
    let change = PartChange::PushPull { face, distance };
    rewrite_rule_program_parts(source, &[("part".to_owned(), change)])
        .expect("rewrite")
        .expect("a program part")
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
    let shared = source("H*2", "");
    let mut original = shared.clone();
    original.source = original.source.replace("(20,20,H)", "(20,20,10)");
    let rewritten = push_pull(&original, 21.0);
    let model = check_height(&rewritten, 41.0);
    assert_eq!(model.part("part").expect("part").face_offsets().count(), 0);
    assert!((rewritten.overrides["H"] - 20.5).abs() < 1.0e-7);
    // Review 2026-10-19 WB-2: H makes "other" too, so the part alone is
    // pushed and "other" stays 10 high.
    let rewritten = push_pull(&shared, 21.0);
    let model = check_height(&rewritten, 41.0);
    assert!(rewritten.overrides.is_empty());
    assert_eq!(model.part("part").expect("part").face_offsets().count(), 1);
    assert_eq!(
        model.part("other").expect("other").local_bounds().1[2],
        10.0
    );
}

#[test]
fn quadratic_driver_falls_back_locally_to_exactly_121_mm() {
    let original = source("H*H", "");
    let rewritten = push_pull(&original, 21.0);
    let model = check_height(&rewritten, 121.0);
    assert_eq!(rewritten.overrides, original.overrides);
    assert_eq!(
        model.part("other").expect("other").local_bounds().1[2],
        10.0
    );
    assert_eq!(model.part("part").expect("part").face_offsets().count(), 1);
    let again = push_pull(&rewritten, -5.0);
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
        let rewritten = push_pull(&original, 21.0);
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
        let applied = session.apply_rule_program(RuleProgramSource { variants: Default::default(),
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
            ketchup_application::program_isolation::CheckDeadline::after(Duration::from_secs(120)),
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
