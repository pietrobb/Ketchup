use ketchup_core::document::{OccurrenceId, Snapshot};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::exact_validation::*;
use ketchup_core::prismatic::TolerancePolicy;
use ketchup_core::validation::{
    DiagnosticSeverity, EvidenceClass, HostNeutralValidator, VALIDATOR_ROLE_DIMENSION_V1,
    ValidationExecution, ValidationInvocation, ValidationState, ValidatorRoleError,
    ValidatorRoleIndex,
};
use std::collections::{BTreeMap, BTreeSet};
const MAX_ASSISTANT_VALIDATION_OCCURRENCES: usize = 100;
const MAX_ASSISTANT_VALIDATION_PATH_STEPS: usize = 256;
const MAX_ASSISTANT_VALIDATION_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ASSISTANT_VALIDATION_ISSUES: usize = 100;
const MAX_STRUCTURAL_SCOPE_OCCURRENCES: usize = 10_000;
const MAX_STRUCTURAL_SCOPE_LOADS: usize = MAX_ASSISTANT_VALIDATION_ISSUES;
const MAX_STRUCTURAL_SCOPE_ROLE_ASSIGNMENTS: usize = 10_000;
const MAX_STRUCTURAL_SCOPE_DEPENDENCIES: usize = 10_000;
const MAX_STRUCTURAL_SCOPE_PARAMETERS: usize = 30_000;
const MAX_STRUCTURAL_CLASSIFICATION_DIMENSIONS: usize = 100;
const MAX_STRUCTURAL_ROLE_CATEGORIES: usize = 10_000;
pub const ASSISTANT_VALIDATOR_IDS: [&str; 9] = [
    "collision",
    "gravity_support",
    "shelf_deflection",
    "tipping",
    "anchoring",
    "hardware_manufacturing",
    "room_placement",
    "passage_clearance",
    "static_load",
];
/// What each validator actually checks, in provider-facing English. The
/// Assistant cannot honestly offer a validator it cannot describe, so this
/// catalog travels with every validation context and is what the
/// `list_validators` sidecar tool reads.
pub const ASSISTANT_VALIDATOR_CATALOG: [(&str, &str); 9] = [
    (
        "collision",
        "solid bodies that overlap each other instead of touching",
    ),
    (
        "gravity_support",
        "parts that are not carried, directly or transitively, by the ground along the declared gravity axis",
    ),
    (
        "shelf_deflection",
        "shelf sag under the declared design load against the span and absolute deflection limits",
    ),
    (
        "tipping",
        "free-standing bodies that tip below the minimum safe tilt angle",
    ),
    (
        "anchoring",
        "tall shallow furniture that must be anchored to the wall",
    ),
    (
        "hardware_manufacturing",
        "hole edge material, hole spacing, hinge cup envelopes, drawer-slide pair alignment and minimum panel thickness",
    ),
    (
        "room_placement",
        "furniture that leaves the declared room volume",
    ),
    (
        "passage_clearance",
        "walking passages narrower or lower than the declared minimum",
    ),
    (
        "static_load",
        "declared static loads against the supports that actually carry them",
    ),
];
pub const SHELF_DESIGN_LOAD_N: f64 = 500.0;
pub const SHELF_ELASTIC_MODULUS_N_MM2: f64 = 2_500.0;
pub const SHELF_DEFLECTION_SPAN_RATIO: f64 = 200.0;
pub const SHELF_MAX_DEFLECTION_MM: f64 = 5.0;
pub const MINIMUM_TIP_ANGLE_DEGREES: f64 = 15.0;
pub const ANCHORING_MINIMUM_HEIGHT_MM: f64 = 1_000.0;
pub const ANCHORING_MINIMUM_HEIGHT_DEPTH_RATIO: f64 = 2.0;
pub const MINIMUM_HOLE_EDGE_MATERIAL_MM: f64 = 5.0;
pub const MINIMUM_HOLE_SPACING_MATERIAL_MM: f64 = 3.0;
pub const MINIMUM_HINGE_CUP_DIAMETER_MM: f64 = 35.0;
pub const MINIMUM_HINGE_CUP_DEPTH_MM: f64 = 12.0;
pub const MAXIMUM_DRAWER_SLIDE_LENGTH_MISMATCH_MM: f64 = 1.0;
pub const MAXIMUM_DRAWER_SLIDE_VERTICAL_MISMATCH_MM: f64 = 1.0;
pub const MINIMUM_PANEL_THICKNESS_MM: f64 = 6.0;
pub const MINIMUM_PASSAGE_WIDTH_MM: f64 = 900.0;
pub const MINIMUM_PASSAGE_HEADROOM_MM: f64 = 2_000.0;
pub const ROOM_PLACEMENT_TOLERANCE_MM: f64 = 0.1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantValidationSelection {
    pub mode: &'static str,
    pub requested: BTreeSet<&'static str>,
    pub unknown: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct StructuralValidationScope {
    document_id: u64,
    revision: u64,
    canonical_digest: String,
    occurrence_ids: BTreeSet<OccurrenceId>,
    resource_limit_exceeded: bool,
}

impl StructuralValidationScope {
    pub fn bind(
        snapshot: &Snapshot,
        occurrence_ids: impl IntoIterator<Item = OccurrenceId>,
    ) -> Self {
        let mut bound_occurrence_ids = BTreeSet::new();
        let mut resource_limit_exceeded = false;
        for (index, occurrence_id) in occurrence_ids
            .into_iter()
            .take(MAX_STRUCTURAL_SCOPE_OCCURRENCES + 1)
            .enumerate()
        {
            if index == MAX_STRUCTURAL_SCOPE_OCCURRENCES {
                resource_limit_exceeded = true;
                break;
            }
            bound_occurrence_ids.insert(occurrence_id);
        }
        Self {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            occurrence_ids: bound_occurrence_ids,
            resource_limit_exceeded,
        }
    }

    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id().0
            && self.revision == snapshot.revision_id()
            && self.canonical_digest == snapshot.canonical_digest()
    }

    pub fn occurrence_ids(&self) -> &BTreeSet<OccurrenceId> {
        &self.occurrence_ids
    }
}

fn scoped_static_load_unavailable(
    snapshot: &Snapshot,
    scope: &StructuralValidationScope,
    reason: &'static str,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "ketchup.scoped-static-load.v1",
        "document_id": snapshot.document_id().0,
        "revision": snapshot.revision_id(),
        "canonical_digest": snapshot.canonical_digest(),
        "state": "not_evaluated",
        "complete": false,
        "applicable_count": 0,
        "issue_count": 0,
        "issues_complete": true,
        "evaluations": [],
        "issues": [],
        "not_evaluated": [{
            "validator": "static_load",
            "reason": reason,
        }],
        "coverage": {
            "complete": false,
            "requested_occurrence_count": scope.occurrence_ids.len(),
            "checked_load_occurrence_count": 0,
            "boundary_support_occurrence_ids": [],
            "exact_geometry_loaded_count": 0,
            "limits": {
                "scope_occurrences": MAX_STRUCTURAL_SCOPE_OCCURRENCES,
                "loads": MAX_STRUCTURAL_SCOPE_LOADS,
                "role_assignments": MAX_STRUCTURAL_SCOPE_ROLE_ASSIGNMENTS,
                "dependencies": MAX_STRUCTURAL_SCOPE_DEPENDENCIES,
                "evaluator_parameters": MAX_STRUCTURAL_SCOPE_PARAMETERS,
                "classification_dimensions": MAX_STRUCTURAL_CLASSIFICATION_DIMENSIONS,
                "role_categories": MAX_STRUCTURAL_ROLE_CATEGORIES,
                "text_bytes": MAX_ASSISTANT_VALIDATION_TEXT_BYTES,
            },
        },
    })
}

pub fn scoped_static_load_report(
    snapshot: &Snapshot,
    scope: &StructuralValidationScope,
    mut cancellation_requested: impl FnMut() -> bool,
) -> serde_json::Value {
    if !scope.is_current(snapshot) {
        return scoped_static_load_unavailable(snapshot, scope, "stale_structural_scope");
    }
    if scope.resource_limit_exceeded {
        return scoped_static_load_unavailable(snapshot, scope, "structural_scope_resource_limit");
    }
    if scope.occurrence_ids.is_empty() {
        return scoped_static_load_unavailable(snapshot, scope, "empty_structural_scope");
    }
    if scope.occurrence_ids.len() > MAX_STRUCTURAL_SCOPE_OCCURRENCES {
        return scoped_static_load_unavailable(snapshot, scope, "structural_scope_resource_limit");
    }
    if scope.occurrence_ids.len() > MAX_STRUCTURAL_SCOPE_LOADS {
        return scoped_static_load_unavailable(snapshot, scope, "structural_load_resource_limit");
    }
    if snapshot
        .occurrences()
        .take(MAX_STRUCTURAL_SCOPE_OCCURRENCES + 1)
        .count()
        > MAX_STRUCTURAL_SCOPE_OCCURRENCES
    {
        return scoped_static_load_unavailable(snapshot, scope, "structural_model_resource_limit");
    }
    if cancellation_requested() {
        return scoped_static_load_unavailable(snapshot, scope, "validation_cancelled");
    }

    let mut classification_dimension_count = 0;
    for dimension in snapshot.classification_dimensions() {
        classification_dimension_count += 1;
        if classification_dimension_count > MAX_STRUCTURAL_CLASSIFICATION_DIMENSIONS {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "structural_classification_resource_limit",
            );
        }
        if dimension.name() == VALIDATOR_ROLE_DIMENSION_V1
            && dimension
                .categories()
                .take(MAX_STRUCTURAL_ROLE_CATEGORIES + 1)
                .count()
                > MAX_STRUCTURAL_ROLE_CATEGORIES
        {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "structural_role_category_resource_limit",
            );
        }
        if cancellation_requested() {
            return scoped_static_load_unavailable(snapshot, scope, "validation_cancelled");
        }
    }

    let roles = ValidatorRoleIndex::from_snapshot(snapshot);
    let Ok(role_index) = &roles else {
        return scoped_static_load_unavailable(
            snapshot,
            scope,
            "missing_or_ambiguous_canonical_roles",
        );
    };
    if role_index.assignments().count() > MAX_STRUCTURAL_SCOPE_ROLE_ASSIGNMENTS {
        return scoped_static_load_unavailable(
            snapshot,
            scope,
            "structural_role_assignment_resource_limit",
        );
    }

    let mut load_ids = BTreeSet::new();
    let mut load_cases = BTreeSet::new();
    for occurrence_id in &scope.occurrence_ids {
        if cancellation_requested() {
            return scoped_static_load_unavailable(snapshot, scope, "validation_cancelled");
        }
        let Some(_) = snapshot.occurrence(*occurrence_id) else {
            return scoped_static_load_unavailable(snapshot, scope, "scoped_occurrence_missing");
        };
        if snapshot.occurrence_effectively_visible(*occurrence_id) != Some(true) {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "scoped_occurrence_not_visible",
            );
        }
        let Some(role) = role_index
            .role(*occurrence_id)
            .and_then(|role| assistant_physics_role(role.as_str()))
        else {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "missing_or_invalid_static_load_role",
            );
        };
        if role.kind != AssistantPhysicsRoleKind::StaticLoad {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "missing_or_invalid_static_load_role",
            );
        }
        load_ids.insert(*occurrence_id);
        if !load_cases.insert(role.group.to_owned()) {
            return scoped_static_load_unavailable(
                snapshot,
                scope,
                "shared_static_load_case_requires_aggregate_evaluation",
            );
        }
    }

    let mut boundary_support_ids = BTreeSet::new();
    for assignment in role_index.assignments() {
        if cancellation_requested() {
            return scoped_static_load_unavailable(snapshot, scope, "validation_cancelled");
        }
        if let Some(role) = assistant_physics_role(assignment.role.as_str())
            && load_cases.contains(role.group)
        {
            match role.kind {
                AssistantPhysicsRoleKind::StaticLoad
                    if !load_ids.contains(&assignment.occurrence_id) =>
                {
                    return scoped_static_load_unavailable(
                        snapshot,
                        scope,
                        "shared_static_load_case_outside_scope",
                    );
                }
                AssistantPhysicsRoleKind::StaticSupport => {
                    boundary_support_ids.insert(assignment.occurrence_id);
                    if boundary_support_ids.len() > MAX_STRUCTURAL_SCOPE_DEPENDENCIES {
                        return scoped_static_load_unavailable(
                            snapshot,
                            scope,
                            "structural_dependency_resource_limit",
                        );
                    }
                }
                _ => {}
            }
        }
    }

    let mut names = BTreeMap::new();
    let mut text_bytes = 0usize;
    for occurrence_id in load_ids.iter().chain(&boundary_support_ids) {
        if let Some(occurrence) = snapshot.occurrence(*occurrence_id)
            && snapshot.occurrence_effectively_visible(*occurrence_id) == Some(true)
        {
            text_bytes = text_bytes.saturating_add(occurrence.name().len());
            if text_bytes > MAX_ASSISTANT_VALIDATION_TEXT_BYTES {
                return scoped_static_load_unavailable(
                    snapshot,
                    scope,
                    "structural_text_resource_limit",
                );
            }
            names.insert(*occurrence_id, occurrence.name().to_owned());
        }
    }
    let mut report = assistant_static_load_report_filtered(
        snapshot,
        &names,
        &roles,
        true,
        true,
        Some(&load_ids),
        &mut cancellation_requested,
    );
    let coverage_complete = report["complete"].as_bool() == Some(true);
    let checked_load_count = report["applicable_count"].as_u64().unwrap_or(0);
    if let Some(object) = report.as_object_mut() {
        object.insert(
            "schema".into(),
            serde_json::json!("ketchup.scoped-static-load.v1"),
        );
        object.insert(
            "document_id".into(),
            serde_json::json!(snapshot.document_id().0),
        );
        object.insert("revision".into(), serde_json::json!(snapshot.revision_id()));
        object.insert(
            "canonical_digest".into(),
            serde_json::json!(snapshot.canonical_digest()),
        );
        object.insert(
            "coverage".into(),
            serde_json::json!({
                "complete": coverage_complete,
                "requested_occurrence_count": scope.occurrence_ids.len(),
                "checked_load_occurrence_count": checked_load_count,
                "boundary_support_occurrence_ids": boundary_support_ids
                    .iter()
                    .map(|id| id.0)
                    .collect::<Vec<_>>(),
                "exact_geometry_loaded_count": 0,
                "limits": {
                    "scope_occurrences": MAX_STRUCTURAL_SCOPE_OCCURRENCES,
                    "loads": MAX_STRUCTURAL_SCOPE_LOADS,
                    "role_assignments": MAX_STRUCTURAL_SCOPE_ROLE_ASSIGNMENTS,
                    "dependencies": MAX_STRUCTURAL_SCOPE_DEPENDENCIES,
                    "evaluator_parameters": MAX_STRUCTURAL_SCOPE_PARAMETERS,
                    "classification_dimensions": MAX_STRUCTURAL_CLASSIFICATION_DIMENSIONS,
                    "role_categories": MAX_STRUCTURAL_ROLE_CATEGORIES,
                "text_bytes": MAX_ASSISTANT_VALIDATION_TEXT_BYTES,
                },
            }),
        );
    }
    report
}

