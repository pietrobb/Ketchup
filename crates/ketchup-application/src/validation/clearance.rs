//! `clearance_occupancy`: visible bodies that intrude into a declared clearance
//! volume, such as the swing of a door or a service zone.

use super::{MAX_ASSISTANT_VALIDATION_ISSUES, assistant_evidence_label};
use ketchup_model::document::{OccurrenceId, Snapshot};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::exact_validation::GeneralBodyParticipant;
use ketchup_model::space::{
    CanonicalClearanceVolume, ClearanceOwner, ClearanceSeverity, validate_clearance_occupancy,
};
use std::collections::BTreeMap;

pub fn assistant_clearance_occupancy_report(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    selected: bool,
    coverage_complete: bool,
) -> serde_json::Value {
    if !selected {
        return serde_json::json!({
            "state": "skipped",
            "complete": false,
            "applicable_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "evaluations": [],
            "issues": [],
            "not_evaluated": [],
        });
    }
    let volumes = snapshot.clearance_volumes().collect::<Vec<_>>();
    if volumes.is_empty() {
        return serde_json::json!({
            "state": "not_evaluated",
            "complete": false,
            "applicable_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "evaluations": [],
            "issues": [],
            "not_evaluated": [{
                "validator": "clearance_occupancy",
                "reason": "clearance_volume_not_found",
                "required_input": "a clearance volume declared with upsert_clearance_volume",
            }],
        });
    }
    let paths = participants
        .iter()
        .map(|participant| participant.instance_path().clone())
        .collect::<Vec<_>>();
    let mut evaluations = Vec::new();
    let mut issues = Vec::new();
    let mut not_evaluated = Vec::new();
    let mut required_occupied = false;
    for clearance in &volumes {
        let id = clearance.id().0;
        let result = match validate_clearance_occupancy(
            snapshot,
            exact_results,
            clearance.id(),
            paths.iter().cloned(),
        ) {
            Ok(result) => result,
            Err(error) => {
                not_evaluated.push(serde_json::json!({
                    "validator": "clearance_occupancy",
                    "clearance_volume_id": id,
                    "reason": "clearance_volume_unverifiable",
                    "detail": error.to_string(),
                }));
                continue;
            }
        };
        let severity = match clearance.severity() {
            ClearanceSeverity::Required => "warning",
            ClearanceSeverity::Advisory => "advisory",
        };
        evaluations.push(serde_json::json!({
            "clearance_volume_id": id,
            "reason": clearance.reason(),
            "owner": owner(snapshot, clearance, names),
            "severity": severity,
            "checked_body_count": paths.len(),
            "occupant_count": result.occupants.len(),
            "evidence": {
                "exact": result.evidence_counts.exact,
                "tolerant": result.evidence_counts.tolerant,
            },
            "result": if result.is_clear() { "passed" } else { "failed" },
        }));
        if !result.is_clear() && clearance.severity() == ClearanceSeverity::Required {
            required_occupied = true;
        }
        for occupant in &result.occupants {
            let occurrence_id = occupant.root_occurrence();
            let evidence = participants
                .iter()
                .find(|participant| participant.instance_path() == occupant)
                .map(|participant| assistant_evidence_label(participant.evidence_class()));
            issues.push(serde_json::json!({
                "code": "clearance.occupied",
                "severity": severity,
                "clearance_volume_id": id,
                "clearance_reason": clearance.reason(),
                "occurrence_id": occurrence_id.0,
                "name": names.get(&occurrence_id),
                "instance_path": occupant,
                "evidence_class": evidence,
                "rule": "a body must not intrude into a declared clearance volume by more than its tolerance",
            }));
        }
    }
    let issue_count = issues.len();
    let state = if required_occupied {
        "failed"
    } else if !coverage_complete || !not_evaluated.is_empty() {
        "not_evaluated"
    } else {
        "passed"
    };
    serde_json::json!({
        "state": state,
        "complete": coverage_complete
            && not_evaluated.is_empty()
            && issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "applicable_count": volumes.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "assumptions": [
            "clearance volumes are world-axis boxes; a body occupies one when its world axis-aligned bounds overlap the box with positive volume beyond the volume's tolerance",
            "the clearance owner itself never occupies its own volume",
            "advisory volumes report occupants as advisories without failing the check",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated,
    })
}

fn owner(
    snapshot: &Snapshot,
    clearance: &CanonicalClearanceVolume,
    names: &BTreeMap<OccurrenceId, String>,
) -> serde_json::Value {
    match clearance.owner() {
        ClearanceOwner::Occurrence(path) => serde_json::json!({
            "kind": "occurrence",
            "occurrence_id": path.root_occurrence().0,
            "name": names.get(&path.root_occurrence()),
            "instance_path": path,
        }),
        ClearanceOwner::Space(id) => serde_json::json!({
            "kind": "space",
            "space_id": id.0,
            "purpose": snapshot.space(*id).map(|space| space.purpose()),
        }),
    }
}
