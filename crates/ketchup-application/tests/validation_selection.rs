use std::collections::BTreeSet;

use ketchup_application::{AssistantValidationSelection, DocumentSession};

#[test]
fn unchecked_public_selection_cannot_report_success_without_a_known_validator() {
    let session = DocumentSession::default();
    for requested in [
        BTreeSet::from(["bogus"]),
        BTreeSet::from(["bogus", "collision"]),
    ] {
        let selection = AssistantValidationSelection {
            mode: "only",
            requested,
            unknown: Vec::new(),
        };
        assert!(!selection.is_valid());
        let report = session.validators(&selection);
        assert_eq!(report["state"], "not_evaluated");
        assert_eq!(report["complete"], false);
        assert_eq!(report["executed"], serde_json::json!([]));
        assert_eq!(
            report["selection_error"],
            "unknown_or_empty_validator_selection"
        );
    }
    assert_eq!(session.visible_undo_steps(), 0);
}

#[test]
fn canonical_catalog_selections_remain_valid() {
    assert!(AssistantValidationSelection::all("all").is_valid());
    assert!(AssistantValidationSelection::only(&["gravity_support"]).is_valid());
    assert!(!AssistantValidationSelection::only(&[]).is_valid());
    assert!(!AssistantValidationSelection::only(&["bogus"]).is_valid());
}