impl AssistantValidationSelection {
    pub fn all(mode: &'static str) -> Self {
        Self {
            mode,
            requested: ASSISTANT_VALIDATOR_IDS.into_iter().collect(),
            unknown: Vec::new(),
        }
    }
    /// Exact catalog identifiers, not natural language. Empty/unknown selections fail closed.
    pub fn only(names: &[&str]) -> Self {
        let mut result = Self {
            mode: "only",
            requested: BTreeSet::new(),
            unknown: Vec::new(),
        };
        for name in names {
            match ASSISTANT_VALIDATOR_IDS.into_iter().find(|id| id == name) {
                Some(id) => {
                    result.requested.insert(id);
                }
                None => result.unknown.push((*name).to_owned()),
            }
        }
        result
    }
    pub fn is_valid(&self) -> bool {
        self.unknown.is_empty()
            && !self.requested.is_empty()
            && self
                .requested
                .iter()
                .all(|id| ASSISTANT_VALIDATOR_IDS.contains(id))
    }
}
pub fn assistant_validator_catalog() -> Vec<serde_json::Value> {
    ASSISTANT_VALIDATOR_CATALOG
        .into_iter()
        .map(|(id, checks)| serde_json::json!({ "id": id, "checks": checks }))
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantSpatialRoleKind {
    Room,
    Furniture,
    Passage {
        surface_axes: [usize; 2],
        height_axis: usize,
    },
    Obstacle,
    Context,
}

#[derive(Clone, Copy, Debug)]
pub struct AssistantSpatialRole<'a> {
    pub kind: AssistantSpatialRoleKind,
    pub group: &'a str,
}

pub fn assistant_spatial_role(role: &str) -> Option<AssistantSpatialRole<'_>> {
    let (role, group) = role.split_once(':')?;
    if group.is_empty() {
        return None;
    }
    let kind = match role {
        "spatial.room" => AssistantSpatialRoleKind::Room,
        "spatial.furniture" => AssistantSpatialRoleKind::Furniture,
        "spatial.passage.xy" => AssistantSpatialRoleKind::Passage {
            surface_axes: [0, 1],
            height_axis: 2,
        },
        "spatial.passage.xz" => AssistantSpatialRoleKind::Passage {
            surface_axes: [0, 2],
            height_axis: 1,
        },
        "spatial.passage.yz" => AssistantSpatialRoleKind::Passage {
            surface_axes: [1, 2],
            height_axis: 0,
        },
        "spatial.obstacle" => AssistantSpatialRoleKind::Obstacle,
        "spatial.context" => AssistantSpatialRoleKind::Context,
        _ => return None,
    };
    Some(AssistantSpatialRole { kind, group })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantPhysicsRoleKind {
    GravityBody,
    GravityGround,
    StaticLoad,
    StaticSupport,
}

#[derive(Clone, Copy, Debug)]
pub struct AssistantPhysicsRole<'a> {
    pub kind: AssistantPhysicsRoleKind,
    pub group: &'a str,
}

pub fn assistant_physics_role(role: &str) -> Option<AssistantPhysicsRole<'_>> {
    let (role, group) = role.split_once(':')?;
    if group.is_empty() {
        return None;
    }
    let kind = match role {
        "physics.gravity.body" => AssistantPhysicsRoleKind::GravityBody,
        "physics.gravity.ground" => AssistantPhysicsRoleKind::GravityGround,
        "physics.static.load" => AssistantPhysicsRoleKind::StaticLoad,
        "physics.static.support" => AssistantPhysicsRoleKind::StaticSupport,
        _ => return None,
    };
    Some(AssistantPhysicsRole { kind, group })
}

pub fn assistant_evidence_label(evidence: &EvidenceClass) -> &'static str {
    match evidence {
        EvidenceClass::Exact => "exact",
        EvidenceClass::Tolerant(_) => "tolerant",
    }
}

pub fn assistant_shelf_role_axes(role: &str) -> Option<([usize; 2], usize)> {
    match role {
        "furniture.shelf.xy" => Some(([0, 1], 2)),
        "furniture.shelf.xz" => Some(([0, 2], 1)),
        "furniture.shelf.yz" => Some(([1, 2], 0)),
        _ => None,
    }
}

pub fn assistant_case_role_axis(role: &str) -> Option<usize> {
    match role {
        "furniture.case.x" => Some(0),
        "furniture.case.y" => Some(1),
        "furniture.case.z" => Some(2),
        _ => None,
    }
}

pub fn assistant_shelf_deflection_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
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
        });
    }
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "not_evaluated": [],
                "issues": [],
            });
        }
    };

    let mut evaluations = Vec::new();
    let mut not_evaluated = Vec::new();
    let mut issues = Vec::new();
    for participant in participants {
        let occurrence_id = participant.instance_path().root_occurrence();
        let Some(role) = roles.role(occurrence_id) else {
            continue;
        };
        let Some((surface_axes, thickness_axis)) = assistant_shelf_role_axes(role.as_str()) else {
            continue;
        };
        let name = names
            .get(&occurrence_id)
            .expect("validated visible participants retain display names");
        let geometry = participant.geometry_evidence();
        let vertical_alignment = geometry
            .source_axis_world_z_alignment(thickness_axis)
            .expect("declared source axis is bounded");
        if vertical_alignment < 1.0 - 1.0e-9 {
            not_evaluated.push(serde_json::json!({
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "reason": "declared shelf thickness axis is not aligned with world gravity",
                "source_axis_world_z_alignment": vertical_alignment,
            }));
            continue;
        }
        let dimensions = geometry.source_frame_extents_mm();
        let span_mm = dimensions[surface_axes[0]].max(dimensions[surface_axes[1]]);
        let depth_mm = dimensions[surface_axes[0]].min(dimensions[surface_axes[1]]);
        let thickness_mm = dimensions[thickness_axis];
        let second_moment_mm4 = depth_mm * thickness_mm.powi(3) / 12.0;
        let line_load_n_mm = SHELF_DESIGN_LOAD_N / span_mm;
        let predicted_deflection_mm = 5.0 * line_load_n_mm * span_mm.powi(4)
            / (384.0 * SHELF_ELASTIC_MODULUS_N_MM2 * second_moment_mm4);
        let allowable_deflection_mm =
            (span_mm / SHELF_DEFLECTION_SPAN_RATIO).min(SHELF_MAX_DEFLECTION_MM);
        let failed = predicted_deflection_mm > allowable_deflection_mm;
        let evaluation = serde_json::json!({
            "occurrence_id": occurrence_id.0,
            "name": name,
            "role": role.as_str(),
            "role_source": "canonical_classification",
            "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
            "evidence_class": assistant_evidence_label(participant.evidence_class()),
            "span_mm": span_mm,
            "depth_mm": depth_mm,
            "thickness_mm": thickness_mm,
            "design_load_n": SHELF_DESIGN_LOAD_N,
            "elastic_modulus_n_mm2": SHELF_ELASTIC_MODULUS_N_MM2,
            "support_model": "simply_supported_uniform_load",
            "predicted_deflection_mm": predicted_deflection_mm,
            "allowable_deflection_mm": allowable_deflection_mm,
            "limit": "min(span/200, 5 mm)",
            "result": if failed { "failed" } else { "passed" },
        });
        if failed {
            issues.push(serde_json::json!({
                "code": "furniture.shelf_deflection_exceeded",
                "severity": "warning",
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "role_source": "canonical_classification",
                "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
                "evidence_class": assistant_evidence_label(participant.evidence_class()),
                "span_mm": span_mm,
                "depth_mm": depth_mm,
                "thickness_mm": thickness_mm,
                "design_load_n": SHELF_DESIGN_LOAD_N,
                "elastic_modulus_n_mm2": SHELF_ELASTIC_MODULUS_N_MM2,
                "predicted_deflection_mm": predicted_deflection_mm,
                "allowable_deflection_mm": allowable_deflection_mm,
            }));
        }
        evaluations.push(evaluation);
    }
    let issue_count = issues.len();
    let complete = coverage_complete && not_evaluated.is_empty();
    let state = if !complete {
        "not_evaluated"
    } else if issue_count > 0 {
        "failed"
    } else {
        "passed"
    };
    serde_json::json!({
        "state": state,
        "complete": complete && issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "applicable_count": evaluations.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "inputs": {
            "design_load_n": SHELF_DESIGN_LOAD_N,
            "elastic_modulus_n_mm2": SHELF_ELASTIC_MODULUS_N_MM2,
            "support_model": "simply_supported_uniform_load",
        },
        "limit": {
            "span_ratio": SHELF_DEFLECTION_SPAN_RATIO,
            "maximum_mm": SHELF_MAX_DEFLECTION_MM,
        },
        "assumptions": [
            "canonical validator roles declare shelf plane and thickness axis",
            "source-frame extents are bound to the accepted body geometry",
            "uniform 500 N service load",
            "2500 N/mm2 engineered-wood elastic modulus",
            "simple supports at both span ends",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
    })
}

