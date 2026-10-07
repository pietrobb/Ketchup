//! Qualification of declared capacities, not a strength model derived from geometry.
use ketchup_model::document::{OccurrenceId, Snapshot};
use ketchup_model::tolerance::ROUNDING;
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const LOAD_MODE: &str = "ketchup.static-load-mode.v1";
pub(super) const CAPACITY: &str = "ketchup.support-capacity.v1";

pub(super) fn classification<'a>(
    snapshot: &'a Snapshot,
    occurrence: OccurrenceId,
    name: &str,
) -> Result<&'a str, Value> {
    let missing = || json!({"reason": "missing_capacity_qualification", "required_input": name});
    let dimension = snapshot.classification_dimension_named(name).map_err(|error| {
        json!({"reason": "ambiguous_capacity_qualification", "required_input": name, "cause": error.to_string()})
    })?.ok_or_else(missing)?;
    let category = snapshot
        .occurrence_classification(occurrence, dimension.id())
        .ok_or_else(missing)?;
    let value = dimension.category(category).ok_or_else(missing)?.name();
    if value.trim().is_empty() {
        return Err(missing());
    }
    Ok(value)
}

pub(super) fn support_inputs(
    snapshot: &Snapshot,
    names: &std::collections::BTreeMap<OccurrenceId, String>,
    loads: impl Iterator<Item = OccurrenceId>,
    supports: &std::collections::BTreeSet<OccurrenceId>,
    capacities: &std::collections::BTreeMap<OccurrenceId, Vec<(u64, f64)>>,
    direction: [f64; 3],
    load_case: &str,
) -> Result<Vec<Value>, Value> {
    let mut mode = None;
    for occurrence in loads {
        let value = classification(snapshot, occurrence, LOAD_MODE).map_err(|mut error| {
            error["occurrence_id"] = json!(occurrence.0);
            error
        })?;
        if mode.is_some_and(|previous| previous != value) {
            return Err(
                json!({"reason": "mixed_load_modes_in_case", "occurrence_id": occurrence.0}),
            );
        }
        mode = Some(value);
    }
    supports
        .iter()
        .map(|id| {
            let qualification = qualify(
                snapshot,
                *id,
                mode.unwrap_or_default(),
                direction,
                supports.len() > 1,
            )
            .map_err(|mut error| {
                error["occurrence_id"] = json!(id.0);
                error
            })?;
            let (node, capacity) = capacities[id][0];
            Ok(
                json!({"occurrence_id": id.0, "name": names.get(id), "role_case": load_case,
            "capacity_node_id": node, "capacity_n": capacity, "qualification": qualification}),
            )
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capacity {
    source: String,
    units: String,
    mode: String,
    direction_world: [f64; 3],
    assumptions: String,
    additive: bool,
}

pub(super) fn qualify(
    snapshot: &Snapshot,
    occurrence: OccurrenceId,
    mode: &str,
    direction: [f64; 3],
    multiple_supports: bool,
) -> Result<Value, Value> {
    let raw = classification(snapshot, occurrence, CAPACITY)?;
    if raw.len() > ketchup_model::tolerance::limits::TEXT_BYTES {
        return Err(
            json!({"reason": "capacity_qualification_too_long", "max_bytes": ketchup_model::tolerance::limits::TEXT_BYTES}),
        );
    }
    let declared: Capacity = serde_json::from_str(raw).map_err(|error| {
        json!({"reason": "invalid_capacity_qualification", "required_input": CAPACITY, "cause": error.to_string()})
    })?;
    if declared.source.trim().is_empty() || declared.assumptions.trim().is_empty() {
        return Err(
            json!({"reason": "missing_capacity_source_or_assumptions", "required_input": CAPACITY}),
        );
    }
    if declared.units != "N" {
        return Err(
            json!({"reason": "unsupported_capacity_units", "declared_units": declared.units, "required_units": "N"}),
        );
    }
    if declared.mode != mode {
        return Err(
            json!({"reason": "uncovered_load_mode", "load_mode": mode, "capacity_mode": declared.mode}),
        );
    }
    let length = declared
        .direction_world
        .iter()
        .fold(0.0_f64, |length, v| length.hypot(*v));
    if !length.is_finite() || length <= 0.0 {
        return Err(
            json!({"reason": "invalid_capacity_direction", "required_input": "nonzero finite direction_world"}),
        );
    }
    let unit = declared.direction_world.map(|v| v / length);
    if unit
        .iter()
        .zip(direction)
        .any(|(a, b)| (a - b).abs() > ROUNDING)
    {
        return Err(
            json!({"reason": "uncovered_load_direction", "load_direction_world": direction, "capacity_direction_world": unit}),
        );
    }
    if multiple_supports && !declared.additive {
        return Err(
            json!({"reason": "capacity_additivity_not_declared", "required_input": "additive with documented load-sharing assumptions"}),
        );
    }
    Ok(json!({
        "source": declared.source, "units": declared.units, "mode": declared.mode,
        "direction_world": unit, "assumptions": declared.assumptions, "additive": declared.additive,
        "basis": "user_declared_capacity_not_independently_verified"
    }))
}
