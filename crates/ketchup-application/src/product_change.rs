//! Plans the CAD edit operations on product entities that are not geometry:
//! spaces, clearance volumes, persistent dimensions and evaluator input
//! values. Spaces refer to each other (mutual
//! adjacency), so they are staged without a preview and the document store
//! checks them together with what follows; a refused change never reaches
//! the document.

use crate::diagnostics::{
    AssistantPlanningResult, AssistantRejection, assistant_canonical_rejection,
    assistant_planning_rejection,
};
use crate::model_query::canonical_instance_path;
use crate::planner::StagedPlanningContext;
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantClearanceOwner, AssistantClearanceSeverity,
    AssistantDimensionUnit,
};
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, Dimension, DimensionDisplayUnit, DimensionPresentation,
    FeatureId, FeatureParameterTarget, NodeId, ParameterValueType, PersistentDimension,
    PersistentDimensionId, PersistentDimensionTarget,
};
use ketchup_model::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceOwner, ClearanceSeverity, ClearanceVolumeId,
    SpaceError, SpaceId,
};
use ketchup_model::tolerance::TolerancePolicy;

/// Stages the command of a product entity operation, or `None` for any other
/// operation.
pub(crate) fn plan_product_change(
    operation: &AssistantCadEditOperation,
    staged: &mut StagedPlanningContext,
) -> Option<AssistantPlanningResult<()>> {
    let command = match operation {
        AssistantCadEditOperation::UpsertSpace {
            space_id,
            purpose,
            min_mm,
            max_mm,
            adjacent_to,
            accessible_to,
        } => {
            let ids = |ids: &[u64]| ids.iter().copied().map(SpaceId).collect();
            box_volume(*min_mm, *max_mm)
                .and_then(|volume| {
                    CanonicalSpace::new(
                        SpaceId(*space_id),
                        purpose.clone(),
                        volume,
                        ids(adjacent_to),
                        ids(accessible_to),
                    )
                })
                .map(CanonicalCommand::UpsertSpace)
                .map_err(|error| rejected(error, "upsert_space", "space", *space_id))
        }
        AssistantCadEditOperation::DeleteSpace { space_id } => Ok(CanonicalCommand::DeleteSpace {
            id: SpaceId(*space_id),
        }),
        AssistantCadEditOperation::UpsertClearanceVolume {
            clearance_volume_id,
            reason,
            owner,
            min_mm,
            max_mm,
            severity,
            tolerance_mm,
        } => plan_clearance(
            staged,
            *clearance_volume_id,
            reason,
            owner,
            (*min_mm, *max_mm),
            *severity,
            *tolerance_mm,
        ),
        AssistantCadEditOperation::DeleteClearanceVolume {
            clearance_volume_id,
        } => Ok(CanonicalCommand::DeleteClearanceVolume {
            id: ClearanceVolumeId(*clearance_volume_id),
        }),
        AssistantCadEditOperation::UpsertDimension {
            dimension_id,
            name,
            feature_id,
            parameter_path,
            unit,
            decimal_places,
        } => plan_dimension(
            staged,
            *dimension_id,
            name,
            FeatureId(*feature_id),
            parameter_path,
            (*unit, *decimal_places),
        ),
        AssistantCadEditOperation::DeleteDimension { dimension_id } => {
            Ok(CanonicalCommand::DeletePersistentDimension {
                id: PersistentDimensionId(*dimension_id),
            })
        }
        AssistantCadEditOperation::CreateEvaluatorInput {
            node_id,
            name,
            value,
        } => Dimension::new(value.to_string(), *value)
            .map(|dimension| CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(*node_id),
                name: name.clone(),
                dimension,
                dependencies: Vec::new(),
            })
            .map_err(|error| {
                assistant_canonical_rejection(
                    error.into(),
                    "create_evaluator_input",
                    &format!("node:{node_id}"),
                )
            }),
        AssistantCadEditOperation::SetEvaluatorInput { node_id, value } => {
            plan_evaluator_input(staged, NodeId(*node_id), *value)
        }
        _ => return None,
    };
    Some(command.map(|command| staged.push(command)))
}