pub fn assistant_tipping_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
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
        });
    }
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "not_evaluated": [],
                "issues": [],
            });
        }
    };

    let mut evaluations = Vec::new();
    let mut not_evaluated = Vec::new();
    let mut issues = Vec::new();
    for participant in participants {
        let occurrence_id = participant.instance_path().root_occurrence();
        let Some(role) = roles.role(occurrence_id) else {
            continue;
        };
        let Some(vertical_axis) = assistant_case_role_axis(role.as_str()) else {
            continue;
        };
        let name = names
            .get(&occurrence_id)
            .expect("validated visible participants retain display names");
        let geometry = participant.geometry_evidence();
        let vertical_alignment = geometry
            .source_axis_world_z_alignment(vertical_axis)
            .expect("declared source axis is bounded");
        if vertical_alignment < 1.0 - 1.0e-9 {
            not_evaluated.push(serde_json::json!({
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "reason": "declared case-furniture vertical axis is not aligned with world gravity",
                "source_axis_world_z_alignment": vertical_alignment,
            }));
            continue;
        }
        let dimensions = geometry.source_frame_extents_mm();
        let base_axes = (0..3)
            .filter(|axis| *axis != vertical_axis)
            .collect::<Vec<_>>();
        let base_depth_mm = dimensions[base_axes[0]].min(dimensions[base_axes[1]]);
        let height_mm = dimensions[vertical_axis];
        let centre_of_mass_height_mm = height_mm / 2.0;
        let critical_tip_angle_degrees = (base_depth_mm / height_mm).atan().to_degrees();
        let failed = critical_tip_angle_degrees < MINIMUM_TIP_ANGLE_DEGREES;
        let evaluation = serde_json::json!({
            "occurrence_id": occurrence_id.0,
            "name": name,
            "role": role.as_str(),
            "role_source": "canonical_classification",
            "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
            "evidence_class": assistant_evidence_label(participant.evidence_class()),
            "base_depth_mm": base_depth_mm,
            "height_mm": height_mm,
            "centre_of_mass_height_mm": centre_of_mass_height_mm,
            "mass_model": "uniform_source_frame_envelope",
            "critical_tip_angle_degrees": critical_tip_angle_degrees,
            "minimum_tip_angle_degrees": MINIMUM_TIP_ANGLE_DEGREES,
            "result": if failed { "failed" } else { "passed" },
        });
        if failed {
            issues.push(serde_json::json!({
                "code": "furniture.tip_angle_below_limit",
                "severity": "warning",
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "role_source": "canonical_classification",
                "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
                "evidence_class": assistant_evidence_label(participant.evidence_class()),
                "base_depth_mm": base_depth_mm,
                "height_mm": height_mm,
                "centre_of_mass_height_mm": centre_of_mass_height_mm,
                "critical_tip_angle_degrees": critical_tip_angle_degrees,
                "minimum_tip_angle_degrees": MINIMUM_TIP_ANGLE_DEGREES,
            }));
        }
        evaluations.push(evaluation);
    }
    let issue_count = issues.len();
    let complete = coverage_complete && not_evaluated.is_empty();
    let state = if !complete {
        "not_evaluated"
    } else if issue_count > 0 {
        "failed"
    } else {
        "passed"
    };
    serde_json::json!({
        "state": state,
        "complete": complete && issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "applicable_count": evaluations.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "inputs": { "mass_model": "uniform_source_frame_envelope" },
        "limit": { "minimum_tip_angle_degrees": MINIMUM_TIP_ANGLE_DEGREES },
        "assumptions": [
            "canonical validator roles declare the case-furniture vertical axis",
            "source-frame extents are bound to the accepted body geometry",
            "mass is uniformly distributed inside the source-frame envelope",
            "the full reported base depth contacts a level floor",
            "no external pull or shelf load is included",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
    })
}

pub fn assistant_anchoring_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
    selected: bool,
    coverage_complete: bool,
) -> serde_json::Value {
    if !selected {
        return serde_json::json!({
            "state": "skipped",
            "complete": false,
            "applicable_count": 0,
            "required_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "evaluations": [],
            "issues": [],
        });
    }
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "required_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "not_evaluated": [],
                "issues": [],
            });
        }
    };

    let mut evaluations = Vec::new();
    let mut not_evaluated = Vec::new();
    let mut issues = Vec::new();
    for participant in participants {
        let occurrence_id = participant.instance_path().root_occurrence();
        let Some(role) = roles.role(occurrence_id) else {
            continue;
        };
        let Some(vertical_axis) = assistant_case_role_axis(role.as_str()) else {
            continue;
        };
        let name = names
            .get(&occurrence_id)
            .expect("validated visible participants retain display names");
        let geometry = participant.geometry_evidence();
        let vertical_alignment = geometry
            .source_axis_world_z_alignment(vertical_axis)
            .expect("declared source axis is bounded");
        if vertical_alignment < 1.0 - 1.0e-9 {
            not_evaluated.push(serde_json::json!({
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "reason": "declared case-furniture vertical axis is not aligned with world gravity",
                "source_axis_world_z_alignment": vertical_alignment,
            }));
            continue;
        }
        let dimensions = geometry.source_frame_extents_mm();
        let base_axes = (0..3)
            .filter(|axis| *axis != vertical_axis)
            .collect::<Vec<_>>();
        let base_depth_mm = dimensions[base_axes[0]].min(dimensions[base_axes[1]]);
        let height_mm = dimensions[vertical_axis];
        let height_depth_ratio = height_mm / base_depth_mm;
        let anchoring_required = height_mm >= ANCHORING_MINIMUM_HEIGHT_MM
            && height_depth_ratio >= ANCHORING_MINIMUM_HEIGHT_DEPTH_RATIO;
        let evaluation = serde_json::json!({
            "occurrence_id": occurrence_id.0,
            "name": name,
            "role": role.as_str(),
            "role_source": "canonical_classification",
            "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
            "evidence_class": assistant_evidence_label(participant.evidence_class()),
            "base_depth_mm": base_depth_mm,
            "height_mm": height_mm,
            "height_depth_ratio": height_depth_ratio,
            "minimum_height_mm": ANCHORING_MINIMUM_HEIGHT_MM,
            "minimum_height_depth_ratio": ANCHORING_MINIMUM_HEIGHT_DEPTH_RATIO,
            "anchoring_required": anchoring_required,
            "anchor_declaration": "not_available_in_current_document_schema",
            "result": if anchoring_required { "required" } else { "not_required" },
        });
        if anchoring_required {
            issues.push(serde_json::json!({
                "code": "furniture.anchor_required",
                "severity": "warning",
                "occurrence_id": occurrence_id.0,
                "name": name,
                "role": role.as_str(),
                "role_source": "canonical_classification",
                "geometry_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
                "evidence_class": assistant_evidence_label(participant.evidence_class()),
                "base_depth_mm": base_depth_mm,
                "height_mm": height_mm,
                "height_depth_ratio": height_depth_ratio,
                "minimum_height_mm": ANCHORING_MINIMUM_HEIGHT_MM,
                "minimum_height_depth_ratio": ANCHORING_MINIMUM_HEIGHT_DEPTH_RATIO,
                "anchor_declaration": "not_available_in_current_document_schema",
            }));
        }
        evaluations.push(evaluation);
    }
    let issue_count = issues.len();
    let complete = coverage_complete && not_evaluated.is_empty();
    let state = if !complete {
        "not_evaluated"
    } else if issue_count > 0 {
        "failed"
    } else {
        "passed"
    };
    serde_json::json!({
        "state": state,
        "complete": complete && issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "applicable_count": evaluations.len(),
        "required_count": issue_count,
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "limit": {
            "minimum_height_mm": ANCHORING_MINIMUM_HEIGHT_MM,
            "minimum_height_depth_ratio": ANCHORING_MINIMUM_HEIGHT_DEPTH_RATIO,
        },
        "assumptions": [
            "canonical validator roles declare the case-furniture vertical axis",
            "source-frame extents are bound to the accepted body geometry",
            "wall-anchor declarations are not represented by the current document schema",
            "a requirement is reported rather than claiming that an anchor is absent",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantManufacturingRoleKind {
    Panel,
    Hole,
    HingeCup,
    LinearHardwarePair,
}

#[derive(Clone, Copy, Debug)]
pub struct AssistantManufacturingRole<'a> {
    pub kind: AssistantManufacturingRoleKind,
    axis: usize,
    pub group: &'a str,
}

pub fn assistant_manufacturing_role(role: &str) -> Option<AssistantManufacturingRole<'_>> {
    let (role, group) = role.split_once(':')?;
    if group.is_empty() {
        return None;
    }
    let (kind, axis) = match role {
        "manufacturing.panel.xy" => (AssistantManufacturingRoleKind::Panel, 2),
        "manufacturing.panel.xz" => (AssistantManufacturingRoleKind::Panel, 1),
        "manufacturing.panel.yz" => (AssistantManufacturingRoleKind::Panel, 0),
        "manufacturing.hole.x" => (AssistantManufacturingRoleKind::Hole, 0),
        "manufacturing.hole.y" => (AssistantManufacturingRoleKind::Hole, 1),
        "manufacturing.hole.z" => (AssistantManufacturingRoleKind::Hole, 2),
        "manufacturing.hinge-cup.x" => (AssistantManufacturingRoleKind::HingeCup, 0),
        "manufacturing.hinge-cup.y" => (AssistantManufacturingRoleKind::HingeCup, 1),
        "manufacturing.hinge-cup.z" => (AssistantManufacturingRoleKind::HingeCup, 2),
        "hardware.linear-pair.x" => (AssistantManufacturingRoleKind::LinearHardwarePair, 0),
        "hardware.linear-pair.y" => (AssistantManufacturingRoleKind::LinearHardwarePair, 1),
        "hardware.linear-pair.z" => (AssistantManufacturingRoleKind::LinearHardwarePair, 2),
        _ => return None,
    };
    Some(AssistantManufacturingRole { kind, axis, group })
}

pub fn assistant_general_body_source_label(source: &GeneralBodySource) -> &'static str {
    match source {
        GeneralBodySource::Exact(_) => "accepted_exact_brep",
        GeneralBodySource::CanonicalMesh { .. } => "canonical_mesh_topology",
        GeneralBodySource::CanonicalExtrusion { .. } => "canonical_extrusion_topology",
        GeneralBodySource::CanonicalExactGraph { .. } => "canonical_exact_feature_graph",
    }
}

pub fn assistant_axis_offset_mm(point: [f64; 3], origin: [f64; 3], direction: [f64; 3]) -> f64 {
    (0..3)
        .map(|coordinate| (point[coordinate] - origin[coordinate]) * direction[coordinate])
        .sum()
}

