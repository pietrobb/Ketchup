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
    let mut load_state = None;
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
        let joints_failed = report
            .joints
            .iter()
            .filter(|joint| joint.status == "fail")
            .count();
        let unassigned = report.loads.unassigned.len();
        let state = if members.fail > 0 || joints_failed > 0 {
            "failed"
        } else if members.not_verified > 0 || joints_open > 0 || unassigned > 0 {
            "incomplete"
        } else {
            "passed"
        };
        load_state = Some(state);
        json!({
            "state": state,
            "method": "ec5_member_check_computed_not_authorized_design",
            "scope": "load_path_members_and_bearing_joints",
            "members": members.members, "fail": members.fail, "not_verified": members.not_verified,
            "bearing_joints_not_verified": joints_open,
            "bearing_joints_failed": joints_failed,
            "unassigned_loads": unassigned
        })
    };
    // A requested load check is part of the verdict: an overloaded member fails it.
    let state = if report.errors > 0 || load_state == Some("failed") {
        "failed"
    } else if !exact_complete
        || unverified
        || distance_incomplete
        || load_state == Some("incomplete")
    {
        "incomplete"
    } else {
        "passed"
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

    #[test]
    fn an_overloaded_member_fails_the_whole_summary() {
        let source = "load_path(only = ['frame'], carriers = ['deck'])
area_load('live', kind = 'imposed', kn_m2 = 2.0, on = ['deck'], source = 'test')
timber_design({'C24': 'C24'})
box('post a', (100, 60, 2000), material = 'C24', grounded = True)
box('post b', (100, 60, 2000), at = (3900, 0, 0), material = 'C24', grounded = True)
box('beam', (4000, 60, 100), at = (0, 0, 2000), material = 'C24', tags = ['frame'])
box('deck', (4000, 600, 20), at = (0, -270, 2100), material = 'OSB', tags = ['deck'])";
        let (evaluated, report) =
            ketchup_program::run("weak.star", source, &BTreeMap::new()).unwrap();
        assert_eq!(report.errors, 0, "{:?}", report.issues);
        let exact = json!({"state":"verified", "distance_measurements_state":"complete"});
        let summary = summary(&report, &evaluated.model, Some(&exact));
        assert_eq!(summary["load_capacity"]["state"], "failed");
        assert_eq!(summary["state"], "failed");
    }

    #[test]
    fn a_load_that_reaches_no_member_leaves_the_load_check_incomplete() {
        // The cover lies on laths that are neither members nor carriers: its
        // load reaches nothing, though the beam itself passes.
        let source = "load_path(only = ['frame'])
self_weight(['frame'])
area_load('live', kind = 'imposed', kn_m2 = 2.0, on = ['cover'], source = 'test')
timber_design({'C24': 'C24'})
box('post a', (100, 100, 2000), material = 'C24', grounded = True)
box('post b', (100, 100, 2000), at = (3900, 0, 0), material = 'C24', grounded = True)
box('beam', (4000, 100, 200), at = (0, 0, 2000), material = 'C24', tags = ['frame'])
box('lath 1', (50, 600, 30), at = (500, -250, 2200), material = 'C24')
box('lath 2', (50, 600, 30), at = (3400, -250, 2200), material = 'C24')
box('cover', (4000, 600, 10), at = (0, -250, 2230), material = 'OSB', tags = ['cover'])";
        let (evaluated, report) =
            ketchup_program::run("lost.star", source, &BTreeMap::new()).unwrap();
        assert_eq!(report.errors, 0, "{:?}", report.issues);
        assert_eq!(
            report.loads.unassigned.keys().collect::<Vec<_>>(),
            ["cover"]
        );
        let exact = json!({"state":"verified", "distance_measurements_state":"complete"});
        let summary = summary(&report, &evaluated.model, Some(&exact));
        assert_eq!(summary["load_capacity"]["not_verified"], 0);
        assert_eq!(summary["load_capacity"]["fail"], 0);
        assert_eq!(summary["load_capacity"]["unassigned_loads"], 1);
        assert_eq!(summary["load_capacity"]["state"], "incomplete");
        assert_eq!(summary["state"], "incomplete");
    }

    #[test]
    fn an_overloaded_hanger_fails_the_load_check_and_a_strong_one_passes_it() {
        // A joist on a hanger at the header and a post at its far end.
        let source = |rating_n: u32| {
            format!(
                "load_path(only = ['frame'], carriers = ['deck'])
area_load('live', kind = 'imposed', kn_m2 = 2.0, on = ['deck'], source = 'test')
timber_design({{'C24': 'C24'}})
header = box('header', (120, 2000, 240), material = 'C24', grounded = True)
joist = box('joist', (3000, 60, 200), at = (120, 500, 40), material = 'C24', tags = ['frame'])
box('post', (100, 60, 240), at = (3020, 500, -200), material = 'C24', grounded = True)
box('deck', (3000, 600, 20), at = (120, 230, 240), material = 'OSB', tags = ['deck'])
joint(joist, header, kind = 'hanger', bearing = True,
      rating = connector_rating(load_n = {rating_n}, basis = 'design', source = 'ETA-00/0000'))"
            )
        };
        let exact = json!({"state":"verified", "distance_measurements_state":"complete"});
        let state = |rating_n: u32| {
            let (evaluated, report) =
                ketchup_program::run("hanger.star", &source(rating_n), &BTreeMap::new()).unwrap();
            assert_eq!(report.errors, 0, "{:?}", report.issues);
            let summary = summary(&report, &evaluated.model, Some(&exact));
            (
                summary["load_capacity"]["state"].clone(),
                summary["load_capacity"]["bearing_joints_failed"].clone(),
            )
        };
        assert_eq!(state(500), (json!("failed"), json!(1)));
        assert_eq!(state(50_000), (json!("passed"), json!(0)));
    }
}
