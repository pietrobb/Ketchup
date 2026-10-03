//! Physical connectivity of canonical groups, independent of program ownership and grounding.
use crate::collision::ExactPairFacts;
use crate::validation::AssistantValidationSelection;
use ketchup_model::document::{GroupId, InstancePath, SceneOccurrence, Snapshot};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::exact_validation::GravitySupportContact;
use ketchup_model::tolerance::limits;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const ID: &str = "group_connectivity";
const MAX_MEMBERSHIPS: usize = 10_000;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    Root(GroupId),
    Local(InstancePath),
}

fn groups(snapshot: &Snapshot) -> Result<BTreeMap<GroupKey, Vec<SceneOccurrence>>, Value> {
    let scene = snapshot
        .scene_query_bounded(4096, limits::INSTANCE_PATH_STEPS, limits::REPORT_TEXT_BYTES)
        .map_err(|error| json!({"reason":"group_scene_limit", "detail":format!("{error:?}")}))?;
    let mut groups = BTreeMap::<GroupKey, Vec<SceneOccurrence>>::new();
    let mut count = 0;
    for leaf in scene.into_iter().filter(|leaf| {
        leaf.visible
            && snapshot
                .definition(leaf.definition_id)
                .is_some_and(|definition| definition.bodies().next().is_some())
    }) {
        let mut keys = Vec::new();
        let mut parent = snapshot
            .occurrence(leaf.instance_path.root_occurrence())
            .and_then(|root| root.parent());
        while let Some(id) = parent {
            keys.push(GroupKey::Root(id));
            parent = snapshot.group(id).and_then(|group| group.parent());
            if keys.len() > limits::INSTANCE_PATH_STEPS {
                return Err(json!({"reason":"group_depth_limit"}));
            }
        }
        let mut prefix = InstancePath::root(leaf.instance_path.root_occurrence());
        for step in leaf.instance_path.steps() {
            keys.push(GroupKey::Local(prefix.clone()));
            prefix = prefix.with_step(*step);
        }
        count += keys.len();
        if count > MAX_MEMBERSHIPS {
            return Err(json!({"reason":"group_membership_limit", "limit":MAX_MEMBERSHIPS}));
        }
        for key in keys {
            groups.entry(key).or_default().push(leaf.clone());
        }
    }
    Ok(groups)
}

fn root(parents: &[usize], mut index: usize) -> usize {
    while parents[index] != index {
        index = parents[index];
    }
    index
}

fn report(snapshot: &Snapshot, facts: &ExactPairFacts, coverage: &Value) -> Value {
    let mut issues = Vec::new();
    let mut unchecked = Vec::new();
    let groups = match groups(snapshot) {
        Ok(groups) => groups,
        Err(error) => {
            return json!({"state":"not_evaluated", "complete":false,
            "issue_count":0, "issues":[], "not_evaluated":[error]});
        }
    };
    // Missing pairs mean separation only after the complete native pass (including broad phase).
    let complete = groups.values().all(|members| members.len() < 2)
        || (coverage["complete"] == true && coverage["method"] == "worker_brep_common_volume");
    let mut checked = 0;
    for (key, members) in groups.iter().filter(|(_, members)| members.len() > 1) {
        let identity = match key {
            GroupKey::Root(id) => json!({"group_id":id.0,
                "name":snapshot.group(*id).map(|group| group.name())}),
            GroupKey::Local(path) => {
                let name = if let Some(ketchup_model::document::InstancePathStep::Group(id)) =
                    path.steps().last()
                {
                    snapshot
                        .resolve_instance_path(path)
                        .ok()
                        .and_then(|resolved| {
                            snapshot.local_group(ketchup_model::document::LocalGroupKey {
                                definition_id: resolved.definition_id,
                                local_id: *id,
                            })
                        })
                        .map(|group| group.name().to_owned())
                } else {
                    crate::rule_program_part_name(snapshot, path)
                };
                json!({"instance_path":path, "name":name})
            }
        };
        if !complete {
            unchecked.push(json!({"group":identity, "reason":"group_contact_unverified"}));
            continue;
        }
        checked += 1;
        let mut parents = (0..members.len()).collect::<Vec<_>>();
        for (left, a) in members.iter().enumerate() {
            for (right, b) in members.iter().enumerate().skip(left + 1) {
                let (a, b) = (&a.instance_path, &b.instance_path);
                if facts
                    .get(&(a.min(b).clone(), a.max(b).clone()))
                    .is_some_and(|pair| {
                        pair.penetrating()
                            || pair
                                .distance_mm
                                .is_some_and(|d| d <= snapshot.tolerance().linear_mm())
                    })
                {
                    let (a, b) = (root(&parents, left), root(&parents, right));
                    parents[b] = a;
                }
            }
        }
        let mut islands = BTreeMap::<usize, Vec<Value>>::new();
        for (index, member) in members.iter().enumerate() {
            islands
                .entry(root(&parents, index))
                .or_default()
                .push(json!({
                    "instance_path":member.instance_path,
                    "name":crate::rule_program_part_name(snapshot, &member.instance_path),
                    "origin_mm":member.transform.transform_point([0.0; 3]),
                }));
        }
        if islands.len() > 1 {
            issues.push(json!({"validator":ID, "kind":"disconnected_group", "code":"group.disconnected", "severity":"warning",
                "occurrence_ids":members.iter().map(|member| member.instance_path.root_occurrence().0).collect::<Vec<_>>(),
                "names":members.iter().map(|member| crate::rule_program_part_name(snapshot, &member.instance_path)).collect::<Vec<_>>(),
                "rule":"Group members form separate sets without physical contact.",
                "group":identity, "islands":islands.into_values().collect::<Vec<_>>(),
                "message":"Group members form separate sets without physical contact.",
                "fix_hint":"Move separated members into contact or put independent bodies in separate groups. Grounding and declared joints do not establish contact."}));
        }
    }
    if !complete && unchecked.is_empty() {
        unchecked
            .push(json!({"reason":"group_contact_unverified", "detail":coverage["not_evaluated"]}));
    }
    let mut report = json!({"state":if !issues.is_empty() {"failed"} else if complete {"passed"} else {"not_evaluated"},
        "complete":complete, "checked_group_count":checked, "issue_count":issues.len(),
        "issues_complete":true, "issues":issues, "not_evaluated":unchecked});
    crate::contact_joints::append_report(snapshot, facts, coverage, &mut report);
    report
}