pub fn assistant_hardware_manufacturing_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
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
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "not_evaluated": [],
                "issues": [],
            });
        }
    };

    struct Geometry<'a> {
        occurrence_id: OccurrenceId,
        name: &'a str,
        role: &'a str,
        role_kind: AssistantManufacturingRoleKind,
        axis: usize,
        group: &'a str,
        dimensions: [f64; 3],
        centre: [f64; 3],
        axes: [[f64; 3]; 3],
        evidence_class: &'static str,
        topology_source: &'static str,
    }

    let geometries = participants
        .iter()
        .filter_map(|participant| {
            let occurrence_id = participant.instance_path().root_occurrence();
            let name = names.get(&occurrence_id)?;
            let role = roles.role(occurrence_id)?;
            let parsed_role = assistant_manufacturing_role(role.as_str())?;
            let geometry = participant.geometry_evidence();
            Some(Geometry {
                occurrence_id,
                name,
                role: role.as_str(),
                role_kind: parsed_role.kind,
                axis: parsed_role.axis,
                group: parsed_role.group,
                dimensions: geometry.source_frame_extents_mm(),
                centre: geometry.source_frame_center_world_mm(),
                axes: std::array::from_fn(|axis| {
                    geometry
                        .source_axis_world_direction(axis)
                        .expect("declared source axis is bounded")
                }),
                evidence_class: assistant_evidence_label(participant.evidence_class()),
                topology_source: assistant_general_body_source_label(participant.source()),
            })
        })
        .collect::<Vec<_>>();
    let panel_indices = geometries
        .iter()
        .enumerate()
        .filter_map(|(index, geometry)| {
            (geometry.role_kind == AssistantManufacturingRoleKind::Panel).then_some(index)
        })
        .collect::<Vec<_>>();
    let mut evaluations = Vec::new();
    let mut issues = Vec::new();
    let mut not_evaluated = Vec::new();

    for &panel_index in &panel_indices {
        let panel = &geometries[panel_index];
        let thickness_mm = panel.dimensions[panel.axis];
        let failed = thickness_mm < MINIMUM_PANEL_THICKNESS_MM;
        evaluations.push(serde_json::json!({
            "rule": "panel_minimum_thickness",
            "occurrence_id": panel.occurrence_id.0,
            "name": panel.name,
            "role": panel.role,
            "role_source": "canonical_classification",
            "topology_source": panel.topology_source,
            "evidence_class": panel.evidence_class,
            "thickness_axis": panel.axis,
            "thickness_mm": thickness_mm,
            "minimum_thickness_mm": MINIMUM_PANEL_THICKNESS_MM,
            "result": if failed { "failed" } else { "passed" },
        }));
        if failed {
            issues.push(serde_json::json!({
                "code": "manufacturing.panel_below_minimum_thickness",
                "severity": "warning",
                "occurrence_id": panel.occurrence_id.0,
                "name": panel.name,
                "role": panel.role,
                "role_source": "canonical_classification",
                "topology_source": panel.topology_source,
                "evidence_class": panel.evidence_class,
                "thickness_axis": panel.axis,
                "thickness_mm": thickness_mm,
                "minimum_thickness_mm": MINIMUM_PANEL_THICKNESS_MM,
                "rule": "panel stock thickness must be at least 6 mm",
            }));
        }
    }

    let mut holes = Vec::new();
    for (hole_index, hole) in geometries.iter().enumerate().filter(|(_, geometry)| {
        matches!(
            geometry.role_kind,
            AssistantManufacturingRoleKind::Hole | AssistantManufacturingRoleKind::HingeCup
        )
    }) {
        let host_indices = panel_indices
            .iter()
            .copied()
            .filter(|panel_index| geometries[*panel_index].group == hole.group)
            .collect::<Vec<_>>();
        let [host_index] = host_indices.as_slice() else {
            not_evaluated.push(serde_json::json!({
                "validator": "hardware_manufacturing",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "role": hole.role,
                "association": hole.group,
                "reason": "explicit_panel_association_is_missing_or_ambiguous",
            }));
            evaluations.push(serde_json::json!({
                "rule": "hole_edge_and_spacing",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "role": hole.role,
                "result": "not_evaluated",
                "reason": "explicit_panel_association_is_missing_or_ambiguous",
            }));
            continue;
        };
        let host_index = *host_index;
        let host = &geometries[host_index];
        let axis = hole.axis;
        if host.axis != axis {
            not_evaluated.push(serde_json::json!({
                "validator": "hardware_manufacturing",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "role": hole.role,
                "host_role": host.role,
                "reason": "declared_hole_axis_does_not_match_panel_normal",
            }));
            continue;
        }
        if (0..3).any(|candidate| {
            let alignment = (0..3)
                .map(|coordinate| {
                    hole.axes[candidate][coordinate] * host.axes[candidate][coordinate]
                })
                .sum::<f64>()
                .abs();
            alignment < 1.0 - 1.0e-9
        }) {
            not_evaluated.push(serde_json::json!({
                "validator": "hardware_manufacturing",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "role": hole.role,
                "host_role": host.role,
                "reason": "hole_and_panel_source_frames_are_not_axis_aligned",
            }));
            continue;
        }
        let radial_axes = (0..3)
            .filter(|candidate| *candidate != axis)
            .collect::<Vec<_>>();
        let diameter_mm = radial_axes
            .iter()
            .map(|radial_axis| hole.dimensions[*radial_axis])
            .max_by(f64::total_cmp)
            .expect("a hole has two radial dimensions");
        let radius_mm = diameter_mm / 2.0;
        let depth_mm = hole.dimensions[axis];
        let edge_material_mm = radial_axes
            .iter()
            .map(|radial_axis| {
                host.dimensions[*radial_axis] / 2.0
                    - assistant_axis_offset_mm(hole.centre, host.centre, host.axes[*radial_axis])
                        .abs()
                    - radius_mm
            })
            .min_by(f64::total_cmp)
            .expect("a hole has two radial edge distances");
        let edge_failed = edge_material_mm < MINIMUM_HOLE_EDGE_MATERIAL_MM;
        evaluations.push(serde_json::json!({
            "rule": "hole_edge_distance",
            "occurrence_id": hole.occurrence_id.0,
            "name": hole.name,
            "role": hole.role,
            "role_source": "canonical_classification",
            "host_occurrence_id": host.occurrence_id.0,
            "host_name": host.name,
            "host_role": host.role,
            "association": hole.group,
            "topology_source": hole.topology_source,
            "evidence_class": hole.evidence_class,
            "diameter_mm": diameter_mm,
            "depth_mm": depth_mm,
            "edge_material_mm": edge_material_mm,
            "minimum_edge_material_mm": MINIMUM_HOLE_EDGE_MATERIAL_MM,
            "result": if edge_failed { "failed" } else { "passed" },
        }));
        if edge_failed {
            issues.push(serde_json::json!({
                "code": "manufacturing.hole_too_close_to_edge",
                "severity": "warning",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "host_occurrence_id": host.occurrence_id.0,
                "host_name": host.name,
                "evidence_class": hole.evidence_class,
                "edge_material_mm": edge_material_mm,
                "minimum_edge_material_mm": MINIMUM_HOLE_EDGE_MATERIAL_MM,
                "rule": "hole perimeter must leave at least 5 mm of material to every panel edge",
            }));
        }
        if hole.role_kind == AssistantManufacturingRoleKind::HingeCup {
            let hinge_failed = diameter_mm < MINIMUM_HINGE_CUP_DIAMETER_MM
                || depth_mm < MINIMUM_HINGE_CUP_DEPTH_MM;
            evaluations.push(serde_json::json!({
                "rule": "hinge_cup_envelope",
                "occurrence_id": hole.occurrence_id.0,
                "name": hole.name,
                "host_occurrence_id": host.occurrence_id.0,
                "host_name": host.name,
                "diameter_mm": diameter_mm,
                "depth_mm": depth_mm,
                "minimum_diameter_mm": MINIMUM_HINGE_CUP_DIAMETER_MM,
                "minimum_depth_mm": MINIMUM_HINGE_CUP_DEPTH_MM,
                "result": if hinge_failed { "failed" } else { "passed" },
            }));
            if hinge_failed {
                issues.push(serde_json::json!({
                    "code": "manufacturing.hinge_cup_envelope_below_minimum",
                    "severity": "warning",
                    "occurrence_id": hole.occurrence_id.0,
                    "name": hole.name,
                    "host_occurrence_id": host.occurrence_id.0,
                    "host_name": host.name,
                    "diameter_mm": diameter_mm,
                    "depth_mm": depth_mm,
                    "minimum_diameter_mm": MINIMUM_HINGE_CUP_DIAMETER_MM,
                    "minimum_depth_mm": MINIMUM_HINGE_CUP_DEPTH_MM,
                    "rule": "named hinge cup requires a 35 mm diameter and 12 mm depth envelope",
                }));
            }
        }
        holes.push((hole_index, host_index, axis, radius_mm));
    }

    for left_index in 0..holes.len() {
        for right in &holes[left_index + 1..] {
            let left = holes[left_index];
            if left.1 != right.1 || left.2 != right.2 {
                continue;
            }
            let left_hole = &geometries[left.0];
            let right_hole = &geometries[right.0];
            let host = &geometries[left.1];
            let radial_distance_mm = (0..3)
                .filter(|axis| *axis != left.2)
                .map(|axis| {
                    assistant_axis_offset_mm(right_hole.centre, left_hole.centre, host.axes[axis])
                        .powi(2)
                })
                .sum::<f64>()
                .sqrt();
            let material_between_mm = radial_distance_mm - left.3 - right.3;
            if material_between_mm < MINIMUM_HOLE_SPACING_MATERIAL_MM {
                issues.push(serde_json::json!({
                    "code": "manufacturing.hole_spacing_below_minimum",
                    "severity": "warning",
                    "left_occurrence_id": left_hole.occurrence_id.0,
                    "left_name": left_hole.name,
                    "right_occurrence_id": right_hole.occurrence_id.0,
                    "right_name": right_hole.name,
                    "host_occurrence_id": host.occurrence_id.0,
                    "host_name": host.name,
                    "material_between_mm": material_between_mm,
                    "minimum_material_between_mm": MINIMUM_HOLE_SPACING_MATERIAL_MM,
                    "rule": "hole perimeters must leave at least 3 mm of material between them",
                }));
            }
        }
    }

    let mut linear_pairs: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for geometry in geometries
        .iter()
        .filter(|geometry| geometry.role_kind == AssistantManufacturingRoleKind::LinearHardwarePair)
    {
        linear_pairs
            .entry(geometry.group)
            .or_default()
            .push(geometry);
    }
    for (group, pair) in linear_pairs {
        let [left, right] = pair.as_slice() else {
            issues.push(serde_json::json!({
                "code": "hardware.linear_pair_incomplete",
                "severity": "warning",
                "association": group,
                "occurrence_ids": pair.iter().map(|member| member.occurrence_id.0).collect::<Vec<_>>(),
                "names": pair.iter().map(|member| member.name).collect::<Vec<_>>(),
                "declared_member_count": pair.len(),
                "required_member_count": 2,
                "rule": "a declared linear-hardware pair requires exactly two members",
            }));
            continue;
        };
        if left.axis != right.axis {
            not_evaluated.push(serde_json::json!({
                "validator": "hardware_manufacturing",
                "association": group,
                "occurrence_ids": [left.occurrence_id.0, right.occurrence_id.0],
                "reason": "linear_pair_axes_do_not_match",
            }));
            continue;
        }
        let length_mismatch_mm = (left.dimensions[left.axis] - right.dimensions[right.axis]).abs();
        let vertical_mismatch_mm = (left.centre[2] - right.centre[2]).abs();
        let failed = length_mismatch_mm > MAXIMUM_DRAWER_SLIDE_LENGTH_MISMATCH_MM
            || vertical_mismatch_mm > MAXIMUM_DRAWER_SLIDE_VERTICAL_MISMATCH_MM;
        evaluations.push(serde_json::json!({
            "rule": "linear_hardware_pair_alignment",
            "association": group,
            "left_occurrence_id": left.occurrence_id.0,
            "left_name": left.name,
            "left_role": left.role,
            "right_occurrence_id": right.occurrence_id.0,
            "right_name": right.name,
            "right_role": right.role,
            "topology_sources": [left.topology_source, right.topology_source],
            "length_axis": left.axis,
            "length_mismatch_mm": length_mismatch_mm,
            "vertical_mismatch_mm": vertical_mismatch_mm,
            "maximum_length_mismatch_mm": MAXIMUM_DRAWER_SLIDE_LENGTH_MISMATCH_MM,
            "maximum_vertical_mismatch_mm": MAXIMUM_DRAWER_SLIDE_VERTICAL_MISMATCH_MM,
            "result": if failed { "failed" } else { "passed" },
        }));
        if failed {
            issues.push(serde_json::json!({
                "code": "hardware.linear_pair_misaligned",
                "severity": "warning",
                "association": group,
                "left_occurrence_id": left.occurrence_id.0,
                "left_name": left.name,
                "right_occurrence_id": right.occurrence_id.0,
                "right_name": right.name,
                "length_mismatch_mm": length_mismatch_mm,
                "vertical_mismatch_mm": vertical_mismatch_mm,
                "maximum_length_mismatch_mm": MAXIMUM_DRAWER_SLIDE_LENGTH_MISMATCH_MM,
                "maximum_vertical_mismatch_mm": MAXIMUM_DRAWER_SLIDE_VERTICAL_MISMATCH_MM,
                "rule": "a linear-hardware pair must have equal declared-axis length and world-Z alignment within 1 mm",
            }));
        }
    }

    let issue_count = issues.len();
    let applicable_count = evaluations.len();
    let state = if issue_count > 0 {
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
        "applicable_count": applicable_count,
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "limits": {
            "minimum_hole_edge_material_mm": MINIMUM_HOLE_EDGE_MATERIAL_MM,
            "minimum_hole_spacing_material_mm": MINIMUM_HOLE_SPACING_MATERIAL_MM,
            "minimum_hinge_cup_diameter_mm": MINIMUM_HINGE_CUP_DIAMETER_MM,
            "minimum_hinge_cup_depth_mm": MINIMUM_HINGE_CUP_DEPTH_MM,
            "maximum_drawer_slide_length_mismatch_mm": MAXIMUM_DRAWER_SLIDE_LENGTH_MISMATCH_MM,
            "maximum_drawer_slide_vertical_mismatch_mm": MAXIMUM_DRAWER_SLIDE_VERTICAL_MISMATCH_MM,
            "minimum_panel_thickness_mm": MINIMUM_PANEL_THICKNESS_MM,
        },
        "assumptions": [
            "canonical validator roles declare panels, holes, hinge cups, linear-hardware pairs, source axes, and association groups",
            "a hole or hinge-cup association group must resolve to exactly one explicitly declared host panel",
            "panel thickness and hole depth use declared source-frame axes bound to accepted topology",
            "hole radial envelopes are conservatively treated as circular using their largest source-frame radial extent",
            "each linear-hardware association group must contain exactly two members",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated,
    })
}

