//! Plans the CAD edit operations on product entities that are not geometry:
//! spaces. The document store checks what spans several entities (mutual
//! adjacency, references to other spaces) when the batch is previewed, so a
//! refused change never reaches the document.

use crate::diagnostics::{
    AssistantPlanningResult, AssistantRejection, assistant_canonical_rejection,
};
use ketchup_assistant::sidecar::AssistantCadEditOperation;
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::document::{CanonicalCommand, CanonicalError};
use ketchup_model::space::{CanonicalSpace, SpaceError, SpaceId};

/// The command of a product entity operation, or `None` for any other operation.
pub(crate) fn plan_product_change(
    operation: &AssistantCadEditOperation,
) -> Option<AssistantPlanningResult<CanonicalCommand>> {
    Some(match operation {
        AssistantCadEditOperation::UpsertSpace {
            space_id,
            purpose,
            min_mm,
            max_mm,
            adjacent_to,
            accessible_to,
        } => {
            let ids = |ids: &[u64]| ids.iter().copied().map(SpaceId).collect();
            Aabb::new(*min_mm, *max_mm)
                .map_err(SpaceError::Prismatic)
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
                .map_err(|error| rejected(error, "upsert_space", *space_id))
        }
        AssistantCadEditOperation::DeleteSpace { space_id } => Ok(CanonicalCommand::DeleteSpace {
            id: SpaceId(*space_id),
        }),
        _ => return None,
    })
}

fn rejected(error: SpaceError, operation: &str, space_id: u64) -> AssistantRejection {
    assistant_canonical_rejection(
        CanonicalError::Space(error),
        operation,
        &format!("space:{space_id}"),
    )
}