pub(crate) fn finish_collision(
    mut collision: Value,
    snapshot: &Snapshot,
    selection: &AssistantValidationSelection,
    facts: Option<&ExactPairFacts>,
) -> Value {
    if selection.requested.contains(ID) {
        collision[ID] = report(
            snapshot,
            facts.unwrap_or(&ExactPairFacts::new()),
            &collision,
        );
    }
    if !selection.requested.contains("collision") {
        collision["state"] = json!("skipped");
        collision["complete"] = json!(false);
        for field in [
            "checked_occurrence_count",
            "checked_body_count",
            "checked_pair_count",
            "total_pair_count",
            "broad_phase_rejected_pair_count",
            "narrow_phase_pair_count",
            "hull_decided_pair_count",
            "issue_count",
        ] {
            collision[field] = json!(0);
        }
        for field in ["issues", "not_evaluated", "unavailable_occurrences"] {
            collision[field] = json!([]);
        }
    }
    collision
}

pub(crate) fn validation_context(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
    mut collision: Value,
    contacts: &[GravitySupportContact],
) -> Value {
    if selection.is_valid() && selection.requested.contains(ID) && collision.get(ID).is_none() {
        collision = finish_collision(collision, snapshot, selection, None);
    }
    let group_report = collision.as_object_mut().and_then(|object| object.remove(ID)).unwrap_or_else(||
        json!({"state":"not_evaluated", "complete":false, "issue_count":0, "issues":[],
            "not_evaluated":[{"reason":"group_contact_unverified", "detail":collision["not_evaluated"]}]}));
    let mut result = crate::validation::assistant_validation_context_base(
        snapshot,
        exact_results,
        selection,
        collision,
        contacts,
    );
    if !selection.is_valid() || !selection.requested.contains(ID) {
        result[ID] = json!({"state":"skipped", "complete":false, "issue_count":0, "issues":[]});
        return result;
    }
    let complete = group_report["complete"] == true;
    let sole = selection.requested.len() == 1;
    result["complete"] = json!(complete && (sole || result["complete"] == true));
    if let Some(issues) = result["issues"].as_array_mut() {
        issues.extend(
            group_report["issues"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    result["issue_count"] = json!(
        result["issue_count"].as_u64().unwrap_or(0)
            + group_report["issue_count"].as_u64().unwrap_or(0)
    );
    if !complete && let Some(unchecked) = result["not_evaluated"].as_array_mut() {
        unchecked.push(json!({"validator":ID, "reason":"group_contact_unverified"}));
    }
    if group_report["state"] == "failed" {
        result["state"] = json!("failed");
    } else if !complete && result["state"] != "failed" {
        result["state"] = json!("not_evaluated");
    }
    result[ID] = group_report;
    result
}