pub fn assistant_room_placement_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
    tolerance: TolerancePolicy,
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
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "issues": [],
                "not_evaluated": [],
            });
        }
    };

    struct Geometry<'a> {
        occurrence_id: OccurrenceId,
        name: &'a str,
        participant: &'a GeneralBodyParticipant,
        kind: AssistantSpatialRoleKind,
        group: &'a str,
    }

    let geometries = participants
        .iter()
        .filter_map(|participant| {
            let occurrence_id = participant.instance_path().root_occurrence();
            let name = names.get(&occurrence_id)?;
            let role = roles
                .role(occurrence_id)
                .and_then(|role| assistant_spatial_role(role.as_str()))?;
            Some(Geometry {
                occurrence_id,
                name,
                participant,
                kind: role.kind,
                group: role.group,
            })
        })
        .collect::<Vec<_>>();
    let rooms = geometries
        .iter()
        .filter(|geometry| geometry.kind == AssistantSpatialRoleKind::Room)
        .collect::<Vec<_>>();
    if rooms.is_empty() {
        return serde_json::json!({
            "state": "not_evaluated",
            "complete": false,
            "applicable_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "limits": { "boundary_tolerance_mm": ROOM_PLACEMENT_TOLERANCE_MM },
            "evaluations": [],
            "issues": [],
            "not_evaluated": [{
                "validator": "room_placement",
                "reason": "spatial_room_role_not_found",
                "required_role": "spatial.room:<group>",
            }],
        });
    }

    let mut evaluations = Vec::new();
    let mut issues = Vec::new();
    let mut not_evaluated = Vec::new();
    for furniture in geometries
        .iter()
        .filter(|geometry| geometry.kind == AssistantSpatialRoleKind::Furniture)
    {
        let matching_rooms = rooms
            .iter()
            .copied()
            .filter(|room| room.group == furniture.group)
            .collect::<Vec<_>>();
        let [room] = matching_rooms.as_slice() else {
            not_evaluated.push(serde_json::json!({
                "validator": "room_placement",
                "occurrence_id": furniture.occurrence_id.0,
                "role": format!("spatial.furniture:{}", furniture.group),
                "reason": if matching_rooms.is_empty() {
                    "associated_room_role_not_found"
                } else {
                    "associated_room_role_ambiguous"
                },
            }));
            continue;
        };
        let Ok(containment) =
            general_body_containment(room.participant, furniture.participant, tolerance)
        else {
            not_evaluated.push(serde_json::json!({
                "validator": "room_placement",
                "occurrence_id": furniture.occurrence_id.0,
                "reason": "oriented_geometry_evidence_unavailable",
            }));
            continue;
        };
        let clearances_mm = containment.clearances_mm;
        let outside_by_mm = clearances_mm.map(|clearance| (-clearance).max(0.0));
        let maximum_outside_mm = outside_by_mm
            .into_iter()
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        let failed = maximum_outside_mm > ROOM_PLACEMENT_TOLERANCE_MM;
        evaluations.push(serde_json::json!({
            "rule": "furniture_inside_associated_room",
            "occurrence_id": furniture.occurrence_id.0,
            "name": furniture.name,
            "role": format!("spatial.furniture:{}", furniture.group),
            "room_occurrence_id": room.occurrence_id.0,
            "room_name": room.name,
            "room_role": format!("spatial.room:{}", room.group),
            "evidence_class": assistant_evidence_label(&containment.evidence_class),
            "narrow_phase_method": containment.method,
            "clearances_mm": {
                "left": clearances_mm[0],
                "right": clearances_mm[1],
                "front": clearances_mm[2],
                "back": clearances_mm[3],
                "floor": clearances_mm[4],
                "ceiling": clearances_mm[5],
            },
            "maximum_outside_mm": maximum_outside_mm,
            "boundary_tolerance_mm": ROOM_PLACEMENT_TOLERANCE_MM,
            "result": if failed { "failed" } else { "passed" },
        }));
        if failed {
            issues.push(serde_json::json!({
                "code": "room.furniture_outside_boundary",
                "severity": "warning",
                "occurrence_id": furniture.occurrence_id.0,
                "name": furniture.name,
                "room_occurrence_id": room.occurrence_id.0,
                "room_name": room.name,
                "role": format!("spatial.furniture:{}", furniture.group),
                "room_role": format!("spatial.room:{}", room.group),
                "evidence_class": assistant_evidence_label(&containment.evidence_class),
                "narrow_phase_method": containment.method,
                "outside_by_mm": {
                    "left": outside_by_mm[0],
                    "right": outside_by_mm[1],
                    "front": outside_by_mm[2],
                    "back": outside_by_mm[3],
                    "floor": outside_by_mm[4],
                    "ceiling": outside_by_mm[5],
                },
                "maximum_outside_mm": maximum_outside_mm,
                "boundary_tolerance_mm": ROOM_PLACEMENT_TOLERANCE_MM,
                "rule": "a spatial.furniture occurrence must stay inside its associated spatial.room envelope",
            }));
        }
    }
    let issue_count = issues.len();
    let state = if issue_count > 0 {
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
        "applicable_count": evaluations.len(),
        "room_count": rooms.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "limits": { "boundary_tolerance_mm": ROOM_PLACEMENT_TOLERANCE_MM },
        "assumptions": [
            "canonical spatial roles explicitly associate furniture with exactly one room group",
            "containment uses current revision-bound oriented source-frame geometry",
            "non-exact body envelopes retain tolerant false-positive-only evidence",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated,
    })
}

