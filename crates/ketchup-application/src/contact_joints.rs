//! Contact checks for physical declarations; kinematic joints do not imply contact.
use crate::collision::ExactPairFacts;
use ketchup_model::document::{InstancePath, Snapshot};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn candidates<'a>(
    snapshot: &Snapshot,
    paths: impl Iterator<Item = &'a InstancePath>,
) -> BTreeSet<(usize, usize)> {
    let mut indices = BTreeMap::<&InstancePath, Vec<usize>>::new();
    for (index, path) in paths.enumerate() {
        indices.entry(path).or_default().push(index);
    }
    let mut pairs = BTreeSet::new();
    for joint in snapshot.contact_joints() {
        if let (Some(left), Some(right)) =
            (indices.get(&joint.parts[0]), indices.get(&joint.parts[1]))
        {
            for &a in left {
                for &b in right {
                    if a != b {
                        pairs.insert((a.min(b), a.max(b)));
                    }
                }
            }
        }
    }
    pairs
}

pub(crate) fn append_report(
    snapshot: &Snapshot,
    facts: &ExactPairFacts,
    coverage: &Value,
    report: &mut Value,
) {
    let complete =
        coverage["complete"] == true && coverage["method"] == "worker_brep_common_volume";
    let mut checked = 0;
    for joint in snapshot.contact_joints() {
        let [a, b] = &joint.parts;
        let fact = facts.get(&(a.min(b).clone(), a.max(b).clone()));
        let distance = fact.and_then(|fact| fact.distance_mm);
        if !complete || distance.is_none() {
            report["complete"] = json!(false);
            report["not_evaluated"].as_array_mut().expect("report array").push(json!({
                "joint":joint.name, "instance_paths":joint.parts, "reason":"joint_contact_unverified"}));
            continue;
        }
        checked += 1;
        let distance = distance.expect("checked distance");
        if distance <= joint.max_gap_mm + snapshot.tolerance().linear_mm()
            || fact.is_some_and(|pair| pair.penetrating())
        {
            continue;
        }
        let names = joint
            .parts
            .each_ref()
            .map(|path| crate::rule_program_part_name(snapshot, path));
        let origins = joint.parts.each_ref().map(|path| {
            snapshot
                .resolve_instance_path(path)
                .ok()
                .map(|part| part.world_transform.transform_point([0.0; 3]))
        });
        report["issues"].as_array_mut().expect("report array").push(json!({
            "validator":"group_connectivity", "kind":"joint_without_contact", "code":"joint.without_contact", "severity":"warning",
            "joint":joint.name, "names":names, "instance_paths":joint.parts,
            "occurrence_ids":joint.parts.each_ref().map(|path| path.root_occurrence().0),
            "origin_mm":origins, "distance_mm":distance, "max_gap_mm":joint.max_gap_mm,
            "rule":"A declared physical joint must connect its parts within the allowed gap.",
            "message":format!("Joint {:?}: parts are {:.3} mm apart; allowed gap {:.3} mm.", joint.name, distance, joint.max_gap_mm),
            "fix_hint":"Move the parts into contact, correct the joint's allowed gap, or remove the physical joint declaration."}));
    }
    report["checked_joint_count"] = json!(checked);
    report["issue_count"] = json!(report["issues"].as_array().expect("report array").len());
    report["state"] = json!(if report["issue_count"] != 0 {
        "failed"
    } else if report["complete"] == true {
        "passed"
    } else {
        "not_evaluated"
    });
}
