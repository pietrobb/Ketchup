//! The exact collision check covers every part of the 750-part house in time.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_all};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

/// In release the check takes about 20 s on 12 cores; it once took over 3 minutes.
const BUDGET: Duration = Duration::from_secs(if cfg!(debug_assertions) { 600 } else { 90 });

#[test]
fn the_whole_house_is_checked_for_collisions_exactly_and_in_time() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "tiny-house.star".to_owned(),
                source: HOUSE.to_owned(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    assert!(
        applied.model.parts.len() >= 700,
        "{}",
        applied.model.parts.len()
    );
    let mut report = applied.report;
    let started = Instant::now();
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
    let elapsed = started.elapsed();
    assert_eq!(summary["state"], "verified", "{summary:#}");
    assert_eq!(summary["collisions"], 0, "{summary:#}");
    assert_eq!(summary["unresolved"], 0, "{summary:#}");
    assert_eq!(
        summary["not_evaluated"],
        serde_json::json!([]),
        "{summary:#}"
    );
    assert!(elapsed < BUDGET, "{elapsed:?}");
}