pub fn assistant_passage_clearance_report(
    participants: &[GeneralBodyParticipant],
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
    tolerance: TolerancePolicy,
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
    let roles = match roles {
        Ok(roles) => roles,
        Err(error) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "role_error": error.to_string(),
                "evaluations": [],
                "issues": [],
                "not_evaluated": [],
            });
        }
    };

    struct Geometry<'a> {
        occurrence_id: OccurrenceId,
        name: &'a str,
        participant: &'a GeneralBodyParticipant,
        kind: AssistantSpatialRoleKind,
        group: &'a str,
    }

    let geometries = participants
        .iter()
        .filter_map(|participant| {
            let occurrence_id = participant.instance_path().root_occurrence();
            let name = names.get(&occurrence_id)?;
            let role = roles
                .role(occurrence_id)
                .and_then(|role| assistant_spatial_role(role.as_str()))?;
            Some(Geometry {
                occurrence_id,
                name,
                participant,
                kind: role.kind,
                group: role.group,
            })
        })
        .collect::<Vec<_>>();
    let passages = geometries
        .iter()
        .filter(|geometry| matches!(geometry.kind, AssistantSpatialRoleKind::Passage { .. }))
        .collect::<Vec<_>>();
    if passages.is_empty() {
        return serde_json::json!({
            "state": "not_evaluated",
            "complete": false,
            "applicable_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "limits": {
                "minimum_width_mm": MINIMUM_PASSAGE_WIDTH_MM,
                "minimum_headroom_mm": MINIMUM_PASSAGE_HEADROOM_MM,
            },
            "evaluations": [],
            "issues": [],
            "not_evaluated": [{
                "validator": "passage_clearance",
                "reason": "spatial_passage_role_not_found",
                "required_role": "spatial.passage.{xy|xz|yz}:<group>",
            }],
        });
    }

    let mut evaluations = Vec::new();
    let mut issues = Vec::new();
    let mut not_evaluated = Vec::new();
    for passage in &passages {
        let AssistantSpatialRoleKind::Passage {
            surface_axes,
            height_axis,
        } = passage.kind
        else {
            unreachable!("passages were filtered by canonical role");
        };
        let geometry = passage.participant.geometry_evidence();
        let dimensions_mm: [f64; 3] = std::array::from_fn(|axis| {
            geometry.source_frame_extents_mm()[axis]
                * geometry
                    .source_axis_world_scale(axis)
                    .expect("three source-axis scales are always present")
        });
        let width_mm = dimensions_mm[surface_axes[0]].min(dimensions_mm[surface_axes[1]]);
        let headroom_mm = dimensions_mm[height_axis];
        let envelope_failed =
            width_mm < MINIMUM_PASSAGE_WIDTH_MM || headroom_mm < MINIMUM_PASSAGE_HEADROOM_MM;
        evaluations.push(serde_json::json!({
            "rule": "minimum_passage_envelope",
            "occurrence_id": passage.occurrence_id.0,
            "name": passage.name,
            "role": format!("spatial.passage.{}:{}", match height_axis {
                2 => "xy",
                1 => "xz",
                0 => "yz",
                _ => unreachable!("source geometry has three axes"),
            }, passage.group),
            "role_source": "canonical_classification",
            "evidence_class": assistant_evidence_label(passage.participant.evidence_class()),
            "source_frame_method": GENERAL_BODY_SOURCE_FRAME_METHOD_V1,
            "width_mm": width_mm,
            "headroom_mm": headroom_mm,
            "minimum_width_mm": MINIMUM_PASSAGE_WIDTH_MM,
            "minimum_headroom_mm": MINIMUM_PASSAGE_HEADROOM_MM,
            "result": if envelope_failed { "failed" } else { "passed" },
        }));
        if envelope_failed {
            issues.push(serde_json::json!({
                "code": "room.passage_envelope_below_minimum",
                "severity": "warning",
                "occurrence_id": passage.occurrence_id.0,
                "name": passage.name,
                "width_mm": width_mm,
                "headroom_mm": headroom_mm,
                "minimum_width_mm": MINIMUM_PASSAGE_WIDTH_MM,
                "minimum_headroom_mm": MINIMUM_PASSAGE_HEADROOM_MM,
                "rule": "a spatial.passage envelope must be at least 900 mm wide and 2000 mm high",
            }));
        }
        for obstacle in geometries.iter().filter(|geometry| {
            geometry.occurrence_id != passage.occurrence_id
                && geometry.group == passage.group
                && matches!(
                    geometry.kind,
                    AssistantSpatialRoleKind::Furniture | AssistantSpatialRoleKind::Obstacle
                )
        }) {
            let Ok(narrow_phase) =
                general_body_narrow_phase(passage.participant, obstacle.participant, tolerance)
            else {
                not_evaluated.push(serde_json::json!({
                    "validator": "passage_clearance",
                    "passage_occurrence_id": passage.occurrence_id.0,
                    "obstacle_occurrence_id": obstacle.occurrence_id.0,
                    "reason": "oriented_geometry_evidence_unavailable",
                }));
                continue;
            };
            if narrow_phase.relation == GeneralBodyNarrowPhaseRelation::Intersecting {
                issues.push(serde_json::json!({
                    "code": "room.passage_blocked",
                    "severity": "warning",
                    "passage_occurrence_id": passage.occurrence_id.0,
                    "passage_name": passage.name,
                    "obstacle_occurrence_id": obstacle.occurrence_id.0,
                    "obstacle_name": obstacle.name,
                    "obstacle_role": format!("spatial.{}:{}", match obstacle.kind {
                        AssistantSpatialRoleKind::Furniture => "furniture",
                        AssistantSpatialRoleKind::Obstacle => "obstacle",
                        _ => unreachable!("obstacles were filtered by canonical role"),
                    }, obstacle.group),
                    "evidence_class": assistant_evidence_label(&narrow_phase.evidence_class),
                    "narrow_phase_method": narrow_phase.method,
                    "narrow_phase_relation": "intersecting",
                    "minimum_penetration_mm": (-narrow_phase.signed_separation_mm).max(0.0),
                    "rule": "a spatial.passage envelope must not intersect an associated spatial obstacle",
                }));
            }
        }
    }
    let issue_count = issues.len();
    let state = if issue_count > 0 {
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
        "applicable_count": passages.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "limits": {
            "minimum_width_mm": MINIMUM_PASSAGE_WIDTH_MM,
            "minimum_headroom_mm": MINIMUM_PASSAGE_HEADROOM_MM,
        },
        "assumptions": [
            "canonical spatial roles explicitly declare passages, obstacles, and association groups",
            "passage width and headroom use explicit source-frame role axes",
            "blocked-passage authority comes from exact/tolerant OBB-SAT narrow-phase geometry",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated,
    })
}

/// Support group used when the roles are read from the document itself.
const DERIVED_GRAVITY_SUPPORT_GROUP: &str = "document";
/// Standard gravity, used when the document declares no gravity vector.
const CANONICAL_GRAVITY_M_S2: [f64; 3] = [0.0, 0.0, -9.81];
const DERIVED_GRAVITY_ROLE_ASSUMPTION: &str = "gravity roles were read from the document: every visible solid is a body, and only the occurrences the document grounds seed support";
const DERIVED_GRAVITY_VECTOR_ASSUMPTION: &str =
    "no gravity vector is declared, so standard gravity 9.81 m/s² along -Z was assumed";

/// Gravity participants read straight from what the document already states.
///
/// No floor is invented here. Support still has to be earned by real contact
/// with an occurrence the document explicitly grounds; this only spares the
/// operator from restating, as classification roles, two facts the document
/// already holds — which solids are visible, and which of them are grounded.
/// Without a single grounded occurrence there is no seed, so nothing is
/// derived and the validator stays honestly unevaluated.
pub fn assistant_derived_gravity_participants(
    snapshot: &Snapshot,
    participants: &[GeneralBodyParticipant],
) -> Vec<GravitySupportParticipant> {
    if snapshot.grounded_occurrences().next().is_none() {
        return Vec::new();
    }
    participants
        .iter()
        .map(|body| {
            let occurrence_id = body.instance_path().root_occurrence();
            GravitySupportParticipant::new(
                body.clone(),
                DERIVED_GRAVITY_SUPPORT_GROUP,
                snapshot.occurrence_is_grounded(occurrence_id),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct AssistantGravityInput {
    pub node_ids: [u64; 3],
    pub vector_m_s2: [f64; 3],
    pub direction: [f64; 3],
    pub magnitude_m_s2: f64,
}

pub fn assistant_gravity_input(snapshot: &Snapshot) -> Result<AssistantGravityInput, &'static str> {
    let mut components = [Vec::<(u64, f64)>::new(), Vec::new(), Vec::new()];
    for node in snapshot.evaluator_nodes() {
        let Some(value) = node.dimension().map(|dimension| dimension.millimetres()) else {
            continue;
        };
        let axis = match node.name() {
            "physics.gravity_x_m_s2" => Some(0),
            "physics.gravity_y_m_s2" => Some(1),
            "physics.gravity_z_m_s2" => Some(2),
            _ => None,
        };
        if let Some(axis) = axis {
            components[axis].push((node.id().0, value));
        }
    }
    if components.iter().any(|values| values.len() != 1) {
        return Err("missing_or_ambiguous_gravity_vector");
    }
    let node_ids = std::array::from_fn(|axis| components[axis][0].0);
    let vector_m_s2 = std::array::from_fn(|axis| components[axis][0].1);
    let magnitude_m_s2 = vector_m_s2
        .into_iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if magnitude_m_s2 <= f64::EPSILON {
        return Err("zero_gravity_vector");
    }
    Ok(AssistantGravityInput {
        node_ids,
        vector_m_s2,
        direction: vector_m_s2.map(|value| value / magnitude_m_s2),
        magnitude_m_s2,
    })
}

pub fn assistant_static_load_report(
    snapshot: &Snapshot,
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
    selected: bool,
    coverage_complete: bool,
) -> serde_json::Value {
    assistant_static_load_report_filtered(
        snapshot,
        names,
        roles,
        selected,
        coverage_complete,
        None,
        &mut || false,
    )
}

fn assistant_static_load_report_filtered(
    snapshot: &Snapshot,
    names: &BTreeMap<OccurrenceId, String>,
    roles: &Result<ValidatorRoleIndex, ValidatorRoleError>,
    selected: bool,
    coverage_complete: bool,
    load_filter: Option<&BTreeSet<OccurrenceId>>,
    cancellation_requested: &mut dyn FnMut() -> bool,
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

    let Ok(roles) = roles else {
        return serde_json::json!({
            "state": "not_evaluated",
            "complete": false,
            "applicable_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "evaluations": [],
            "issues": [],
            "not_evaluated": [{
                "validator": "static_load",
                "reason": "missing_or_ambiguous_canonical_roles",
                "role_error": roles.as_ref().unwrap_err().to_string(),
            }],
        });
    };

    let occurrence_parameter = |name: &str, prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|suffix| suffix.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .map(OccurrenceId)
    };
    let mut masses = BTreeMap::<OccurrenceId, Vec<(u64, f64)>>::new();
    let mut applied_loads = BTreeMap::<OccurrenceId, Vec<(u64, f64)>>::new();
    let mut capacities = BTreeMap::<OccurrenceId, Vec<(u64, f64)>>::new();

    for (parameter_index, node) in snapshot.evaluator_nodes().enumerate() {
        if load_filter.is_some() && parameter_index >= MAX_STRUCTURAL_SCOPE_PARAMETERS {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "evaluations": [],
                "issues": [],
                "not_evaluated": [{
                    "validator": "static_load",
                    "reason": "structural_parameter_resource_limit",
                }],
            });
        }
        if cancellation_requested() {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "evaluations": [],
                "issues": [],
                "not_evaluated": [{
                    "validator": "static_load",
                    "reason": "validation_cancelled",
                }],
            });
        }
        let Some(value) = node.dimension().map(|dimension| dimension.millimetres()) else {
            continue;
        };
        let name = node.name();
        if let Some(occurrence_id) = occurrence_parameter(name, "physics.mass_kg.occurrence.") {
            masses
                .entry(occurrence_id)
                .or_default()
                .push((node.id().0, value));
        } else if let Some(occurrence_id) =
            occurrence_parameter(name, "physics.applied_load_n.occurrence.")
        {
            applied_loads
                .entry(occurrence_id)
                .or_default()
                .push((node.id().0, value));
        } else if let Some(occurrence_id) =
            occurrence_parameter(name, "physics.support_capacity_n.occurrence.")
        {
            capacities
                .entry(occurrence_id)
                .or_default()
                .push((node.id().0, value));
        }
    }

    let gravity = match assistant_gravity_input(snapshot) {
        Ok(gravity) => gravity,
        Err(reason) => {
            return serde_json::json!({
                "state": "not_evaluated",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "evaluations": [],
                "issues": [],
                "not_evaluated": [{
                    "validator": "static_load",
                    "reason": reason,
                }],
            });
        }
    };

    let mut evaluations = Vec::new();
    let mut issues = Vec::new();
    let mut not_evaluated = Vec::new();
    let loaded_ids = load_filter.map_or_else(
        || masses.keys().copied().collect::<Vec<_>>(),
        |ids| ids.iter().copied().collect::<Vec<_>>(),
    );
    if loaded_ids.is_empty() {
        not_evaluated.push(serde_json::json!({
            "validator": "static_load",
            "reason": "explicit_load_case_not_found",
            "required_input": "physics.mass_kg.occurrence.<id>",
        }));
    }
    'loads: for loaded_id in loaded_ids {
        if cancellation_requested() {
            not_evaluated.push(serde_json::json!({
                "validator": "static_load",
                "reason": "validation_cancelled",
            }));
            break;
        }
        let loaded_name = names.get(&loaded_id);
        let mass_declarations = masses.get(&loaded_id);
        let load_declarations = applied_loads.get(&loaded_id);
        let loaded_role = roles
            .role(loaded_id)
            .and_then(|role| assistant_physics_role(role.as_str()));
        let mut support_ids = Vec::new();
        if let Some(loaded_role) =
            loaded_role.filter(|role| role.kind == AssistantPhysicsRoleKind::StaticLoad)
        {
            for assignment in roles.assignments() {
                if cancellation_requested() {
                    not_evaluated.push(serde_json::json!({
                        "validator": "static_load",
                        "reason": "validation_cancelled",
                    }));
                    break 'loads;
                }
                if let Some(role) = assistant_physics_role(assignment.role.as_str())
                    && role.kind == AssistantPhysicsRoleKind::StaticSupport
                    && role.group == loaded_role.group
                {
                    support_ids.push(assignment.occurrence_id);
                }
            }
        }
        let missing_reason = if loaded_name.is_none() {
            Some("loaded_occurrence_not_visible")
        } else if loaded_role.is_none_or(|role| role.kind != AssistantPhysicsRoleKind::StaticLoad) {
            Some("missing_or_invalid_static_load_role")
        } else if mass_declarations.is_none_or(|values| values.len() != 1) {
            Some("missing_or_ambiguous_mass")
        } else if mass_declarations.unwrap()[0].1 < 0.0 {
            Some("negative_mass")
        } else if load_declarations.is_none_or(|values| values.len() != 1) {
            Some("missing_or_ambiguous_applied_load")
        } else if load_declarations.unwrap()[0].1 < 0.0 {
            Some("negative_applied_load")
        } else if support_ids.is_empty() {
            Some("static_support_role_not_found")
        } else if support_ids.iter().any(|support_id| {
            !names.contains_key(support_id)
                || capacities
                    .get(support_id)
                    .is_none_or(|values| values.len() != 1 || values[0].1 < 0.0)
        }) {
            Some("missing_invalid_or_ambiguous_support_capacity")
        } else {
            None
        };
        if let Some(reason) = missing_reason {
            not_evaluated.push(serde_json::json!({
                "validator": "static_load",
                "reason": reason,
                "occurrence_id": loaded_id.0,
                "name": loaded_name,
            }));
            continue;
        }

        let (mass_node_id, mass_kg) = mass_declarations.unwrap()[0];
        let (applied_load_node_id, applied_load_n) = load_declarations.unwrap()[0];
        let support_inputs = support_ids
            .iter()
            .map(|support_id| {
                let (capacity_node_id, capacity_n) = capacities[support_id][0];
                serde_json::json!({
                    "occurrence_id": support_id.0,
                    "name": names.get(support_id),
                    "role_case": loaded_role.unwrap().group,
                    "capacity_node_id": capacity_node_id,
                    "capacity_n": capacity_n,
                })
            })
            .collect::<Vec<_>>();
        let total_support_capacity_n = support_inputs
            .iter()
            .filter_map(|support| support["capacity_n"].as_f64())
            .sum::<f64>();
        let weight_force_n = mass_kg * gravity.magnitude_m_s2;
        let resultant_force_n = weight_force_n + applied_load_n;
        let capacity_margin_n = total_support_capacity_n - resultant_force_n;
        let failed = capacity_margin_n < 0.0;
        let evaluation = serde_json::json!({
            "occurrence_id": loaded_id.0,
            "name": loaded_name,
            "input_source": "canonical_evaluator_parameters",
            "mass": { "node_id": mass_node_id, "value_kg": mass_kg },
            "applied_load": { "node_id": applied_load_node_id, "value_n": applied_load_n },
            "gravity": {
                "node_ids": gravity.node_ids,
                "vector_m_s2": gravity.vector_m_s2,
                "direction": gravity.direction,
                "magnitude_m_s2": gravity.magnitude_m_s2,
            },
            "supports": support_inputs,
            "weight_force_n": weight_force_n,
            "resultant_force_n": resultant_force_n,
            "total_support_capacity_n": total_support_capacity_n,
            "capacity_margin_n": capacity_margin_n,
            "calculation": "resultant_force_n = mass_kg * |gravity_m_s2| + applied_load_n",
            "result": if failed { "failed" } else { "passed" },
        });
        if failed {
            issues.push(serde_json::json!({
                "code": "physics.support_capacity_exceeded",
                "severity": "error",
                "occurrence_id": loaded_id.0,
                "name": loaded_name,
                "resultant_force_n": resultant_force_n,
                "total_support_capacity_n": total_support_capacity_n,
                "capacity_shortfall_n": -capacity_margin_n,
                "support_occurrence_ids": support_ids.iter().map(|support_id| support_id.0).collect::<Vec<_>>(),
                "rule": "the sum of explicitly declared support capacities must cover the explicitly calculated static load",
            }));
        }
        evaluations.push(evaluation);
    }

    if !coverage_complete {
        not_evaluated.push(serde_json::json!({
            "validator": "static_load",
            "reason": "incomplete_occurrence_identity_coverage",
        }));
    }
    let issue_count = issues.len();
    let state = if issue_count > 0 {
        "failed"
    } else if !not_evaluated.is_empty() {
        "not_evaluated"
    } else {
        "passed"
    };
    serde_json::json!({
        "state": state,
        "complete": not_evaluated.is_empty() && issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "applicable_count": evaluations.len(),
        "issue_count": issue_count,
        "issues_complete": issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
        "input_contract": {
            "source": "canonical evaluator parameter nodes",
            "mass": "physics.mass_kg.occurrence.<id>",
            "applied_load": "physics.applied_load_n.occurrence.<id>",
            "gravity": ["physics.gravity_x_m_s2", "physics.gravity_y_m_s2", "physics.gravity_z_m_s2"],
            "roles": ["physics.static.load:<case>", "physics.static.support:<case>"],
            "support_capacity": "physics.support_capacity_n.occurrence.<support_id>",
        },
        "assumptions": [
            "all physical quantities are explicit canonical evaluator parameters",
            "applied_load_n acts in the declared gravity direction",
            "support capacities are additive for explicitly linked supports",
            "no mass, load, gravity, support, or capacity is inferred from occurrence names or geometry",
        ],
        "evaluations": evaluations.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "issues": issues.into_iter().take(MAX_ASSISTANT_VALIDATION_ISSUES).collect::<Vec<_>>(),
        "not_evaluated": not_evaluated,
    })
}

