//! The validator panel's whole-model check on the 750-part house: collision
//! and gravity check every visible part instead of stopping at a small-scene
//! limit (code review 2026-10-07, GE-2).
use ketchup_application::validation::assistant_validation_context_with_worker;
use ketchup_application::{AssistantValidationSelection, DocumentSession, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::persistence::ContainerData;
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

/// About 6 s in release on 12 cores; the panel gives the check 30 s.
const PANEL_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
#[ignore = "release-only: exact geometry of the whole house; run with --ignored"]
fn the_panel_checks_every_visible_part_of_the_house_for_collision_and_gravity() {
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
    let visible = applied
        .snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .count();
    assert!(visible > 512, "{visible}");
    let started = Instant::now();
    let report = assistant_validation_context_with_worker(
        &applied.snapshot,
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["collision", "gravity_support"]),
        &ContainerData::default(),
        None,
        PANEL_TIMEOUT,
    );
    assert!(started.elapsed() < PANEL_TIMEOUT, "{:?}", started.elapsed());
    // Concept walls and their construction are both shown in a bare session,
    // so the check finds their overlaps; what matters is that it ran.
    let collision = &report["collision"];
    assert_eq!(
        collision["complete"], true,
        "{}",
        collision["not_evaluated"]
    );
    assert_eq!(collision["scope"]["mode"], "whole_visible_model");
    assert_eq!(collision["checked_occurrence_count"], visible);
    let gravity = &report["gravity_support"];
    assert_ne!(
        gravity["state"], "not_evaluated",
        "{}",
        gravity["assumptions"]
    );
    assert_eq!(gravity["checked_occurrence_count"], visible);
}
