//! Plans the CAD edit operations on product entities that are not geometry:
//! spaces and clearance volumes. Spaces refer to each other (mutual
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
};
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::document::{CanonicalCommand, CanonicalError};
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
