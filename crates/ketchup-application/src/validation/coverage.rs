use ketchup_model::document::{OccurrenceId, SceneOccurrence};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn occurrence_names(occurrences: &[SceneOccurrence]) -> BTreeMap<OccurrenceId, String> {
    occurrences
        .iter()
        .take(super::MAX_VALIDATION_PARTICIPANTS)
        .filter(|item| item.instance_path.steps().is_empty())
        .map(|item| {
            (
                item.instance_path.root_occurrence(),
                item.occurrence_name.clone(),
            )
        })
        .collect()
}

pub(super) fn with_nested_scope(mut report: Value, occurrences: &[SceneOccurrence]) -> Value {
    let nested = occurrences.iter().filter(|item| !item.instance_path.steps().is_empty())
        .take(super::MAX_ASSISTANT_VALIDATION_ISSUES).map(|item| json!({
            "occurrence_id": item.instance_path.root_occurrence().0,
            "name": item.occurrence_name,
            "instance_path": item.instance_path,
            "reason": "nested_role_inputs_unsupported",
            "required_dimension": "ketchup.validator-role.v1",
            "fix_hint": "Role, material and numeric inputs currently address root parts only. Declare the checked bodies as root parts; shared member metadata cannot be substituted by the assembly root's inputs.",
        })).collect::<Vec<_>>();
    if nested.is_empty() {
        return report;
    }
    // Geometry/contact checks have instance-aware inputs; role checks do not yet.
    for validator in crate::validation_rules::ValidationRules::library()
        .required_roles
        .keys()
    {
        let detail = &mut report[validator];
        if detail["state"] == "skipped" || detail.is_null() {
            continue;
        }
        let mut missing = detail["not_evaluated"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        missing.extend(nested.iter().cloned());
        detail["not_evaluated"] = json!(missing);
        detail["complete"] = json!(false);
        if detail["state"] == "passed" {
            detail["state"] = json!("not_evaluated");
        }
        report["not_evaluated"]
            .as_array_mut()
            .expect("validation reports carry coverage")
            .push(json!({
                "validator": validator, "reason": "nested_role_inputs_unsupported",
            }));
        report["complete"] = json!(false);
        if report["state"] == "passed" {
            report["state"] = json!("not_evaluated");
        }
    }
    report
}

pub(super) fn role_check_state(
    validator: &str,
    rules: &crate::validation_rules::ValidationRules,
    evaluated_count: usize,
    issue_count: usize,
    coverage_complete: bool,
    not_evaluated: &mut Vec<Value>,
) -> (&'static str, bool) {
    if evaluated_count == 0 && issue_count == 0 && not_evaluated.is_empty() {
        not_evaluated.push(json!({
            "validator": validator,
            "reason": "applicable_role_not_found",
            "required_dimension": "ketchup.validator-role.v1",
            "required_roles": rules.required_roles.get(validator),
            "fix_hint": "Assign a matching role to the intended visible body, including its source-frame axis and association group where required. A role describes intent; it is not inferred from the body's name or shape.",
        }));
    }
    let complete = coverage_complete && not_evaluated.is_empty();
    // Known violations remain failures even when another body could not be checked.
    let state = if issue_count > 0 {
        "failed"
    } else if complete {
        "passed"
    } else {
        "not_evaluated"
    };
    (state, complete)
}