fn plan_clearance(
    staged: &mut StagedPlanningContext,
    id: u64,
    reason: &str,
    owner: &AssistantClearanceOwner,
    (min_mm, max_mm): ([f64; 3], [f64; 3]),
    severity: AssistantClearanceSeverity,
    tolerance_mm: Option<f64>,
) -> AssistantPlanningResult<CanonicalCommand> {
    const OPERATION: &str = "upsert_clearance_volume";
    let target = format!("clearance_volume:{id}");
    let owner = match owner {
        AssistantClearanceOwner::Space { space_id } => ClearanceOwner::Space(SpaceId(*space_id)),
        AssistantClearanceOwner::Occurrence { instance_path } => {
            // The owner may come from earlier operations of this program.
            let document = format!("document:{}", staged.base_snapshot().document_id().0);
            staged.refresh(OPERATION, &document)?;
            let path = canonical_instance_path(staged.staged_snapshot(), instance_path).map_err(
                |error| {
                    assistant_planning_rejection(
                        "planning.clearance_owner_not_found",
                        OPERATION,
                        &target,
                        format!("the owner instance path does not resolve ({error:?})"),
                        "Name an existing occurrence path, e.g. from inspect query instances.",
                    )
                },
            )?;
            ClearanceOwner::Occurrence(path)
        }
    };
    let tolerance = tolerance_mm
        .map_or_else(|| Ok(TolerancePolicy::default()), TolerancePolicy::new)
        .map_err(|error| assistant_canonical_rejection(error.into(), OPERATION, &target))?;
    let severity = match severity {
        AssistantClearanceSeverity::Advisory => ClearanceSeverity::Advisory,
        AssistantClearanceSeverity::Required => ClearanceSeverity::Required,
    };
    box_volume(min_mm, max_mm)
        .and_then(|volume| {
            CanonicalClearanceVolume::new(
                ClearanceVolumeId(id),
                owner,
                reason,
                volume,
                tolerance,
                severity,
                None,
            )
        })
        .map(CanonicalCommand::UpsertClearanceVolume)
        .map_err(|error| rejected(error, OPERATION, "clearance_volume", id))
}

fn plan_dimension(
    staged: &mut StagedPlanningContext,
    id: u64,
    name: &str,
    feature_id: FeatureId,
    parameter_path: &str,
    (unit, decimal_places): (Option<AssistantDimensionUnit>, Option<u8>),
) -> AssistantPlanningResult<CanonicalCommand> {
    const OPERATION: &str = "upsert_dimension";
    let target = format!("dimension:{id}");
    let not_found = |reason: String| {
        assistant_planning_rejection(
            "planning.dimension_target_not_found",
            OPERATION,
            &target,
            reason,
            "Name a feature and one of its length parameter paths, e.g. extent.distance of an extrusion, from inspect query features.",
        )
    };
    let parameter =
        FeatureParameterTarget::new(feature_id, parameter_path, ParameterValueType::Length)
            .map_err(|error| not_found(error.to_string()))?;
    // The feature may come from earlier operations of this program.
    let document = format!("document:{}", staged.base_snapshot().document_id().0);
    staged.refresh(OPERATION, &document)?;
    if staged
        .staged_snapshot()
        .feature_parameter_value(&parameter)
        .is_none()
    {
        return Err(not_found(format!(
            "feature {} has no length parameter {parameter_path}",
            feature_id.0
        )));
    }
    let unit = match unit.unwrap_or(AssistantDimensionUnit::Mm) {
        AssistantDimensionUnit::Mm => DimensionDisplayUnit::Millimetres,
        AssistantDimensionUnit::Cm => DimensionDisplayUnit::Centimetres,
        AssistantDimensionUnit::In => DimensionDisplayUnit::Inches,
    };
    DimensionPresentation::new(unit, decimal_places.unwrap_or(1))
        .and_then(|presentation| {
            PersistentDimension::new(
                PersistentDimensionId(id),
                name,
                PersistentDimensionTarget::FeatureParameter(parameter),
                presentation,
            )
        })
        .map(CanonicalCommand::UpsertPersistentDimension)
        .map_err(|error| assistant_canonical_rejection(error, OPERATION, &target))
}

/// Only an input (a parameter node) takes a value; an expression or a rule
/// would be replaced by one.
fn plan_evaluator_input(
    staged: &mut StagedPlanningContext,
    id: NodeId,
    value: f64,
) -> AssistantPlanningResult<CanonicalCommand> {
    const OPERATION: &str = "set_evaluator_input";
    let target = format!("node:{}", id.0);
    let document = format!("document:{}", staged.base_snapshot().document_id().0);
    staged.refresh(OPERATION, &document)?;
    let node = staged.staged_snapshot().evaluator_node(id);
    if node.and_then(|node| node.dimension()).is_none() {
        return Err(assistant_planning_rejection(
            "planning.evaluator_input_not_found",
            OPERATION,
            &target,
            format!("node {} is not an evaluator input", id.0),
            "Name a node created with create_evaluator_input.",
        ));
    }
    Dimension::new(value.to_string(), value)
        .map(|dimension| CanonicalCommand::SetEvaluatorDimension { id, dimension })
        .map_err(|error| assistant_canonical_rejection(error.into(), OPERATION, &target))
}

fn box_volume(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Aabb, SpaceError> {
    Aabb::new(min_mm, max_mm).map_err(SpaceError::Prismatic)
}

fn rejected(error: SpaceError, operation: &str, kind: &str, id: u64) -> AssistantRejection {
    assistant_canonical_rejection(
        CanonicalError::Space(error),
        operation,
        &format!("{kind}:{id}"),
    )
}
