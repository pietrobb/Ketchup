//! The demo house (examples/programs/house-project.star, ~1850 timber parts) has no overlap
//! in the exact collision check: every joint is cut into the receiving member.
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