pub use crate::collision::{
    CollisionScope, assistant_validation_context, assistant_validation_context_with_worker,
    scoped_collision_report_with_worker,
};

pub(crate) fn assistant_validation_context_base(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
    collision: serde_json::Value,
) -> serde_json::Value {
    let tolerance = TolerancePolicy::default();
    let needs_participant_projection = selection.requested.iter().any(|id| *id != "collision");
    let (visible_occurrences, scene_query_error) = if needs_participant_projection {
        match snapshot.scene_query_bounded(
            MAX_ASSISTANT_VALIDATION_OCCURRENCES,
            MAX_ASSISTANT_VALIDATION_PATH_STEPS,
            MAX_ASSISTANT_VALIDATION_TEXT_BYTES,
        ) {
            Ok(occurrences) => (
                occurrences
                    .into_iter()
                    .filter(|occurrence| occurrence.visible)
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error)),
        }
    } else {
        (Vec::new(), None)
    };
    let requested = ASSISTANT_VALIDATOR_IDS
        .into_iter()
        .filter(|validator| selection.requested.contains(validator))
        .collect::<Vec<_>>();
    let skipped = ASSISTANT_VALIDATOR_IDS
        .into_iter()
        .filter(|validator| !selection.requested.contains(validator))
        .collect::<Vec<_>>();
    if !selection.is_valid() {
        let not_evaluated = selection
            .unknown
            .iter()
            .map(|name| {
                serde_json::json!({
                    "validator": name,
                    "reason": "unknown_validator",
                })
            })
            .collect::<Vec<_>>();
        return serde_json::json!({
            "schema": "ketchup.assistant-validation-context.v1",
            "document_id": snapshot.document_id().0,
            "revision": snapshot.revision_id(),
            "canonical_digest": snapshot.canonical_digest(),
            "selection_mode": selection.mode,
            "validators": assistant_validator_catalog(),
            "requested": requested,
            "executed": [],
            "skipped": ASSISTANT_VALIDATOR_IDS,
            "not_evaluated": not_evaluated,
            "selection_error": "unknown_or_empty_validator_selection",
            "state": "not_evaluated",
            "complete": false,
            "visible_occurrence_count": visible_occurrences.len(),
            "checked_occurrence_count": 0,
            "checked_pair_count": 0,
            "issue_count": 0,
            "issues_complete": true,
            "issues": [],
            "collision": {
                "state": "skipped",
                "complete": false,
                "checked_occurrence_count": 0,
                "checked_pair_count": 0,
                "issue_count": 0,
                "issues_complete": true,
                "issues": [],
            },
            "gravity_support": {
                "state": "skipped",
                "complete": false,
                "gravity_axis": "-Z",
                "floor_z_mm": 0.0,
                "checked_occurrence_count": 0,
                "unsupported_count": 0,
                "issues_complete": true,
                "issues": [],
                "assumptions": [],
            },
            "shelf_deflection": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
            },
            "tipping": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
            },
            "anchoring": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "required_count": 0,
                "issue_count": 0,
                "issues": [],
            },
            "hardware_manufacturing": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
                "not_evaluated": [],
            },
            "room_placement": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
                "not_evaluated": [],
            },
            "passage_clearance": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
                "not_evaluated": [],
            },
            "static_load": {
                "state": "skipped",
                "complete": false,
                "applicable_count": 0,
                "issue_count": 0,
                "issues": [],
                "not_evaluated": [],
            },
            "unavailable_occurrences": [],
        });
    }
    let occurrence_limit_complete = scene_query_error.is_none()
        && visible_occurrences.len() <= MAX_ASSISTANT_VALIDATION_OCCURRENCES;
    let names = visible_occurrences
        .iter()
        .take(MAX_ASSISTANT_VALIDATION_OCCURRENCES)
        .map(|occurrence| {
            (
                occurrence.instance_path.root_occurrence(),
                occurrence.occurrence_name.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut participants = Vec::new();
    let mut unavailable = Vec::new();
    for occurrence in visible_occurrences
        .iter()
        .take(MAX_ASSISTANT_VALIDATION_OCCURRENCES)
    {
        match GeneralBodyParticipant::accept(
            snapshot,
            exact_results,
            occurrence.instance_path.clone(),
            tolerance,
        ) {
            Ok(participant) => participants.push(participant),
            Err(error) => unavailable.push(serde_json::json!({
                "occurrence_id": occurrence.instance_path.root_occurrence().0,
                "name": occurrence.occurrence_name.clone(),
                "reason": format!("{error:?}"),
            })),
        }
    }
    let validator_roles = ValidatorRoleIndex::from_snapshot(snapshot);
    let mut gravity_derivations = Vec::new();
    let mut gravity_participants = validator_roles
        .as_ref()
        .ok()
        .map(|roles| {
            participants
                .iter()
                .filter_map(|body| {
                    let occurrence_id = body.instance_path().root_occurrence();
                    let role = roles
                        .role(occurrence_id)
                        .and_then(|role| assistant_physics_role(role.as_str()))?;
                    matches!(
                        role.kind,
                        AssistantPhysicsRoleKind::GravityBody
                            | AssistantPhysicsRoleKind::GravityGround
                    )
                    .then(|| {
                        GravitySupportParticipant::new(
                            body.clone(),
                            role.group,
                            role.kind == AssistantPhysicsRoleKind::GravityGround
                                || snapshot.occurrence_is_grounded(occurrence_id),
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if gravity_participants.is_empty() {
        gravity_participants = assistant_derived_gravity_participants(snapshot, &participants);
        if !gravity_participants.is_empty() {
            gravity_derivations.push(DERIVED_GRAVITY_ROLE_ASSUMPTION.to_owned());
        }
    }
    let declared_gravity = assistant_gravity_input(snapshot).ok();
    if declared_gravity.is_none() && !gravity_participants.is_empty() {
        gravity_derivations.push(DERIVED_GRAVITY_VECTOR_ASSUMPTION.to_owned());
    }
    let gravity_validator_input = (!gravity_participants.is_empty())
        .then(|| declared_gravity.map_or(CANONICAL_GRAVITY_M_S2, |gravity| gravity.vector_m_s2))
        .and_then(|vector_m_s2| {
            GravitySupportInput::new(gravity_participants.clone(), vector_m_s2).ok()
        });
    // Collision coverage and BRep evidence are independent of the bounded
    // envelope participants used by the remaining structural validators.
    let gravity_report = selection
        .requested
        .contains("gravity_support")
        .then_some(gravity_validator_input.as_ref())
        .flatten()
        .map(|validator_input| {
            let validator = BuiltinGravitySupportValidator::new(tolerance);
            let policy = gravity_support_validation_policy();
            let input = gravity_support_input_bytes(validator_input);
            let invocation = ValidationInvocation::bind(
                snapshot,
                validator.descriptor(),
                &policy,
                Vec::new(),
                &input,
            );
            validator.invoke(ValidationExecution {
                snapshot,
                invocation,
                policy: &policy,
                input: validator_input,
            })
        });
    let coverage_complete = occurrence_limit_complete && unavailable.is_empty();
    let shelf_deflection = assistant_shelf_deflection_report(
        &participants,
        &names,
        &validator_roles,
        selection.requested.contains("shelf_deflection"),
        coverage_complete,
    );
    let tipping = assistant_tipping_report(
        &participants,
        &names,
        &validator_roles,
        selection.requested.contains("tipping"),
        coverage_complete,
    );
    let anchoring = assistant_anchoring_report(
        &participants,
        &names,
        &validator_roles,
        selection.requested.contains("anchoring"),
        coverage_complete,
    );
    let hardware_manufacturing = assistant_hardware_manufacturing_report(
        &participants,
        &names,
        &validator_roles,
        selection.requested.contains("hardware_manufacturing"),
        coverage_complete,
    );
    let room_placement = assistant_room_placement_report(
        &participants,
        &names,
        &validator_roles,
        tolerance,
        selection.requested.contains("room_placement"),
        coverage_complete,
    );
    let passage_clearance = assistant_passage_clearance_report(
        &participants,
        &names,
        &validator_roles,
        tolerance,
        selection.requested.contains("passage_clearance"),
        coverage_complete,
    );
    let static_load = assistant_static_load_report(
        snapshot,
        &names,
        &validator_roles,
        selection.requested.contains("static_load"),
        coverage_complete,
    );
    let collision_state = collision["state"].as_str().unwrap_or("not_evaluated");
    let issue_count = collision["issue_count"].as_u64().unwrap_or(0) as usize;
    let issues = collision["issues"].as_array().cloned().unwrap_or_default();
    let (gravity_state, gravity_issue_count, gravity_issues, gravity_assumptions) = if let Some(
        report,
    ) =
        &gravity_report
    {
        let issue_count = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "gravity.unsupported")
            .count();
        let issues = gravity_participants
            .iter()
            .zip(&report.diagnostics)
            .filter(|(_, diagnostic)| diagnostic.code == "gravity.unsupported")
            .take(MAX_ASSISTANT_VALIDATION_ISSUES)
            .map(|(participant, diagnostic)| {
                let occurrence_id = participant.body.instance_path().root_occurrence();
                serde_json::json!({
                    "code": diagnostic.code.as_str(),
                    "severity": match diagnostic.severity {
                        DiagnosticSeverity::Information => "information",
                        DiagnosticSeverity::Advisory => "advisory",
                        DiagnosticSeverity::Warning => "warning",
                        DiagnosticSeverity::Error => "error",
                    },
                    "evidence_class": match &diagnostic.evidence_class {
                        EvidenceClass::Exact => "exact",
                        EvidenceClass::Tolerant(_) => "tolerant",
                    },
                    "occurrence_id": occurrence_id.0,
                    "name": names.get(&occurrence_id),
                    "evidence": diagnostic.evidence.as_str(),
                })
            })
            .collect::<Vec<_>>();
        let state = if !coverage_complete {
            "not_evaluated"
        } else {
            match report.state {
                ValidationState::Passed => "passed",
                ValidationState::Failed => "failed",
                ValidationState::NotEvaluated => "not_evaluated",
                ValidationState::Unavailable => "unavailable",
            }
        };
        let mut assumptions = gravity_derivations.clone();
        assumptions.extend(report.assumptions.iter().cloned());
        (state, issue_count, issues, assumptions)
    } else if selection.requested.contains("gravity_support") {
        (
                "not_evaluated",
                0,
                Vec::new(),
                vec![
                    "no occurrence is grounded, so nothing can carry load: ground the parts that stand on the ground, or declare physics.gravity.* roles"
                        .to_owned(),
                ],
            )
    } else {
        ("skipped", 0, Vec::new(), Vec::new())
    };
    let shelf_state = shelf_deflection["state"]
        .as_str()
        .unwrap_or("not_evaluated");
    let tipping_state = tipping["state"].as_str().unwrap_or("not_evaluated");
    let anchoring_state = anchoring["state"].as_str().unwrap_or("not_evaluated");
    let hardware_manufacturing_state = hardware_manufacturing["state"]
        .as_str()
        .unwrap_or("not_evaluated");
    let room_placement_state = room_placement["state"].as_str().unwrap_or("not_evaluated");
    let passage_clearance_state = passage_clearance["state"]
        .as_str()
        .unwrap_or("not_evaluated");
    let static_load_state = static_load["state"].as_str().unwrap_or("not_evaluated");
    let shelf_issue_count = shelf_deflection["issue_count"].as_u64().unwrap_or(0) as usize;
    let tipping_issue_count = tipping["issue_count"].as_u64().unwrap_or(0) as usize;
    let anchoring_issue_count = anchoring["issue_count"].as_u64().unwrap_or(0) as usize;
    let hardware_manufacturing_issue_count =
        hardware_manufacturing["issue_count"].as_u64().unwrap_or(0) as usize;
    let room_placement_issue_count = room_placement["issue_count"].as_u64().unwrap_or(0) as usize;
    let passage_clearance_issue_count =
        passage_clearance["issue_count"].as_u64().unwrap_or(0) as usize;
    let static_load_issue_count = static_load["issue_count"].as_u64().unwrap_or(0) as usize;
    let mut not_evaluated = Vec::new();
    for (validator, validator_state) in [
        ("collision", collision_state),
        ("gravity_support", gravity_state),
        ("shelf_deflection", shelf_state),
        ("tipping", tipping_state),
        ("anchoring", anchoring_state),
        ("hardware_manufacturing", hardware_manufacturing_state),
        ("room_placement", room_placement_state),
        ("passage_clearance", passage_clearance_state),
        ("static_load", static_load_state),
    ] {
        if selection.requested.contains(validator)
            && ((validator == "collision" && collision["complete"].as_bool() != Some(true))
                || (validator != "collision" && !coverage_complete)
                || matches!(validator_state, "not_evaluated" | "unavailable"))
        {
            not_evaluated.push(serde_json::json!({
                "validator": validator,
                "reason": if validator == "collision" || coverage_complete {
                    "validator_specific_context_unavailable"
                } else if scene_query_error.is_some() {
                    "validation_context_resource_limit"
                } else {
                    "incomplete_geometry_coverage"
                },
            }));
        }
    }
    let state = if [
        collision_state,
        gravity_state,
        shelf_state,
        tipping_state,
        anchoring_state,
        hardware_manufacturing_state,
        room_placement_state,
        passage_clearance_state,
        static_load_state,
    ]
    .into_iter()
    .any(|state| state == "failed")
    {
        "failed"
    } else if not_evaluated.is_empty() {
        "passed"
    } else {
        "not_evaluated"
    };
    let complete = (selection.requested.iter().all(|id| *id == "collision") || coverage_complete)
        && not_evaluated.is_empty()
        && (!selection.requested.contains("collision")
            || collision["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("gravity_support")
            || gravity_issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES)
        && (!selection.requested.contains("shelf_deflection")
            || shelf_deflection["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("tipping")
            || tipping["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("anchoring")
            || anchoring["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("hardware_manufacturing")
            || hardware_manufacturing["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("room_placement")
            || room_placement["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("passage_clearance")
            || passage_clearance["complete"].as_bool() == Some(true))
        && (!selection.requested.contains("static_load")
            || static_load["complete"].as_bool() == Some(true));
    let total_issue_count = issue_count
        + gravity_issue_count
        + shelf_issue_count
        + tipping_issue_count
        + anchoring_issue_count
        + hardware_manufacturing_issue_count
        + room_placement_issue_count
        + passage_clearance_issue_count
        + static_load_issue_count;
    let mut all_issues = issues.clone();
    all_issues.extend(gravity_issues.iter().cloned());
    for report in [
        &shelf_deflection,
        &tipping,
        &anchoring,
        &hardware_manufacturing,
        &room_placement,
        &passage_clearance,
        &static_load,
    ] {
        all_issues.extend(report["issues"].as_array().into_iter().flatten().cloned());
    }
    let visible_occurrence_count = if selection.requested.contains("collision") {
        collision["visible_occurrence_count"].clone()
    } else if scene_query_error.is_some() {
        serde_json::Value::Null
    } else {
        serde_json::json!(visible_occurrences.len())
    };
    let visible_occurrence_count_at_least = if selection.requested.contains("collision") {
        collision["visible_occurrence_count_at_least"].clone()
    } else {
        scene_query_error
            .as_ref()
            .map(|error| serde_json::json!(error.observed_at_least))
            .unwrap_or(serde_json::Value::Null)
    };
    let validation_context_resource_limit = scene_query_error.as_ref().map(|error| {
        serde_json::json!({
            "resource": format!("{:?}", error.kind),
            "limit": error.limit,
            "observed_at_least": error.observed_at_least,
        })
    });
    // Collision issues are already resource-bounded and must not be silently truncated.
    serde_json::json!({
        "schema": "ketchup.assistant-validation-context.v1",
        "document_id": snapshot.document_id().0,
        "revision": snapshot.revision_id(),
        "canonical_digest": snapshot.canonical_digest(),
        "selection_mode": selection.mode,
        "validators": assistant_validator_catalog(),
        "requested": requested,
        "executed": requested,
        "skipped": skipped,
        "not_evaluated": not_evaluated,
        "selection_error": null,
        "validation_context_resource_limit": validation_context_resource_limit,
        "state": state,
        "complete": complete,
        "visible_occurrence_count": visible_occurrence_count,
        "visible_occurrence_count_at_least": visible_occurrence_count_at_least,
        "checked_occurrence_count": if selection.requested.contains("collision") {
            collision["checked_occurrence_count"].clone()
        } else { serde_json::json!(participants.len()) },
        "checked_pair_count": collision["checked_pair_count"],
        "issue_count": total_issue_count,
        "issues_complete": all_issues.len() == total_issue_count,
        "issues": all_issues,
        "collision": collision,
        "gravity_support": {
            "state": gravity_state,
            "complete": selection.requested.contains("gravity_support")
                && coverage_complete
                && gravity_issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
            "gravity_axis": "-Z",
            "floor_z_mm": 0.0,
            "checked_occurrence_count": if selection.requested.contains("gravity_support") {
                gravity_participants.len()
            } else {
                0
            },
            "unsupported_count": gravity_issue_count,
            "issues_complete": gravity_issue_count <= MAX_ASSISTANT_VALIDATION_ISSUES,
            "issues": gravity_issues,
            "assumptions": gravity_assumptions,
        },
        "shelf_deflection": shelf_deflection,
        "tipping": tipping,
        "anchoring": anchoring,
        "hardware_manufacturing": hardware_manufacturing,
        "room_placement": room_placement,
        "passage_clearance": passage_clearance,
        "static_load": static_load,
        "unavailable_occurrences": if selection.requested.iter().all(|id| *id == "collision") {
            collision["unavailable_occurrences"].clone()
        } else { serde_json::json!(unavailable) },
    })
}
