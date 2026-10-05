//! Separate accepted edits, declared intent and exact evidence; absence is not a pass.
use ketchup_program::{ProgramModel, Report};
use serde_json::{Value, json};

pub(super) fn summary(report: &Report, model: &ProgramModel, exact: Option<&Value>) -> Value {
    let has = |kind: &str| report.issues.iter().any(|issue| issue.kind == kind);
    let exact_complete = exact.is_some_and(|value| value["state"] == "verified");
    let unverified = report
        .issues
        .iter()
        .any(|issue| issue.kind.ends_with("_unverified"));
    let distances = exact.map(|value| &value["distance_measurements_state"]);
    let distance_incomplete = distances.is_some_and(|value| value == "incomplete");
    let state = if report.errors > 0 {
        "failed"
    } else if !exact_complete || unverified || distance_incomplete {
        "incomplete"
    } else {
        "passed"
    };
    let required_exact_distances = !ketchup_program::exact::measured_pairs(model).is_empty();
    let dimensions = if model.expectations.is_empty() {
        "not_requested"
    } else if has("expectation_failed") {
        "failed"
    } else if has("expectation_unverified")
        || (required_exact_distances && (!exact_complete || distance_incomplete))
    {
        "incomplete"
    } else {
        "passed"
    };
    let collision_count = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .count();
    let holes = model
        .parts
        .iter()
        .map(|part| part.finished_holes().count())
        .sum::<usize>();
    let mut not_assessed = vec!["assembly_access", "machine_specific_export"];
    let load_capacity = if report.design.is_empty() {
        not_assessed.insert(0, "load_capacity");
        Value::Null
    } else {
        let members = report.design.summary();
        let joints_open = report
            .joints
            .iter()
            .filter(|joint| joint.status == "not_verified")
            .count();
        let state = if members.fail > 0 {
            "failed"
        } else if members.not_verified > 0 || joints_open > 0 {
            "incomplete"
        } else {
            "passed"
        };
        json!({
            "state": state,
            "method": "ec5_member_check_computed_not_authorized_design",
            "scope": "load_path_members_and_bearing_joints",
            "members": members.members, "fail": members.fail, "not_verified": members.not_verified,
            "bearing_joints_not_verified": joints_open
        })
    };
    json!({
        "load_capacity": load_capacity,
        "state": state,
        "program": {"state": "accepted", "method": "starlark_evaluation"},
        "declared_intent": {
            "state": dimensions, "checks": model.expectations.len(),
            "method": "program_constraints_with_available_exact_evidence",
            "scope": "declared_expectations_only"
        },
        "geometry": {
            "state": if exact_complete { "verified" } else if exact.is_some() { "incomplete" } else { "not_evaluated" },
            "scope": "requested_exact_pairs",
            "reason": if exact.is_none() { json!("exact_check_not_run") } else { Value::Null },
            "not_evaluated": exact.map(|value| &value["not_evaluated"])
        },
        "collisions": {
            "state": if collision_count > 0 { "failed" } else if exact_complete && !has("collision_unverified") { "passed" } else { "incomplete" },
            "count": collision_count,
            "scope": "program_parts_and_requested_exact_pairs"
        },
        "joints_and_holes": {
            "state": "checked", "method": "declared_geometry_rules",
            "joints": model.joints.len(), "holes": holes,
            "scope": "declared_joints_and_machining_only",
            "exact_manufacturing_verification": "not_evaluated"
        },
        "not_assessed": not_assessed
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn accepted_program_is_not_exact_validation_and_incomplete_distance_is_not_pass() {
        let (evaluated, report) = ketchup_program::run("check.star", "a=box('a',(10,10,10))\nhole(a,'z+',at=(5,5),diameter=2,depth=2)\nb=box('b',(10,10,10),at=(0,0,660))\nexpect_gap(a,b,650)", &BTreeMap::new()).unwrap();
        let absent = summary(&report, &evaluated.model, None);
        assert_eq!(absent["program"]["state"], "accepted");
        assert_eq!(absent["geometry"]["state"], "not_evaluated");
        assert_eq!(absent["state"], "incomplete");
        assert_eq!(absent["declared_intent"]["state"], "incomplete");
        let exact = json!({"state":"verified", "distance_measurements_state":"incomplete"});
        let partial = summary(&report, &evaluated.model, Some(&exact));
        assert_eq!(partial["state"], "incomplete");
        assert_eq!(partial["declared_intent"]["state"], "incomplete");
        assert_eq!(partial["joints_and_holes"]["holes"], 1);
        assert_eq!(
            partial["joints_and_holes"]["exact_manufacturing_verification"],
            "not_evaluated"
        );
    }
}
